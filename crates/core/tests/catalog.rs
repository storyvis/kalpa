use kalpa_core::{HttpModelCatalog, KalpaError, ModelCatalog};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn openai_lists_models_and_finds_one_by_a_single_lookup() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/models")).and(header("authorization", "Bearer sk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "id": "gpt-4o" }, { "id": "gpt-4.1" }] })))
        .mount(&server).await;
    Mock::given(path("/models/gpt-4o")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "gpt-4o" }))).mount(&server).await;
    Mock::given(path("/models/nope")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    let c = HttpModelCatalog::openai("sk".into()).with_base_url(&server.uri());
    let ids: Vec<_> = c.list_models().await.unwrap().into_iter().map(|m| m.id).collect();
    assert_eq!(ids, ["gpt-4o", "gpt-4.1"]);
    assert!(c.model_exists("gpt-4o").await.unwrap());
    assert!(!c.model_exists("nope").await.unwrap(), "only a 404 means the model does not exist");
}

#[tokio::test]
async fn a_refused_key_or_server_error_is_an_error_not_a_missing_model() {
    let server = MockServer::start().await;
    Mock::given(path("/models/a")).respond_with(ResponseTemplate::new(401).set_body_string("bad key")).mount(&server).await;
    Mock::given(path("/models/b")).respond_with(ResponseTemplate::new(503)).mount(&server).await;
    let c = HttpModelCatalog::openai("sk".into()).with_base_url(&server.uri());
    assert!(matches!(c.model_exists("a").await, Err(KalpaError::ProviderError { status: 401, .. })));
    assert!(matches!(c.model_exists("b").await, Err(KalpaError::ProviderError { status: 503, .. })));
}

#[tokio::test]
async fn claude_pages_through_its_list_with_its_own_headers() {
    let server = MockServer::start().await;
    Mock::given(path("/models")).and(query_param("after_id", "m1")).and(header("x-api-key", "ak"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "id": "m2", "display_name": "Two" }], "has_more": false })))
        .mount(&server).await;
    Mock::given(path("/models")).and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "id": "m1", "display_name": "One" }], "has_more": true, "last_id": "m1" })))
        .mount(&server).await;
    let c = HttpModelCatalog::claude("ak".into()).with_base_url(&server.uri());
    let all = c.list_models().await.unwrap();
    assert_eq!(all.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m1", "m2"]);
    assert_eq!(all[1].display_name.as_deref(), Some("Two"));
}

#[tokio::test]
async fn gemini_strips_the_models_prefix_and_sends_the_key_in_a_header() {
    let server = MockServer::start().await;
    Mock::given(path("/models")).and(header("x-goog-api-key", "gk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [{ "name": "models/gemini-2.0-flash", "displayName": "Flash" }] })))
        .mount(&server).await;
    Mock::given(path("/models/gemini-2.0-flash")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).mount(&server).await;
    let c = HttpModelCatalog::gemini("gk".into()).with_base_url(&server.uri());
    assert_eq!(c.list_models().await.unwrap()[0].id, "gemini-2.0-flash");
    assert!(c.model_exists("models/gemini-2.0-flash").await.unwrap());
}

#[tokio::test]
async fn a_compatible_server_is_searched_in_its_list_and_may_need_no_key() {
    let server = MockServer::start().await;
    Mock::given(path("/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [{ "id": "meta-llama/llama-3.1-8b" }] }))).mount(&server).await;
    let c = HttpModelCatalog::openai_compatible("openrouter", &server.uri(), None);
    assert!(c.model_exists("meta-llama/llama-3.1-8b").await.unwrap());
    assert!(!c.model_exists("meta-llama/other").await.unwrap());
    assert!(server.received_requests().await.unwrap().iter().all(|r| !r.headers.contains_key("authorization")));
}
