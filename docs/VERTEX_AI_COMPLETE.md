s# Vertex AI Complete Implementation Guide

This document describes the complete implementation of Vertex AI support for text, image, and video generation with job tracking.

## Overview

Kalpa now supports the following Vertex AI capabilities:

1. **Text Generation** - Using Gemini models (gemini-2.5-flash, gemini-2.0-flash, etc.)
2. **Image Generation** - Using Imagen models (imagen-4.0-generate-001, imagen-3.0-generate-001, etc.)
3. **Video Generation** - Using Veo models (veo-3.0-generate, veo-2.0-generate-001, etc.)
4. **Job Tracking** - Async job system for long-running operations

## Setup

Before using Vertex AI, you need to configure your service account:

```bash
# Configure Vertex AI service account
kalpa configure --set vertex.service_account_json /path/to/service-account.json
kalpa configure --set vertex.location us-central1

# Verify authentication
kalpa auth -v
```

## Text Generation

Generate text using Vertex AI's Gemini models:

```bash
# Basic text generation
kalpa generate -v text "Explain quantum computing in simple terms"

# Using a specific model
kalpa generate -v text --model gemini-2.0-flash "Write a haiku about AI"

# JSON output
kalpa generate -v text "What is machine learning?" --json
```

### Supported Text Models

- `gemini-3.1-flash` - Latest Nano Banana 2
- `gemini-3-pro` - Nano Banana Pro
- `gemini-2.5-flash` - Original Nano Banana (default)
- `gemini-2.0-flash`
- `gemini-1.5-pro`
- `gemini-1.5-flash`

## Image Generation

Generate images using Vertex AI's Imagen models:

```bash
# Generate an image
kalpa generate -v image "A futuristic city with flying cars at sunset"

# Using a specific Imagen model
kalpa generate -v image --model imagen-3.0-generate-001 "A cat wearing a space helmet"

# The image will be saved locally as vertex_image_<timestamp>.png
```

### Supported Image Models

- `imagen-4.0-generate-001` - Latest Imagen 4.0 (default)
- `imagen-3.0-generate-001` - Imagen 3.0
- `imagen-3.0-generate-002` - Imagen 3.0 v2
- `imagen-3.0-fast-generate-001` - Fast Imagen 3.0

### Image Output

Images are automatically saved as PNG files with the format `vertex_image_<timestamp>.png` in the current directory.

## Video Generation

Generate videos using Vertex AI's Veo models. Video generation is a long-running operation that uses the job tracking system:

```bash
# Generate a video (creates a job and polls for completion)
kalpa generate -v video "A serene beach at sunrise with gentle waves"

# Using a specific Veo model
kalpa generate -v video --model veo-2.0-generate-001 "A time-lapse of a flower blooming"

# The video will be saved locally as vertex_video_<timestamp>.mp4
```

### Supported Video Models

- `veo-3.0-generate` - Latest Veo 3.0
- `veo-3.0-fast-generate-preview` - Fast Veo 3.0 (preview)
- `veo-2.0-generate-001` - Veo 2.0

### Video Generation Process

1. **Job Creation** - A job is created and tracked in the local job store
2. **API Request** - Video generation request is sent to Vertex AI
3. **Polling** - The system polls the operation status every 5 seconds
4. **Completion** - Video is decoded and saved locally when ready
5. **Job Update** - Job status is updated to "completed" or "failed"

Video files are saved as MP4 files with the format `vertex_video_<timestamp>.mp4`.

## Job Tracking System

The job tracking system allows you to monitor long-running video generation operations.

### List All Jobs

```bash
# List all recent jobs
kalpa jobs

# Output example:
#   ✓ vid_ver_1737412345  vid    completed  A serene beach...
#   ◷ vid_ver_1737412346  vid    running    A futuristic city...
#   ○ img_ver_1737412347  img    pending    A cat on mars...
```

### Check Specific Job Status

```bash
# Check a specific job by ID
kalpa jobs vid_ver_1737412345

# Output example:
# Job Details
# 
#   ID:              vid_ver_1737412345
#   Type:            Video
#   Status:          completed
#   Provider:        vertex
#   Model:           veo-2.0-generate-001
#   Prompt:          A serene beach at sunrise with gentle waves
#   Result:          vertex_video_1737412345.mp4
#   Created:         2m ago
#   Updated:         1m ago
```

### Job Management

```bash
# Clear completed jobs
kalpa jobs --clear-completed

# Clear failed jobs
kalpa jobs --clear-failed

# Delete a specific job
kalpa jobs --delete vid_ver_1737412345

# JSON output for all jobs
kalpa jobs --json
```

### Job Status Values

- **pending** - Job is queued but not yet started
- **running** - Job is actively being processed
- **completed** - Job finished successfully (result available)
- **failed** - Job encountered an error

## Implementation Details

### Architecture

The Vertex AI implementation consists of:

1. **Provider Layer** (`crates/core/src/providers/vertex.rs`)
   - Implements `CompletionProvider` for text generation
   - Implements `ImageGenerationProvider` for image generation
   - Implements `VideoGenerationProvider` for video generation
   - Handles OAuth authentication and API communication

2. **Job System** (`crates/core/src/jobs.rs`)
   - `Job` struct for tracking job metadata
   - `JobStore` for persisting jobs to local disk
   - Job status transitions (pending → running → completed/failed)

3. **CLI Commands** (`crates/cli/src/commands/`)
   - `generate.rs` - Unified generation interface
   - `jobs.rs` - Job management and status checking

### Authentication Flow

1. Service account JSON is loaded from configured path
2. JWT token is created and signed using service account credentials
3. OAuth2 access token is obtained from Google's token endpoint
4. Access token is used for all Vertex AI API requests

### Video Generation Flow

1. User runs `kalpa generate -v video "prompt"`
2. System creates a Job entry with status "running"
3. API request is sent to Vertex AI's predict endpoint
4. Vertex AI returns an operation ID for the long-running task
5. System polls the operation endpoint every 5 seconds
6. When operation is complete, video data is retrieved
7. Video is base64-decoded and saved as MP4 file
8. Job status is updated to "completed" with result path

### Error Handling

- **Authentication errors** - Clear message with setup instructions
- **API errors** - HTTP status and error message displayed
- **Timeout errors** - Video generation timeout after 10 minutes
- **Job failures** - Errors are captured and stored in job record

## API Usage Examples

### Text Generation Output

```bash
$ kalpa generate -v text "What is AI?"

  → Generating text with Vertex AI (gemini-2.5-flash)...

─── Vertex AI (gemini-2.5-flash) 

Artificial Intelligence (AI) refers to the simulation of human 
intelligence in machines programmed to think and learn like humans.

  Usage: 156 tokens
───────────────────────────────
```

### Image Generation Output

```bash
$ kalpa generate -v image "A mountain landscape"

  → Generating image with Vertex AI (imagen-4.0-generate-001)...

✓ Image generated!
  File:  vertex_image_1737412345.png
  Size:  245678 bytes
```

### Video Generation Output

```bash
$ kalpa generate -v video "A sunset over the ocean"

  → Starting video generation with Vertex AI (veo-2.0-generate-001)...
  ℹ Video generation can take several minutes
  ✓ Job created: vid_ver_1737412345
Starting video generation with veo-2.0-generate-001 model...
Note: Video generation can take several minutes
Video generation started. Operation ID: 1234567890
Polling for completion...
Still generating... (attempt 1/120)
Still generating... (attempt 2/120)
...
✓ Video generation completed!

✓ Video generated!
  File:  vertex_video_1737412345.mp4
  Size:  4567890 bytes
  Job:   vid_ver_1737412345
```

## Configuration Reference

```toml
# ~/.config/kalpa/config.toml

[vertex]
service_account_json = "/path/to/service-account.json"
location = "us-central1"
default_model = "gemini-2.5-flash"  # For text generation
```

## Troubleshooting

### Authentication Issues

```bash
# Verify service account file exists
ls -l ~/.config/kalpa/service-account.json

# Test authentication
kalpa auth -v

# Check configuration
kalpa configure --show
```

### Video Generation Timeout

If video generation times out after 10 minutes:
- Check the job status: `kalpa jobs <job-id>`
- The operation may still be processing on Vertex AI's side
- Try with a shorter/simpler prompt
- Consider using `veo-3.0-fast-generate-preview` for faster generation

### Job Not Found

Jobs are stored locally in `~/.local/share/kalpa/jobs/`. If a job is not found:
- The job ID may be incorrect
- The job may have been deleted
- Check the jobs directory exists and has read permissions

## Best Practices

1. **Use appropriate models** - Choose models based on your needs:
   - Fast models for quick iterations
   - Pro models for high-quality output

2. **Monitor long-running jobs** - For video generation:
   - Check job status periodically
   - Clean up completed jobs to avoid clutter

3. **Handle rate limits** - Vertex AI has quotas:
   - Space out requests if hitting limits
   - Consider batch processing for multiple generations

4. **Secure credentials** - Keep service account JSON secure:
   - Use appropriate file permissions (600)
   - Don't commit to version control
   - Rotate credentials periodically

## Next Steps

- Explore different models for various use cases
- Set up default models in configuration
- Integrate with CI/CD pipelines
- Build automation workflows using the JSON output

## Related Documentation

- [Vertex AI Setup Guide](./VERTEX_AI_SETUP.md)
- [Job System API](../crates/core/src/jobs.rs)
- [Provider Implementation](../crates/core/src/providers/vertex.rs)
