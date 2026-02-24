use crate::app::AppEvent;
use crate::plugin::PluginEvent;
use anyhow::{anyhow, Result};
use tokio::sync::mpsc;

pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

/// Stream an OpenAI-compatible chat completion, sending each token as a `PluginEvent`.
pub async fn stream_chat_completion(
    http: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    messages: Vec<ChatMessage>,
    tx: mpsc::UnboundedSender<AppEvent>,
) {
    let result = stream_inner(http, api_url, api_key, model, messages, &tx).await;
    match result {
        Ok(()) => {
            let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeComplete));
        }
        Err(e) => {
            let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeError {
                message: format!("{e:#}"),
            }));
        }
    }
}

async fn stream_inner(
    http: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    messages: Vec<ChatMessage>,
    tx: &mpsc::UnboundedSender<AppEvent>,
) -> Result<()> {
    let msg_json: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content,
            })
        })
        .collect();

    let body = serde_json::json!({
        "model": model,
        "messages": msg_json,
        "stream": true,
    });

    let mut resp = http
        .post(api_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let preview = if text.len() > 200 {
            format!("{}...", &text[..200])
        } else {
            text
        };
        return Err(anyhow!("LLM API {status}: {preview}"));
    }

    let mut line_buf = String::new();
    while let Some(chunk) = resp.chunk().await? {
        let text = String::from_utf8_lossy(&chunk);
        for ch in text.chars() {
            if ch == '\n' {
                process_sse_line(&line_buf, tx)?;
                line_buf.clear();
            } else {
                line_buf.push(ch);
            }
        }
    }
    // Process any trailing content
    if !line_buf.trim().is_empty() {
        process_sse_line(&line_buf, tx)?;
    }

    Ok(())
}

/// Returns `Err` with a sentinel to signal `[DONE]`.
fn process_sse_line(
    line: &str,
    tx: &mpsc::UnboundedSender<AppEvent>,
) -> Result<()> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(());
    }
    let Some(data) = line.strip_prefix("data: ") else {
        return Ok(());
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(());
    }

    let json: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Ok(()), // skip malformed lines
    };

    if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
        if !content.is_empty() {
            let _ = tx.send(AppEvent::PluginEvent(PluginEvent::SummarizeChunk {
                delta: content.to_string(),
            }));
        }
    }

    Ok(())
}
