use anyhow::{Context, Result};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::error::Category;

pub(super) async fn decode_json<T>(response: Response, target: String) -> Result<T>
where
    T: DeserializeOwned,
{
    let evidence = ResponseEvidence::from_response(&response);
    let body = response
        .bytes()
        .await
        .map_err(reqwest::Error::without_url)
        .with_context(|| {
            format!(
                "HackerWeb {target}: response body read failed ({}); retry or \
                 --api-backend firebase",
                evidence.describe(None),
            )
        })?;

    serde_json::from_slice(&body).map_err(|error| {
        let (problem, position) = match error.classify() {
            Category::Eof => ("incomplete JSON", "EOF"),
            Category::Data => ("incompatible JSON", "schema mismatch"),
            Category::Syntax | Category::Io => ("invalid JSON", "parse error"),
        };
        anyhow::anyhow!(
            "HackerWeb {target}: {problem} ({}; {position} {}:{}); retry or \
             --api-backend firebase",
            evidence.describe(Some(body.len())),
            error.line(),
            error.column(),
        )
    })
}

struct ResponseEvidence {
    status: StatusCode,
    content_length: Option<u64>,
    content_type: &'static str,
}

impl ResponseEvidence {
    fn from_response(response: &Response) -> Self {
        let headers = response.headers();
        Self {
            status: response.status(),
            content_length: response.content_length(),
            content_type: classify_content_type(headers.get(CONTENT_TYPE)),
        }
    }

    fn describe(&self, bytes: Option<usize>) -> String {
        let mut details = format!("HTTP {}", self.status);
        match (bytes, self.content_length) {
            (Some(received), Some(expected)) if received as u64 != expected => {
                details.push_str(&format!(", {received} B, expected {expected} B"));
            }
            (Some(received), _) => details.push_str(&format!(", {received} B")),
            (None, Some(expected)) => details.push_str(&format!(", expected {expected} B")),
            (None, None) => {}
        }
        if self.content_type != "json" {
            details.push_str(&format!(", content-type {}", self.content_type));
        }
        details
    }
}

fn classify_content_type(value: Option<&reqwest::header::HeaderValue>) -> &'static str {
    let Some(value) = value else {
        return "missing";
    };
    let Ok(value) = value.to_str() else {
        return "invalid";
    };
    let media_type = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if media_type == "application/json" || media_type.ends_with("+json") {
        "json"
    } else {
        "non-json"
    }
}
