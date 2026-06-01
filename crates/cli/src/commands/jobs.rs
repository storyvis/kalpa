//! Jobs command — manage and check status of async generation jobs.

use clap::Args;
use colored::Colorize;
use kalpa_core::jobs::{Job, JobStore, JobStatus};
use kalpa_core::KalpaConfig;

/// Arguments for the `jobs` subcommand.
#[derive(Debug, Args)]
#[command(
    about = "Manage async generation jobs",
    long_about = "View status and manage asynchronous image/video generation jobs.\n\n\
                  Examples:\n  \
                  kalpa jobs                    # List all jobs\n  \
                  kalpa jobs <job-id>           # Check specific job status\n  \
                  kalpa jobs --clear-completed  # Remove completed jobs"
)]
pub struct JobsArgs {
    /// Job ID to check status for (optional)
    pub job_id: Option<String>,

    /// Clear all completed jobs
    #[arg(long)]
    pub clear_completed: bool,

    /// Clear all failed jobs
    #[arg(long)]
    pub clear_failed: bool,

    /// Delete a specific job
    #[arg(long)]
    pub delete: Option<String>,
}

/// Execute the jobs command.
pub async fn execute(args: JobsArgs, json: bool) -> anyhow::Result<()> {
    let store = JobStore::new()?;

    // Handle delete operation
    if let Some(job_id) = &args.delete {
        store.delete(job_id)?;
        if !json {
            println!("{} Deleted job: {}", "✓".green(), job_id);
        }
        return Ok(());
    }

    // Handle clear operations
    if args.clear_completed || args.clear_failed {
        let jobs = store.list()?;
        let mut deleted_count = 0;

        for job in jobs {
            let should_delete = (args.clear_completed && job.status == JobStatus::Completed)
                || (args.clear_failed && job.status == JobStatus::Failed);

            if should_delete {
                store.delete(&job.id)?;
                deleted_count += 1;
            }
        }

        if !json {
            let job_type = if args.clear_completed {
                "completed"
            } else {
                "failed"
            };
            println!(
                "{} Cleared {} {} job(s)",
                "✓".green(),
                deleted_count,
                job_type
            );
        }
        return Ok(());
    }

    // Check specific job
    if let Some(job_id) = &args.job_id {
        let mut job = store.load(job_id)?;

        // For running jobs, do a live status check
        if job.status == JobStatus::Running {
            if let Some(op_id) = job.operation_id.clone() {
                let config = KalpaConfig::load()?;
                match job.provider.as_str() {
                    "fal" => {
                        if let Some(api_key) = config.get_api_key(kalpa_core::Provider::Fal) {
                            live_check_fal_job(&mut job, api_key, &op_id, &store, json).await?;
                        }
                    }
                    "vertex" => {
                        live_check_vertex_job(&mut job, &config, &op_id, &store, json).await?;
                    }
                    _ => {}
                }
            }
        }

        if json {
            println!("{}", serde_json::to_string_pretty(&job)?);
        } else {
            println!();
            println!("{}", "Job Details".bold().underline());
            println!();
            println!("  {:<15} {}", "ID:".bold(), job.id);
            println!("  {:<15} {:?}", "Type:".bold(), job.job_type);
            println!("  {:<15} {}", "Status:".bold(), format_status(&job.status));
            println!("  {:<15} {}", "Provider:".bold(), job.provider);
            println!("  {:<15} {}", "Model:".bold(), job.model);
            println!("  {:<15} {}", "Prompt:".bold(), truncate(&job.prompt, 60));

            if let Some(op_id) = &job.operation_id {
                println!("  {:<15} {}", "Operation ID:".bold(), op_id);
            }

            if let Some(result) = &job.result_path {
                println!("  {:<15} {}", "Result:".bold(), result.green());
            }

            if let Some(error) = &job.error {
                println!("  {:<15} {}", "Error:".bold(), error.red());
            }

            let created = format_timestamp(job.created_at);
            let updated = format_timestamp(job.updated_at);
            println!("  {:<15} {}", "Created:".bold(), created.dimmed());
            println!("  {:<15} {}", "Updated:".bold(), updated.dimmed());
            println!();
        }

        return Ok(());
    }

    // List all jobs
    let jobs = store.list()?;

    if jobs.is_empty() {
        if !json {
            println!("{}", "No jobs found".dimmed());
        }
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&jobs)?);
    } else {
        println!();
        println!("{}", "Recent Jobs".bold().underline());
        println!();

        for job in jobs.iter().take(20) {
            let status_icon = match job.status {
                JobStatus::Completed => "✓".green(),
                JobStatus::Failed => "✗".red(),
                JobStatus::Running => "◷".yellow(),
                JobStatus::Pending => "○".blue(),
            };

            let job_type_str = match job.job_type {
                kalpa_core::jobs::JobType::Image => "img",
                kalpa_core::jobs::JobType::Video => "vid",
            };

            println!(
                "  {} {:<18} {:<6} {} {}",
                status_icon,
                job.id,
                job_type_str,
                format_status(&job.status),
                truncate(&job.prompt, 40).dimmed()
            );
        }

        println!();
        println!(
            "  {} Use {} to check job details",
            "ℹ".blue(),
            "kalpa jobs <job-id>".cyan()
        );
        println!();
    }

    Ok(())
}

/// Do a live status check for a running Vertex AI job and update the local job store.
/// Uses the :fetchPredictOperation endpoint with the operation name.
async fn live_check_vertex_job(
    job: &mut Job,
    config: &KalpaConfig,
    operation_name: &str,
    store: &JobStore,
    json: bool,
) -> anyhow::Result<()> {
    use kalpa_core::auth::VertexAuthToken;
    use kalpa_core::Provider;
    use std::path::Path;

    // Get service account path for OAuth
    let service_account_path = config
        .get_service_account_path(Provider::Vertex)
        .ok_or_else(|| anyhow::anyhow!("No service account configured for Vertex AI"))?;

    // Get OAuth token
    let auth_token = VertexAuthToken::from_service_account_file(Path::new(service_account_path))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to authenticate with Vertex AI: {}", e))?;

    // Parse operation_name to extract location and model
    // Format: "projects/{project}/locations/{location}/publishers/google/models/{model}/operations/{uuid}"
    let location = operation_name
        .split("/locations/")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or("us-central1");

    let model_name = operation_name
        .split("/models/")
        .nth(1)
        .and_then(|s| s.split("/operations/").next())
        .ok_or_else(|| anyhow::anyhow!("Invalid operation name format: {}", operation_name))?;

    let project_id = &auth_token.project_id;

    // Call :fetchPredictOperation endpoint
    let url = format!(
        "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:fetchPredictOperation",
        location, project_id, location, model_name
    );

    let body = serde_json::json!({
        "operationName": operation_name
    });

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", auth_token.access_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await;

    match response {
        Ok(resp) => {
            if !resp.status().is_success() {
                let status_code = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                if !json {
                    println!("  {} Could not check live status (HTTP {}): {}", "⚠".yellow(), status_code, &error_text[..error_text.len().min(100)]);
                }
                return Ok(());
            }

            let response_json: serde_json::Value = resp.json().await
                .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))?;

            let done = response_json.get("done").and_then(|d| d.as_bool()).unwrap_or(false);

            if done {
                // Check for errors
                if let Some(error) = response_json.get("error") {
                    let error_msg = error.get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown error");
                    job.fail(error_msg.to_string());
                    store.save(job)?;

                    if !json {
                        println!("  {} Live status: {} — {}", "↻".cyan(), "FAILED".red().bold(), error_msg);
                    }
                } else {
                    // Try to extract video GCS URI from response
                    let gcs_uri = response_json.get("response")
                        .and_then(|r| r.get("predictions"))
                        .and_then(|p| p.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|pred| pred.get("gcsUri").or_else(|| pred.get("videoUri")))
                        .and_then(|u| u.as_str())
                        .map(|s| s.to_string());

                    if let Some(uri) = gcs_uri {
                        job.complete(uri.clone());
                        if !json {
                            println!("  {} Live status: {}", "↻".cyan(), "COMPLETED".green().bold());
                            println!("  {} Video: {}", "→".dimmed(), uri.cyan());
                        }
                    } else {
                        // Done but can't extract URI — might be in GCS bucket already
                        job.update_status(JobStatus::Completed);
                        if !json {
                            println!("  {} Live status: {} (check GCS bucket for output)", "↻".cyan(), "COMPLETED".green().bold());
                        }
                    }
                    store.save(job)?;
                }
            } else {
                // Still in progress
                if !json {
                    println!("  {} Live status: {}", "↻".cyan(), "IN_PROGRESS".yellow());
                }
            }
        }
        Err(e) => {
            if !json {
                println!("  {} Could not check live status: {}", "⚠".yellow(), e);
            }
        }
    }

    Ok(())
}

fn format_status(status: &JobStatus) -> colored::ColoredString {
    match status {
        JobStatus::Pending => "pending".blue(),
        JobStatus::Running => "running".yellow(),
        JobStatus::Completed => "completed".green(),
        JobStatus::Failed => "failed".red(),
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

fn format_timestamp(ts: u64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let diff = now.saturating_sub(ts);

    if diff < 60 {
        format!("{}s ago", diff)
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// Do a live status check for a running fal.ai job and update the local job store.
async fn live_check_fal_job(
    job: &mut Job,
    api_key: &str,
    request_id: &str,
    store: &JobStore,
    json: bool,
) -> anyhow::Result<()> {
    use kalpa_core::providers::FalAIProvider;
    use kalpa_core::types::FalQueueStatus;

    let provider = FalAIProvider::new(api_key.to_string());

    // Construct the canonical status URL from the model stored in the job
    // Fal.ai uses a shortened model path in the status URL, but we don't know
    // exactly what it is. We'll construct one from the request_id and use 
    // queue_status_by_url with the pattern fal.ai returns.
    // The status URL pattern from fal.ai is:
    // https://queue.fal.run/{shortened_model}/requests/{request_id}/status
    // Since we don't store the status_url, we construct it from model path.
    // However, the model in the job might be the full path. Let's try the 
    // direct approach: construct the URL with the model from the job and if that
    // fails, try extracting just the base model.
    
    // First, try the shortened model path (first two segments e.g. "fal-ai/kling-video")
    let model_parts: Vec<&str> = job.model.split('/').collect();
    let shortened_model = if model_parts.len() > 2 {
        format!("{}/{}", model_parts[0], model_parts[1])
    } else {
        job.model.clone()
    };

    let status_url = format!(
        "https://queue.fal.run/{}/requests/{}/status",
        shortened_model, request_id
    );

    match provider.queue_status_by_url(&status_url).await {
        Ok(status) => {
            match status {
                FalQueueStatus::Completed { .. } => {
                    // Try to fetch the result
                    let response_url = format!(
                        "https://queue.fal.run/{}/requests/{}",
                        shortened_model, request_id
                    );

                    match provider.queue_result_by_url(&response_url).await {
                        Ok(result_json) => {
                            // Try to extract video URL
                            let video_url = result_json.get("video")
                                .and_then(|v| v.get("url"))
                                .and_then(|u| u.as_str())
                                .or_else(|| {
                                    result_json.get("videos")
                                        .and_then(|v| v.as_array())
                                        .and_then(|arr| arr.first())
                                        .and_then(|v| v.get("url"))
                                        .and_then(|u| u.as_str())
                                });

                            if let Some(url) = video_url {
                                job.complete(url.to_string());
                            } else {
                                // Completed but no video URL found — mark completed with raw info
                                job.complete(format!("Result available (keys: {:?})",
                                    result_json.as_object().map(|o| o.keys().collect::<Vec<_>>())
                                ));
                            }
                        }
                        Err(_) => {
                            // Can't fetch result but status is completed
                            job.update_status(JobStatus::Completed);
                        }
                    }
                    store.save(job)?;

                    if !json {
                        println!("  {} Live status: {}", "↻".cyan(), "COMPLETED".green().bold());
                    }
                }
                FalQueueStatus::Failed { error, .. } => {
                    job.fail(error.clone());
                    store.save(job)?;

                    if !json {
                        println!("  {} Live status: {} — {}", "↻".cyan(), "FAILED".red().bold(), error);
                    }
                }
                FalQueueStatus::InProgress { .. } => {
                    if !json {
                        println!("  {} Live status: {}", "↻".cyan(), "IN_PROGRESS".yellow());
                    }
                }
                FalQueueStatus::InQueue { queue_position, .. } => {
                    let pos = queue_position
                        .map(|p| format!(" (position {})", p))
                        .unwrap_or_default();
                    if !json {
                        println!("  {} Live status: {}{}", "↻".cyan(), "IN_QUEUE".blue(), pos);
                    }
                }
            }
        }
        Err(e) => {
            if !json {
                println!("  {} Could not check live status: {}", "⚠".yellow(), e);
            }
        }
    }

    Ok(())
}
