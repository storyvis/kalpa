//! Wire regressions exposed by the official upstream schema refresh.
use kalpa_libgen::{falai, gemini, openai, vertex};
use serde_json::json;

#[test]
fn chat_completion_usage_keeps_prompt_and_completion_tokens() {
    let response: openai::types::ChatCompletionResponse = serde_json::from_value(json!({
        "id": "chatcmpl-fixture",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello"}}],
        "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}
    }))
    .unwrap();
    let wire = serde_json::to_value(response).unwrap();
    assert_eq!(wire["usage"]["prompt_tokens"], 11);
    assert_eq!(wire["usage"]["completion_tokens"], 7);
}

#[test]
fn gemini_media_uses_canonical_json_names() {
    let request: gemini::types::GenerateContentRequest = serde_json::from_value(json!({
        "contents": [{"role": "user", "parts": [
            {"inlineData": {"mimeType": "image/png", "data": "AA=="}},
            {"fileData": {"mimeType": "image/jpeg", "fileUri": "gs://test/image.jpg"}}
        ]}],
        "generationConfig": {"responseModalities": ["TEXT", "IMAGE"]}
    }))
    .unwrap();
    let wire = serde_json::to_value(request).unwrap();
    assert_eq!(
        wire["contents"][0]["parts"][0]["inlineData"]["mimeType"],
        "image/png"
    );
    assert_eq!(
        wire["contents"][0]["parts"][1]["fileData"]["fileUri"],
        "gs://test/image.jpg"
    );
    assert_eq!(
        wire["generationConfig"]["responseModalities"],
        json!(["TEXT", "IMAGE"])
    );
}

#[test]
fn fal_queue_urls_can_be_null() {
    let response: falai::types::FalQueueSubmitResponse = serde_json::from_value(json!({
        "request_id": "fixture-request",
        "status_url": null, "response_url": null, "cancel_url": null
    }))
    .unwrap();
    assert!(response.status_url.is_none());
    assert!(response.response_url.is_none());
}

#[test]
fn vertex_operation_retains_model_specific_video_result() {
    let response: vertex::types::OperationResponse = serde_json::from_value(json!({
        "name": "projects/test/locations/us-central1/operations/fixture",
        "done": true,
        "response": {
            "@type": "type.googleapis.com/cloud.ai.large_models.vision.GenerateVideoResponse",
            "generateVideoResponse": {"generatedSamples": [{"video": {"uri": "gs://test/video.mp4"}}]}
        }
    })).unwrap();
    let wire = serde_json::to_value(response).unwrap();
    assert_eq!(
        wire["response"]["generateVideoResponse"]["generatedSamples"][0]["video"]["uri"],
        "gs://test/video.mp4"
    );
}

#[test]
fn video_duration_is_a_string_on_the_wire() {
    let response: openai::types::VideoGenerationResponse = serde_json::from_value(json!({
        "id": "video-fixture", "seconds": "8"
    }))
    .unwrap();
    assert_eq!(serde_json::to_value(response).unwrap()["seconds"], "8");
}
