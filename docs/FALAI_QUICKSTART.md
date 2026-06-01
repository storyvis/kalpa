q# Fal.ai Quick Start Guide

Get started with fal.ai image and video generation in minutes.

## Setup

1. **Get your fal.ai API key** from [fal.ai](https://fal.ai)

2. **Configure Kalpa**:
   ```bash
   kalpa configure --set fal.api_key YOUR_FAL_KEY
   ```

3. **Verify configuration**:
   ```bash
   kalpa status
   ```

## Quick Examples

### Generate an Image (Fast)
```bash
# Default fast model
kalpa generate -f image "a futuristic city at sunset"

# High quality
kalpa generate -f image --model fal-ai/flux-pro "cyberpunk samurai"
```

### Generate a Video
```bash
# Default model (takes a few minutes)
kalpa generate -f video "a cat playing with yarn"

# Fast model (~30 seconds)
kalpa generate -f video --model fal-ai/minimax/video-01-live "ocean waves"

# High quality with audio
kalpa generate -f video --model fal-ai/kling-video/v2.6/pro/text-to-video "jazz band"

# 4K quality
kalpa generate -f video --model fal-ai/veo3.1 "northern lights"
```

## Popular Models

### Images
- **fal-ai/fast-sdxl** - Fastest, good quality (default)
- **fal-ai/flux-pro** - Best quality
- **fal-ai/flux/schnell** - Very fast

### Videos  
- **fal-ai/minimax/video-01-live** - Fast, good for testing
- **fal-ai/hunyuan-video** - High quality, good text alignment
- **fal-ai/kling-video/v2.6/pro/text-to-video** - Pro quality with audio
- **fal-ai/veo3.1** - 4K, best quality
- **bytedance/seedance-2.0/text-to-video** - Cinematic with audio

## What Happens During Video Generation?

When you generate a video, Kalpa:

1. ✓ Submits your request to fal.ai queue
2. ⏳ Shows queue position while waiting
3. 🔄 Displays "Processing..." when generation starts
4. ✓ Downloads the video when complete
5. 📁 Saves as `video_<timestamp>.mp4`

Example output:
```
  → Generating video with Fal.ai (fal-ai/hunyuan-video)...
  ℹ Video generation may take several minutes
In queue, position: 2
In queue, position: 1
Processing...
Processing...

✓ Video generated!
  URL: https://v3.fal.media/files/xxx.mp4
```

## JSON Output

For scripting or integration:

```bash
kalpa generate -f video --json "dancing robot" | jq .
```

Output:
```json
{
  "provider": "fal",
  "model": "fal-ai/kling-video/v1/standard/text-to-video",
  "type": "video",
  "prompt": "dancing robot",
  "url": "https://v3.fal.media/files/xxx.mp4"
}
```

## Tips

- **Test with fast models first**: Use `fal-ai/minimax/video-01-live` for quick testing
- **Video takes time**: High-quality models can take 5-10 minutes
- **Watch queue position**: Lower numbers = faster start
- **Check your balance**: Video generation costs vary by model

## Troubleshooting

### "No API key configured"
```bash
kalpa configure --set fal.api_key YOUR_KEY
```

### "Model not supported"
Check available models:
```bash
# In generate command error message, or see docs/FALAI_INTEGRATION.md
```

### Long wait times
- This is normal for video generation
- Try faster models: `fal-ai/minimax/video-01-live`
- Queue position shows your place in line

## Next Steps

- Read full documentation: `docs/FALAI_INTEGRATION.md`
- Explore all models: See the integration guide
- Try image-to-video: Coming soon via CLI

## Cost Estimates

Approximate pricing (check [fal.ai/pricing](https://fal.ai/pricing) for current rates):

- **Images**: ~$0.01-0.05 per image
- **Videos (standard)**: ~$0.10-0.50 per video
- **Videos (premium with audio)**: ~$0.07-0.14 per second

Lower quality/faster models are more cost-effective for testing.
