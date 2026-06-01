# Fal.ai Integration Guide

This document describes the fal.ai integration in Kalpa, including supported models, API usage, and queue-based video generation.

## Overview

Fal.ai provides fast, scalable AI image and video generation through a REST API. Kalpa integrates with fal.ai for:

- **Text-to-Image**: Fast image generation using FLUX, Stable Diffusion, and other models
- **Text-to-Video**: Generate videos from text prompts using various state-of-the-art models
- **Image-to-Video**: Animate still images into videos

## Authentication

All fal.ai requests require an API key with the format: `Authorization: Key YOUR_FAL_KEY`

Configure your fal.ai API key:

```bash
kalpa configure --set fal.api_key YOUR_API_KEY
```

## API Architecture

### Direct API (Image Generation)
Fast image models use direct synchronous endpoints at `https://fal.run/{model_id}`

### Queue API (Video Generation)
Video generation uses a queue-based asynchronous API at `https://queue.fal.run/{model_id}`

#### Queue Flow

1. **Submit Request** → Returns `request_id`
2. **Poll Status** → Check `IN_QUEUE`, `IN_PROGRESS`, or `COMPLETED`
3. **Get Result** → Fetch final video URL

Kalpa handles this flow automatically with automatic polling every 2 seconds.

## Supported Text-to-Image Models

| Model ID | Description | Speed |
|----------|-------------|-------|
| `fal-ai/flux/dev` | FLUX Dev - high quality | Fast |
| `fal-ai/flux/schnell` | FLUX Schnell - very fast | Very Fast |
| `fal-ai/flux-pro` | FLUX Pro - best quality | Fast |
| `fal-ai/flux-realism` | FLUX Realism | Fast |
| `fal-ai/recraft-v3` | Recraft V3 | Fast |
| `fal-ai/aura-flow` | Aura Flow | Fast |
| `fal-ai/stable-diffusion-v3-medium` | Stable Diffusion V3 | Medium |
| `fal-ai/fast-sdxl` | Fast SDXL (default) | Very Fast |

## Supported Text-to-Video Models

| Model ID | Description | Quality | Speed | Cost |
|----------|-------------|---------|-------|------|
| `fal-ai/minimax/video-01` | Great for human motion, 6-sec clips | High | ~4min | Medium |
| `fal-ai/minimax/video-01-live` | Live version, 1280×720, 25fps | High | Fast | Medium |
| `fal-ai/hunyuan-video` | High quality, strong text-video alignment | Very High | ~4min | ~$0.40/video |
| `fal-ai/mochi-v1` | High-fidelity motion, strong prompt adherence | High | Medium | Medium |
| `fal-ai/kling-video/v1/standard/text-to-video` | Kling V1 Standard | Medium | Medium | Low |
| `fal-ai/kling-video/v1.5/standard/text-to-video` | Kling V1.5 Standard | High | Medium | Low |
| `fal-ai/kling-video/v1.6/standard/text-to-video` | Kling V1.6 Standard | High | Medium | Low |
| `fal-ai/kling-video/v2.1/master/text-to-video` | Kling 2.0 Master | Very High | Slow | High |
| `fal-ai/kling-video/v2.6/pro/text-to-video` | Kling 2.6 Pro (native audio) | Very High | Slow | $0.07-0.14/sec |
| `fal-ai/wan/v2.2-a14b/text-to-video` | Wan V2.2 | High | Medium | Medium |
| `fal-ai/ltx-2/text-to-video` | LTX Video 2.0 Pro | High | Medium | Medium |
| `fal-ai/ltx-2.3/text-to-video` | LTX 2.3 (4K, up to 20sec, native audio) | Very High | Slow | High |
| `fal-ai/veo3` | Google Veo 3 (native audio) | Very High | Slow | High |
| `fal-ai/veo3.1` | Google Veo 3.1 (up to 4K, 8sec extendable) | Very High | Slow | Very High |
| `bytedance/seedance-2.0/text-to-video` | Seedance 2.0 (cinematic, native audio) | Very High | Slow | High |
| `bytedance/seedance-2.0/fast/text-to-video` | Seedance 2.0 Fast tier | High | Fast | Medium |

## Supported Image-to-Video Models

| Model ID | Description | Quality | Speed |
|----------|-------------|---------|-------|
| `fal-ai/veo2/image-to-video` | Google Veo 2 | Very High | Slow |
| `fal-ai/veo3/image-to-video` | Google Veo 3 | Very High | Slow |
| `fal-ai/luma-dream-machine/image-to-video` | Luma Dream Machine | High | Medium |
| `fal-ai/kling-video/v2.1/master/image-to-video` | Kling 2.1 Master | Very High | Slow |
| `fal-ai/kling-video/v1.6/pro/image-to-video` | Kling 1.6 Pro | High | Medium |
| `fal-ai/minimax/video-01-live/image-to-video` | Minimax Live | High | Fast |
| `fal-ai/pixverse/v4.5/image-to-video` | Pixverse V4.5 | High | Medium |
| `bytedance/seedance-2.0/image-to-video` | Seedance 2.0 | Very High | Slow |
| `fal-ai/luma-dream-machine` | Luma (legacy) | High | Medium |

## Usage Examples

### Image Generation

```bash
# Generate image with default model (fast-sdxl)
kalpa generate -f image "a cat on mars"

# Use specific model
kalpa generate -f image --model fal-ai/flux-pro "a futuristic city at sunset"

# JSON output
kalpa generate -f image --json "cyberpunk landscape"
```

### Text-to-Video Generation

```bash
# Generate video with default model
kalpa generate -f video "a robot dancing in the rain"

# Use specific high-quality model
kalpa generate -f video --model fal-ai/hunyuan-video "ocean waves crashing on beach"

# Use fast model
kalpa generate -f video --model fal-ai/minimax/video-01-live "cat playing with yarn"

# Use Kling 2.6 Pro with native audio support
kalpa generate -f video --model fal-ai/kling-video/v2.6/pro/text-to-video "jazz band performing"

# Use Google Veo 3.1 for 4K quality
kalpa generate -f video --model fal-ai/veo3.1 "northern lights over mountains"

# Use Seedance 2.0 for cinematic quality with audio
kalpa generate -f video --model bytedance/seedance-2.0/text-to-video "sunset over city skyline"
```

### Image-to-Video Generation

Image-to-video models are currently planned for future support via CLI. For now, they can be accessed programmatically:

```rust
use kalpa_core::providers::FalAIProvider;
use kalpa_core::provider::VideoGenerationProvider;
use kalpa_core::types::VideoGenerationRequest;

let provider = FalAIProvider::new(api_key);
let request = VideoGenerationRequest {
    model: "fal-ai/kling-video/v2.1/master/image-to-video".to_string(),
    prompt: "make the person wave".to_string(),
    image_url: Some("https://example.com/image.jpg".to_string()),
    duration: None,
};

let response = provider.generate_video(&request).await?;
```

## Queue Status Types

When generating videos, you'll see different status messages:

- **IN_QUEUE** - Waiting for GPU resources (shows queue position)
- **IN_PROGRESS** - Video is being generated
- **COMPLETED** - Video is ready, downloading result
- **FAILED** - Generation failed with error message

## Implementation Details

### Provider Structure

The `FalAIProvider` struct implements both `ImageGenerationProvider` and `VideoGenerationProvider` traits:

```rust
pub struct FalAIProvider {
    client: Client,
    api_key: String,
}
```

### Queue Methods

- `queue_submit()` - Submit request to queue, get request_id
- `queue_status()` - Check current status with logs
- `queue_result()` - Fetch final result
- `queue_submit_and_wait()` - Submit and poll until complete (used internally)

### Automatic Polling

Video generation automatically polls the queue every 2 seconds until completion:

1. Submit to `https://queue.fal.run/{model_id}`
2. Poll `https://queue.fal.run/{model_id}/requests/{request_id}/status?logs=1`
3. When `COMPLETED`, fetch from `https://queue.fal.run/{model_id}/requests/{request_id}`

### Error Handling

- Network errors return HTTP 500
- API errors include original status code and message
- Failed generations include error details from logs

## Model Selection Guidelines

### For Speed
- Image: `fal-ai/flux/schnell` or `fal-ai/fast-sdxl`
- Video: `fal-ai/minimax/video-01-live` or `bytedance/seedance-2.0/fast/text-to-video`

### For Quality
- Image: `fal-ai/flux-pro` or `fal-ai/flux-realism`
- Video: `fal-ai/veo3.1`, `fal-ai/hunyuan-video`, or `bytedance/seedance-2.0/text-to-video`

### For Audio
- `fal-ai/kling-video/v2.6/pro/text-to-video` (native audio, $0.07-0.14/sec)
- `fal-ai/ltx-2.3/text-to-video` (4K, native audio)
- `fal-ai/veo3` or `fal-ai/veo3.1` (native audio)
- `bytedance/seedance-2.0/text-to-video` (cinematic, native audio)

### For 4K Resolution
- `fal-ai/ltx-2.3/text-to-video` (up to 20 seconds)
- `fal-ai/veo3.1` (up to 8 seconds, extendable)

### For Cost-Effectiveness
- Image: `fal-ai/fast-sdxl`
- Video: `fal-ai/kling-video/v1.5/standard/text-to-video`

## Pricing Notes

- Images are typically per-generation
- Videos are priced per second or per video
- Queue API has no additional cost
- Failed generations (5xx errors) are auto-retried and not billed
- See [fal.ai pricing](https://fal.ai/pricing) for current rates

## Limitations

- Video generation can take several minutes
- Files expire after configured duration (use `X-Fal-Object-Lifecycle-Preference` header)
- Some models have resolution/duration limits
- Rate limits apply based on your plan

## Troubleshooting

### "Fal AI queue submit failed"
- Check API key is configured: `kalpa status`
- Verify network connectivity
- Check fal.ai service status

### Long wait times
- Video generation is compute-intensive
- Queue position indicates your place in line
- Consider using faster models for testing

### "Failed to parse queue status response"
- This may indicate API changes
- File an issue with the response details

## Future Enhancements

- [ ] CLI support for image-to-video with `--image-url` flag
- [ ] Duration parameter support
- [ ] Audio generation control parameters
- [ ] Resolution/aspect ratio configuration
- [ ] Streaming status updates (SSE)
- [ ] Webhook support for completion notifications
- [ ] Batch processing support

## References

- [fal.ai Documentation](https://docs.fal.ai/)
- [fal.ai Model APIs](https://docs.fal.ai/model-apis)
- [Queue API Documentation](https://docs.fal.ai/queue)
- [Pricing](https://fal.ai/pricing)
