//! Job tracking system for async generation tasks.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::error::{KalpaError, KalpaResult};

/// Job status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// Job is queued or starting
    Pending,
    /// Job is actively running
    Running,
    /// Job completed successfully
    Completed,
    /// Job failed with an error
    Failed,
}

/// Job type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobType {
    Image,
    Video,
}

/// Job metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Unique job ID
    pub id: String,
    /// Job type (image or video)
    pub job_type: JobType,
    /// Current status
    pub status: JobStatus,
    /// Provider name
    pub provider: String,
    /// Model name
    pub model: String,
    /// Prompt used for generation
    pub prompt: String,
    /// Operation ID from the provider (if applicable)
    pub operation_id: Option<String>,
    /// Result file path (when completed)
    pub result_path: Option<String>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// Creation timestamp
    pub created_at: u64,
    /// Last updated timestamp
    pub updated_at: u64,
}

impl Job {
    /// Create a new job
    pub fn new(
        job_type: JobType,
        provider: String,
        model: String,
        prompt: String,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let id = format!(
            "{}_{}_{}", 
            match job_type {
                JobType::Image => "img",
                JobType::Video => "vid",
            },
            &provider[..3.min(provider.len())],
            now
        );

        Self {
            id,
            job_type,
            status: JobStatus::Pending,
            provider,
            model,
            prompt,
            operation_id: None,
            result_path: None,
            error: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Update job status
    pub fn update_status(&mut self, status: JobStatus) {
        self.status = status;
        self.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    /// Mark job as completed with result path
    pub fn complete(&mut self, result_path: String) {
        self.status = JobStatus::Completed;
        self.result_path = Some(result_path);
        self.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    /// Mark job as failed with error
    pub fn fail(&mut self, error: String) {
        self.status = JobStatus::Failed;
        self.error = Some(error);
        self.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
}

/// Job store for persisting job metadata
pub struct JobStore {
    jobs_dir: PathBuf,
}

impl JobStore {
    /// Create a new job store
    pub fn new() -> KalpaResult<Self> {
        let jobs_dir = dirs::data_local_dir()
            .ok_or_else(|| KalpaError::Config("Cannot determine data directory".into()))?
            .join("kalpa")
            .join("jobs");

        std::fs::create_dir_all(&jobs_dir).map_err(|e| {
            KalpaError::Config(format!("Failed to create jobs directory: {}", e))
        })?;

        Ok(Self { jobs_dir })
    }

    /// Save a job
    pub fn save(&self, job: &Job) -> KalpaResult<()> {
        let job_file = self.jobs_dir.join(format!("{}.json", job.id));
        let json = serde_json::to_string_pretty(job).map_err(|e| {
            KalpaError::Config(format!("Failed to serialize job: {}", e))
        })?;

        std::fs::write(&job_file, json).map_err(|e| {
            KalpaError::Config(format!("Failed to write job file: {}", e))
        })?;

        Ok(())
    }

    /// Load a job by ID
    pub fn load(&self, job_id: &str) -> KalpaResult<Job> {
        let job_file = self.jobs_dir.join(format!("{}.json", job_id));
        
        if !job_file.exists() {
            return Err(KalpaError::Config(format!("Job not found: {}", job_id)));
        }

        let json = std::fs::read_to_string(&job_file).map_err(|e| {
            KalpaError::Config(format!("Failed to read job file: {}", e))
        })?;

        let job: Job = serde_json::from_str(&json).map_err(|e| {
            KalpaError::Config(format!("Failed to parse job file: {}", e))
        })?;

        Ok(job)
    }

    /// List all jobs
    pub fn list(&self) -> KalpaResult<Vec<Job>> {
        let mut jobs = Vec::new();

        for entry in std::fs::read_dir(&self.jobs_dir).map_err(|e| {
            KalpaError::Config(format!("Failed to read jobs directory: {}", e))
        })? {
            let entry = entry.map_err(|e| {
                KalpaError::Config(format!("Failed to read directory entry: {}", e))
            })?;

            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(json) = std::fs::read_to_string(&path) {
                    if let Ok(job) = serde_json::from_str::<Job>(&json) {
                        jobs.push(job);
                    }
                }
            }
        }

        // Sort by creation time (newest first)
        jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(jobs)
    }

    /// Delete a job
    pub fn delete(&self, job_id: &str) -> KalpaResult<()> {
        let job_file = self.jobs_dir.join(format!("{}.json", job_id));
        
        if job_file.exists() {
            std::fs::remove_file(&job_file).map_err(|e| {
                KalpaError::Config(format!("Failed to delete job file: {}", e))
            })?;
        }

        Ok(())
    }
}
