use kalpa_libgen::{falai, progenitor_client::Error, MAX_RESPONSE_BYTES};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(response: String) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 8192];
        let n = socket.read(&mut request).await.unwrap();
        socket.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8_lossy(&request[..n]).into_owned()
    });
    (base, task)
}

#[tokio::test]
async fn generated_client_rejects_oversized_response_before_deserialization() {
    let (base, task) = server(format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_RESPONSE_BYTES + 1
    ))
    .await;
    let error = falai::Client::new(&base)
        .get_queue_result("owner/model", "id")
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Custom(message) if message.contains("exceeds")));
    task.await.unwrap();
}

#[tokio::test]
async fn generated_errors_preserve_status_without_secrets() {
    let (base, task) = server("HTTP/1.1 429 Too Many Requests\r\nContent-Length: 10\r\nConnection: close\r\n\r\nsecret-key".into()).await;
    let error = falai::Client::new(&base)
        .get_queue_result("owner/model", "id")
        .await
        .unwrap_err();
    assert_eq!(error.status().unwrap(), 429);
    assert!(!format!("{error:?}").contains("secret-key"));
    task.await.unwrap();
}

#[tokio::test]
async fn fal_paths_preserve_model_segments_but_escape_query_and_request_id() {
    let (base, task) =
        server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into()).await;
    falai::Client::new(&base)
        .get_queue_result("owner/model?escape", "id/other")
        .await
        .unwrap();
    let request = task.await.unwrap();
    assert!(
        request.starts_with("GET /owner/model%3Fescape/requests/id%2Fother "),
        "{request}"
    );
    assert!(kalpa_libgen::encode_model_path::<()>("owner/../model").is_err());
}

#[tokio::test]
async fn chunked_response_is_capped_without_content_length() {
    let (base, task) = server("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\n123\r\n3\r\n456\r\n0\r\n\r\n".into()).await;
    let response = reqwest::Client::new().get(base).send().await.unwrap();
    assert!(matches!(
        kalpa_libgen::read_bounded(response, 5).await,
        Err(kalpa_libgen::BodyError::TooLarge(5))
    ));
    task.await.unwrap();
}

/// CPU/loopback-only reproduction of the pooled Vertex polling-client change.
#[tokio::test]
#[ignore = "manual transport microbenchmark"]
async fn benchmark_client_reuse() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let connections = Arc::new(AtomicUsize::new(0));
    let count = connections.clone();
    let server = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut bytes = [0; 4096];
                loop {
                    match stream.read(&mut bytes).await {
                        Ok(0) | Err(_) => break,
                        _ => {}
                    }
                    if stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
    for reuse in [false, true] {
        let pooled = reqwest::Client::new();
        let before = connections.load(Ordering::SeqCst);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let client = if reuse {
                pooled.clone()
            } else {
                reqwest::Client::new()
            };
            client
                .get(&url)
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
        }
        let opened = connections.load(Ordering::SeqCst) - before;
        println!(
            "reuse={reuse}: 100 requests, {opened} connections, {:?}",
            start.elapsed()
        );
        assert_eq!(opened, if reuse { 1 } else { 100 });
    }
    server.abort();
}
