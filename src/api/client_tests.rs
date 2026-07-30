use super::{ApiBackend, HnClient, Story};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

fn serve_once(
    respond: impl FnOnce(&mut TcpStream) + Send + 'static,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).expect("read request");
        respond(&mut stream);
    });
    (format!("http://{address}"), handle)
}

fn serve_chunked_once(body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
    serve_once(move |stream| {
        let headers = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json; charset=utf-8\r\n\
             Transfer-Encoding: chunked\r\n\
             Connection: close\r\n\
             \r\n\
             {:x}\r\n",
            body.len()
        );
        stream.write_all(headers.as_bytes()).expect("write headers");
        stream.write_all(&body).expect("write body");
        stream
            .write_all(b"\r\n0\r\n\r\n")
            .expect("finish chunked body");
    })
}

fn serve_short_body_once(
    body: Vec<u8>,
    declared_length: usize,
) -> (String, thread::JoinHandle<()>) {
    serve_once(move |stream| {
        let headers = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {declared_length}\r\n\
             Connection: close\r\n\
             \r\n"
        );
        stream.write_all(headers.as_bytes()).expect("write headers");
        stream.write_all(&body).expect("write short body");
    })
}

fn serve_status_once(status: &str) -> (String, thread::JoinHandle<()>) {
    let status = status.to_string();
    serve_once(move |stream| {
        let response = format!(
            "HTTP/1.1 {status}\r\n\
             Content-Length: 0\r\n\
             Connection: close\r\n\
             \r\n"
        );
        stream
            .write_all(response.as_bytes())
            .expect("write status response");
    })
}

fn hackerweb_client(base_url: String) -> HnClient {
    let http = reqwest::Client::builder()
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .expect("build HTTP client");
    HnClient::new(http, base_url, ApiBackend::HackerWeb, 1, 1, None).expect("build HN client")
}

fn story(id: u64) -> Story {
    Story {
        id,
        title: "Test story".to_string(),
        url: Some("https://example.com".to_string()),
        text: None,
        score: 1,
        by: "test".to_string(),
        time: 1,
        comment_count: 1,
        kids: Vec::new(),
    }
}

#[tokio::test]
async fn incomplete_hackerweb_json_reports_response_evidence_and_recovery() {
    let mut body = br#"{"comments":[{"id":1,"content":""#.to_vec();
    body.resize(8256, b'a');
    let (base_url, server) = serve_chunked_once(body);

    let error = hackerweb_client(base_url)
        .fetch_comment_roots(&story(49_104_117))
        .await
        .expect_err("incomplete JSON must fail");

    server.join().expect("server thread");
    assert_eq!(
        format!("{error:#}"),
        "HackerWeb item 49104117: incomplete JSON (HTTP 200 OK, 8256 B; EOF 1:8256); \
         retry or --api-backend firebase"
    );
}

#[tokio::test]
async fn transport_truncation_is_reported_as_a_body_read_failure() {
    let body = vec![b'a'; 8256];
    let declared_length = body.len() + 1_000;
    let (base_url, server) = serve_short_body_once(body, declared_length);

    let error = hackerweb_client(base_url)
        .fetch_comment_roots(&story(49_104_117))
        .await
        .expect_err("short HTTP body must fail");

    server.join().expect("server thread");
    let message = format!("{error:#}");
    assert!(
        message.starts_with(
            "HackerWeb item 49104117: response body read failed \
             (HTTP 200 OK, expected 9256 B); retry or --api-backend firebase"
        ),
        "{message}"
    );
    assert!(!message.contains("incomplete JSON"), "{message}");
}

#[tokio::test]
async fn incompatible_hackerweb_shape_does_not_echo_response_values() {
    let secret = "TOP_SECRET_SENTINEL";
    let body = format!(r#"{{"comments":"{secret}"}}"#).into_bytes();
    let body_len = body.len();
    let (base_url, server) = serve_chunked_once(body);

    let error = hackerweb_client(base_url)
        .fetch_comment_roots(&story(49_104_117))
        .await
        .expect_err("incompatible JSON shape must fail");

    server.join().expect("server thread");
    let message = format!("{error:#}");
    assert!(
        message.starts_with(
            "HackerWeb item 49104117: incompatible JSON \
             (HTTP 200 OK"
        ),
        "{message}"
    );
    assert!(message.contains(&format!("{body_len} B")), "{message}");
    assert!(message.contains("schema mismatch 1:"), "{message}");
    assert!(!message.contains(secret), "{message}");
}

#[tokio::test]
async fn valid_hackerweb_json_larger_than_8k_decodes() {
    let content = "a".repeat(12_000);
    let body = format!(r#"{{"comments":[{{"id":1,"content":"{content}"}}]}}"#).into_bytes();
    assert!(body.len() > 8 * 1024);
    let (base_url, server) = serve_chunked_once(body);

    let thread = hackerweb_client(base_url)
        .fetch_comment_roots(&story(49_104_117))
        .await
        .expect("valid large JSON must decode");

    server.join().expect("server thread");
    assert_eq!(thread.comments.len(), 1);
    assert_eq!(thread.comments[0].comment.text, content);
}

#[tokio::test]
async fn hackerweb_http_status_is_reported_separately_without_the_url() {
    let (base_url, server) = serve_status_once("503 Service Unavailable");

    let error = hackerweb_client(base_url.clone())
        .fetch_comment_roots(&story(49_104_117))
        .await
        .expect_err("server error must fail");

    server.join().expect("server thread");
    let message = format!("{error:#}");
    assert_eq!(
        message,
        "HackerWeb returned an error for item id=49104117: HTTP status server error \
         (503 Service Unavailable)"
    );
    assert!(!message.contains(&base_url), "{message}");
}

#[tokio::test]
async fn malformed_hackerweb_json_reports_syntax_without_echoing_the_body() {
    let secret = "TOP_SECRET_SYNTAX_SENTINEL";
    let body = secret.as_bytes().to_vec();
    let (base_url, server) = serve_chunked_once(body);

    let error = hackerweb_client(base_url)
        .fetch_comment_roots(&story(49_104_117))
        .await
        .expect_err("malformed JSON must fail");

    server.join().expect("server thread");
    let message = format!("{error:#}");
    assert!(
        message.starts_with("HackerWeb item 49104117: invalid JSON (HTTP 200 OK"),
        "{message}"
    );
    assert!(message.contains("parse error 1:"), "{message}");
    assert!(!message.contains(secret), "{message}");
}

#[tokio::test]
async fn hackerweb_request_failure_does_not_expose_the_base_url() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve closed address");
    let address = listener.local_addr().expect("closed address");
    drop(listener);
    let secret = "SENSITIVE_BASE_URL_TOKEN";
    let base_url = format!("http://{address}/{secret}");

    let error = hackerweb_client(base_url)
        .fetch_comment_roots(&story(49_104_117))
        .await
        .expect_err("closed address must fail");

    let message = format!("{error:#}");
    assert!(
        message.starts_with("HackerWeb request failed for item id=49104117"),
        "{message}"
    );
    assert!(!message.contains(secret), "{message}");
}
