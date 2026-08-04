//! ArticleFetcher — obtains the Article for a Story.
//!
//! Linked pages go through the `localwebrs` CLI as a subprocess; self-posts
//! resolve locally from the story body. See
//! `docs/adr/20260725-article-fetch-via-localwebrs-subprocess.md`.

use crate::api::Story;
use crate::text::hn_html_to_article_markdown;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use url::Url;

/// Cache TTL handed to localwebrs. Its `--cache` defaults to 0 (off), so this
/// must be passed explicitly for a `v` press to be free the second time.
const CACHE_TTL_SECS: u64 = 86_400;

/// Extraction tier. `smart` is also localwebrs's default, but the ADR pins it,
/// so pass it rather than inherit whatever the CLI defaults to next release.
const VISITOR_TIER: &str = "smart";

/// Ceiling on one fetch. The smart visitor may fall back to a headless
/// browser, which legitimately takes tens of seconds.
const FETCH_TIMEOUT: Duration = Duration::from_secs(90);

const INSTALL_HINT: &str =
    "install it with: cargo install --git https://github.com/rocrp/localwebrs";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    pub title: Option<String>,
    pub content: String,
    /// Final URL reported by localwebrs after redirects. Relative links in the
    /// extracted Markdown resolve against this URL, not the original Story URL.
    pub effective_url: Option<String>,
}

#[derive(Debug)]
pub enum ArticleError {
    /// The configured binary is not on PATH.
    BinaryMissing {
        bin: String,
    },
    /// Spawning or waiting on the child failed for any other reason.
    Spawn {
        bin: String,
        source: std::io::Error,
    },
    TimedOut {
        seconds: u64,
    },
    /// Non-zero exit; carries the child's stderr.
    Failed {
        message: String,
    },
    /// localwebrs ran but produced no usable text (paywall, captcha, PDF
    /// without pdfium, …). Never a crash — the JSON contract is informal.
    NoContent,
}

impl std::fmt::Display for ArticleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinaryMissing { bin } => {
                write!(formatter, "{bin} not found — {INSTALL_HINT}")
            }
            Self::Spawn { bin, source } => write!(formatter, "could not run {bin}: {source}"),
            Self::TimedOut { seconds } => write!(formatter, "timed out after {seconds}s"),
            Self::Failed { message } => write!(formatter, "{message}"),
            Self::NoContent => write!(formatter, "no readable content extracted"),
        }
    }
}

impl std::error::Error for ArticleError {}

/// The `to_dict()` shape localwebrs prints under `--json`. Parsed tolerantly:
/// unknown fields are ignored so an upstream addition is not a breakage.
#[derive(Debug, Deserialize)]
struct VisitOutput {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ArticleFetcher {
    bin: String,
    /// localwebrs writes a CWD-relative `cache/cache.sqlite`; running the child
    /// from hntui's cache dir keeps that under our data directory.
    working_dir: Option<PathBuf>,
    timeout: Duration,
}

impl ArticleFetcher {
    pub fn new(bin: String, working_dir: Option<PathBuf>) -> Self {
        Self {
            bin,
            working_dir,
            timeout: FETCH_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_timeout(bin: String, timeout: Duration) -> Self {
        Self {
            bin,
            working_dir: None,
            timeout,
        }
    }

    pub async fn fetch(&self, url: String) -> Result<Article, ArticleError> {
        let mut command = Command::new(&self.bin);
        command
            .arg("visit")
            .arg(&url)
            .arg("--json")
            .arg("-v")
            .arg(VISITOR_TIER)
            .arg("-c")
            .arg(CACHE_TTL_SECS.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = &self.working_dir {
            command.current_dir(dir);
        }

        let child = command.spawn().map_err(|source: std::io::Error| {
            if source.kind() == std::io::ErrorKind::NotFound {
                ArticleError::BinaryMissing {
                    bin: self.bin.clone(),
                }
            } else {
                ArticleError::Spawn {
                    bin: self.bin.clone(),
                    source,
                }
            }
        })?;

        // The child is killed on drop, so the timeout branch needs no cleanup.
        let output = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
            Err(_) => {
                return Err(ArticleError::TimedOut {
                    seconds: self.timeout.as_secs(),
                })
            }
            Ok(Err(source)) => {
                return Err(ArticleError::Spawn {
                    bin: self.bin.clone(),
                    source,
                })
            }
            Ok(Ok(output)) => output,
        };

        if !output.status.success() {
            return Err(ArticleError::Failed {
                message: exit_message(&output.status, &output.stderr),
            });
        }

        parse_visit_output(&output.stdout, &url)
    }
}

fn parse_visit_output(stdout: &[u8], requested_url: &str) -> Result<Article, ArticleError> {
    let visit: VisitOutput =
        serde_json::from_slice(stdout).map_err(|error| ArticleError::Failed {
            message: format!("could not read localwebrs output: {error}"),
        })?;

    let content = visit
        .content
        .map(|content| content.trim().to_string())
        .filter(|content| !content.is_empty())
        .ok_or(ArticleError::NoContent)?;
    let effective_url = normalize_effective_url(visit.url, requested_url)?;

    Ok(Article {
        title: visit
            .title
            .map(|title| title.trim().to_string())
            .filter(|title| !title.is_empty()),
        content,
        effective_url,
    })
}

fn normalize_effective_url(
    reported_url: Option<String>,
    requested_url: &str,
) -> Result<Option<String>, ArticleError> {
    let candidate = reported_url
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .or_else(|| {
            let requested_url = requested_url.trim();
            (!requested_url.is_empty()).then(|| requested_url.to_string())
        });
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let url = Url::parse(&candidate).map_err(|error| ArticleError::Failed {
        message: format!("localwebrs returned an invalid effective URL: {error}"),
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ArticleError::Failed {
            message:
                "localwebrs returned an invalid effective URL: expected credential-free HTTP(S)"
                    .to_string(),
        });
    }
    Ok(Some(url.into()))
}

fn exit_message(status: &std::process::ExitStatus, stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let detail = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim();
    if detail.is_empty() {
        format!("localwebrs exited with {status}")
    } else {
        detail.to_string()
    }
}

/// A self-post's Article is its own body — resolved locally, no subprocess.
/// Returns `None` when the story has no body to show.
pub fn self_post_article(story: &Story) -> Option<Article> {
    body_article(&story.title, story.text.as_deref())
}

/// Turn an HN self-post body into an Article. Separate from `self_post_article`
/// because the body sometimes arrives on a StoryThread rather than the Story.
pub fn body_article(title: &str, body: Option<&str>) -> Option<Article> {
    let content = hn_html_to_article_markdown(body?);
    (!content.trim().is_empty()).then(|| Article {
        title: Some(title.to_string()),
        content,
        effective_url: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn story_with_text(text: Option<&str>) -> Story {
        Story {
            id: 1,
            title: "Ask HN: anything?".to_string(),
            url: None,
            text: text.map(str::to_string),
            score: 1,
            by: "alice".to_string(),
            time: 1,
            comment_count: 0,
            kids: vec![],
        }
    }

    /// Write an executable stand-in for localwebrs, so the subprocess seam is
    /// exercised without reaching the network.
    fn fake_bin(dir: &std::path::Path, name: &str, script: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, script).expect("write fake localwebrs");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make fake localwebrs executable");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn json_with_title_and_content_becomes_an_article() {
        let article = parse_visit_output(
            br#"{"url":"https://example.com/","title":"Example","content":"body text",
                 "extra":{"sitename":"example.com"},"future_field":42}"#,
            "https://requested.example/",
        )
        .expect("well-formed visit output");

        assert_eq!(article.title.as_deref(), Some("Example"));
        assert_eq!(article.content, "body text");
        assert_eq!(
            article.effective_url.as_deref(),
            Some("https://example.com/")
        );
    }

    #[test]
    fn missing_or_blank_content_is_an_extraction_failure_not_a_panic() {
        for payload in [
            br#"{"title":"Example"}"#.as_slice(),
            br#"{"title":"Example","content":null}"#.as_slice(),
            br#"{"title":"Example","content":"   \n "}"#.as_slice(),
        ] {
            assert!(matches!(
                parse_visit_output(payload, "https://example.com"),
                Err(ArticleError::NoContent)
            ));
        }
    }

    #[test]
    fn garbage_output_reports_a_read_failure() {
        let error = parse_visit_output(b"not json at all", "https://example.com")
            .expect_err("garbage must not parse");

        assert!(
            matches!(&error, ArticleError::Failed { message } if message.contains("localwebrs output")),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_content_only_payload_still_yields_an_article() {
        let article = parse_visit_output(
            br#"{"url":"  ","content":"body"}"#,
            "https://requested.example/path",
        )
        .expect("title is optional upstream");

        assert_eq!(article.title, None);
        assert_eq!(article.content, "body");
        assert_eq!(
            article.effective_url.as_deref(),
            Some("https://requested.example/path")
        );
    }

    #[test]
    fn malformed_effective_url_is_a_clear_extraction_failure() {
        let error = parse_visit_output(
            br#"{"url":"not a URL","content":"body"}"#,
            "https://requested.example/path",
        )
        .expect_err("malformed final URL must fail at the extraction boundary");

        assert!(
            matches!(&error, ArticleError::Failed { message } if message.contains("effective URL")),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn a_missing_binary_maps_to_the_install_hint() {
        let fetcher = ArticleFetcher::new("hntui-localwebrs-does-not-exist".to_string(), None);

        let error = fetcher
            .fetch("https://example.com".to_string())
            .await
            .expect_err("missing binary must fail");

        assert!(matches!(error, ArticleError::BinaryMissing { .. }));
        assert!(
            error.to_string().contains("cargo install"),
            "install hint missing from: {error}"
        );
    }

    #[tokio::test]
    async fn a_nonzero_exit_surfaces_the_last_stderr_line() {
        let dir = tempfile::tempdir().expect("temp dir");
        let bin = fake_bin(
            dir.path(),
            "failing",
            "#!/bin/sh\necho 'noise' >&2\necho 'captcha blocked the request' >&2\nexit 3\n",
        );
        let fetcher = ArticleFetcher::new(bin, None);

        let error = fetcher
            .fetch("https://example.com".to_string())
            .await
            .expect_err("non-zero exit must fail");

        assert_eq!(error.to_string(), "captcha blocked the request");
    }

    #[tokio::test]
    async fn the_watchdog_bounds_a_hung_child() {
        let dir = tempfile::tempdir().expect("temp dir");
        let bin = fake_bin(dir.path(), "hanging", "#!/bin/sh\nsleep 600\n");
        let fetcher = ArticleFetcher::with_timeout(bin, Duration::from_millis(50));

        let error = fetcher
            .fetch("https://example.com".to_string())
            .await
            .expect_err("hung child must time out");

        assert!(
            matches!(error, ArticleError::TimedOut { .. }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn the_shipped_watchdog_matches_the_adr() {
        let fetcher = ArticleFetcher::new("localwebrs".to_string(), None);

        assert_eq!(fetcher.timeout, Duration::from_secs(90));
    }

    #[tokio::test]
    async fn a_successful_child_is_parsed_and_runs_in_the_cache_dir() {
        let dir = tempfile::tempdir().expect("temp dir");
        let bin = fake_bin(
            dir.path(),
            "succeeding",
            "#!/bin/sh\nprintf '{\"title\":\"T\",\"content\":\"%s\"}' \"$(pwd)\"\n",
        );
        let working_dir = dir.path().join("cache");
        std::fs::create_dir_all(&working_dir).expect("create working dir");
        let fetcher = ArticleFetcher::new(bin, Some(working_dir.clone()));

        let article = fetcher
            .fetch("https://example.com".to_string())
            .await
            .expect("fake visitor succeeds");

        assert_eq!(article.title.as_deref(), Some("T"));
        assert!(
            std::path::Path::new(&article.content).ends_with("cache"),
            "child ran in {} instead of the cache dir",
            article.content
        );
    }

    #[test]
    fn a_self_post_body_preserves_links_as_markdown() {
        let article = self_post_article(&story_with_text(Some(
            "<p>Hello&nbsp;world</p><p>second <a href=\"https:&#x2F;&#x2F;example.com&#x2F;docs\">link</a></p>",
        )))
        .expect("self-post has an article");

        assert_eq!(article.title.as_deref(), Some("Ask HN: anything?"));
        assert_eq!(
            article.content,
            "Hello world\n\nsecond [link](https://example.com/docs)"
        );
    }

    #[test]
    fn a_story_without_a_body_has_no_local_article() {
        assert_eq!(self_post_article(&story_with_text(None)), None);
        assert_eq!(self_post_article(&story_with_text(Some("  <p>  "))), None);
    }
}
