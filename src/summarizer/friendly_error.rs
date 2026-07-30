use std::time::Duration;

const BODY_EXCERPT_CHARS: usize = 120;
const FALLBACK_MESSAGE_CHARS: usize = 160;

pub(crate) fn friendly_llm_error(
    error: &smolllm::Error,
    request_timeout: Option<Duration>,
) -> String {
    match error {
        smolllm::Error::Request(error) if error.is_timeout() => request_timeout.map_or_else(
            || "request timed out".to_string(),
            |timeout| format!("timed out after {}s", timeout.as_secs()),
        ),
        smolllm::Error::Request(error) if error.is_connect() => {
            let host = error
                .url()
                .and_then(reqwest::Url::host_str)
                .unwrap_or("endpoint");
            format!("cannot reach {host}")
        }
        smolllm::Error::Http {
            status: 401 | 403,
            body,
        } => with_body_excerpt("check API key", body),
        smolllm::Error::Http { status: 404, .. } => {
            "endpoint path wrong? if your URL is already the full endpoint, end it with `#`"
                .to_string()
        }
        smolllm::Error::Http { status, body } => with_body_excerpt(&format!("HTTP {status}"), body),
        _ => truncate(&error.to_string(), FALLBACK_MESSAGE_CHARS),
    }
}

fn with_body_excerpt(prefix: &str, body: &str) -> String {
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix} · {}", truncate(&normalized, BODY_EXCERPT_CHARS))
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn http_errors_map_to_actionable_bounded_messages() {
        let cases = [
            (
                smolllm::Error::Http {
                    status: 401,
                    body: "invalid token".to_string(),
                },
                "check API key · invalid token",
            ),
            (
                smolllm::Error::Http {
                    status: 403,
                    body: String::new(),
                },
                "check API key",
            ),
            (
                smolllm::Error::Http {
                    status: 404,
                    body: "not found".to_string(),
                },
                "endpoint path wrong? if your URL is already the full endpoint, end it with `#`",
            ),
            (
                smolllm::Error::Http {
                    status: 418,
                    body: "short\n  and   stout".to_string(),
                },
                "HTTP 418 · short and stout",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(friendly_llm_error(&error, None), expected);
        }

        let long_body = "x".repeat(500);
        let message = friendly_llm_error(
            &smolllm::Error::Http {
                status: 500,
                body: long_body,
            },
            None,
        );
        assert!(message.ends_with('…'));
        assert!(message.chars().count() <= 133);
    }

    #[tokio::test]
    async fn connection_failure_names_the_unreachable_host() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind unused port");
        let address = listener.local_addr().expect("listener address");
        drop(listener);
        let error = reqwest::Client::new()
            .get(format!("http://{address}"))
            .send()
            .await
            .expect_err("unused port must reject the connection");

        assert!(error.is_connect());
        assert_eq!(
            friendly_llm_error(&smolllm::Error::Request(error), None),
            "cannot reach 127.0.0.1"
        );
    }

    #[tokio::test]
    async fn request_timeout_uses_the_callers_timeout_context() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind timeout server");
        let address = listener.local_addr().expect("listener address");
        let server = std::thread::spawn(move || {
            let _connection = listener.accept().expect("accept request").0;
            std::thread::sleep(Duration::from_millis(100));
        });
        let error = reqwest::Client::builder()
            .timeout(Duration::from_millis(10))
            .build()
            .expect("timeout client")
            .get(format!("http://{address}"))
            .send()
            .await
            .expect_err("server never responds");

        assert!(error.is_timeout());
        assert_eq!(
            friendly_llm_error(
                &smolllm::Error::Request(error),
                Some(Duration::from_secs(15))
            ),
            "timed out after 15s"
        );
        server.join().expect("timeout server");
    }
}
