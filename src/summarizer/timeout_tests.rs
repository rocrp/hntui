use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

#[derive(Clone, Copy)]
enum StallAt {
    Headers,
    FirstChunk,
    MidStream,
}

/// A real SSE endpoint that stays open until the test drops the stop sender.
fn stalled_server(
    stall_at: StallAt,
    complete_after: Option<Duration>,
) -> (String, mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind SSE server");
    let address = listener.local_addr().expect("server address");
    let (stop, stopped) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept request");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        let mut reader = BufReader::new(&mut socket);
        let mut length = None;
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).expect("read headers") > 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = Some(value.trim().parse::<usize>().expect("content length"));
            }
        }
        let mut body = vec![0; length.expect("request body length")];
        reader.read_exact(&mut body).expect("read request body");
        if !matches!(stall_at, StallAt::Headers) {
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                )
                .expect("write SSE headers");
        }
        if matches!(stall_at, StallAt::MidStream) {
            socket
                .write_all(b"data: {\"model\":\"test\",\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n")
                .expect("write first chunk");
        }
        match stopped.recv_timeout(complete_after.unwrap_or(Duration::from_secs(5))) {
            Err(mpsc::RecvTimeoutError::Disconnected) | Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) if complete_after.is_some() => {
                socket
                    .write_all(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
                    .expect("complete stream");
            }
            Err(error) => panic!("test did not release stalled server: {error}"),
        }
    });
    (format!("http://{address}"), stop, server)
}

fn http_summarizer(base_url: String) -> Summarizer {
    Summarizer::new(
        Some(SummarizeConfig {
            model: "openai/test".to_string(),
            base_url: Some(base_url),
            timeout_secs: 1,
            ..config()
        }),
        Some("test-key".to_string()),
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client"),
    )
}

#[tokio::test]
async fn summary_timeout_before_first_chunk_reports_configured_limit() {
    assert_initial_timeout(StallAt::FirstChunk).await;
}

#[tokio::test]
async fn summary_timeout_before_headers_reports_configured_limit() {
    assert_initial_timeout(StallAt::Headers).await;
}

async fn assert_initial_timeout(stall_at: StallAt) {
    let (url, stop, server) = stalled_server(stall_at, None);
    let summarizer = http_summarizer(url);
    let events = tokio::time::timeout(
        Duration::from_secs(3),
        summarizer.summarize(input()).collect::<Vec<_>>(),
    )
    .await;
    drop(stop);
    server.join().expect("server thread");
    let events = events.expect("configured 1s timeout must beat the 3s watchdog");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].as_ref().unwrap_err().to_string(),
        "timed out after 1s"
    );
}

#[tokio::test]
async fn summary_timeout_mid_stream_preserves_partial_content_without_complete() {
    let (url, stop, server) = stalled_server(StallAt::MidStream, None);
    let summarizer = http_summarizer(url);
    let events = tokio::time::timeout(
        Duration::from_secs(3),
        summarizer.summarize(input()).collect::<Vec<_>>(),
    )
    .await;
    drop(stop);
    server.join().expect("server thread");
    let events = events.expect("configured 1s timeout must beat the 3s watchdog");
    assert_eq!(events.len(), 3, "started, partial content, then error");
    assert!(matches!(&events[0], Ok(SummaryEvent::Started { .. })));
    assert_eq!(
        events[1].as_ref().expect("partial chunk"),
        &SummaryEvent::Chunk {
            content: "partial".to_string(),
            reasoning: String::new(),
        }
    );
    assert_eq!(
        events[2].as_ref().unwrap_err().to_string(),
        "timed out after 1s"
    );
}

#[tokio::test]
#[ignore = "real-time regression for the old 120s SDK deadline; takes 121s"]
async fn default_summary_timeout_allows_a_stream_longer_than_120_seconds() {
    let (url, stop, server) = stalled_server(StallAt::MidStream, Some(Duration::from_secs(121)));
    let summary_config: SummarizeConfig =
        toml::from_str(&format!("model = 'openai/test'\nbase_url = '{url}'\n"))
            .expect("config with default timeout");
    let summarizer = Summarizer::new(
        Some(summary_config),
        Some("test-key".to_string()),
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client"),
    );
    let events = tokio::time::timeout(
        Duration::from_secs(130),
        summarizer.summarize(input()).collect::<Vec<_>>(),
    )
    .await;
    drop(stop);
    server.join().expect("server thread");
    let events = events.expect("stream should finish after 121s");
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    assert!(matches!(
        events.last(),
        Some(Ok(SummaryEvent::Complete { .. }))
    ));
}
