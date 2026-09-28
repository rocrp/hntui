//! The JSON error contract every subcommand shares, mirroring rcrs's clikit:
//! stderr holds `{"error":{"kind","message","retryable"}}`, stdout stays empty,
//! and the exit code says which kind of failure it was.

use crate::api::StoryLookupError;
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// HTTP-level or protocol failure talking to an HN backend.
    Service,
    /// The requested Story does not exist, or the id names a comment.
    NotFound,
    /// Bad command-line input, detected before calling any backend.
    Usage,
    /// Connection, DNS, TLS or timeout failure.
    Network,
    /// Local failure: env file, config, cache dir.
    Local,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::NotFound => "not_found",
            Self::Usage => "usage",
            Self::Network => "network",
            Self::Local => "local",
        }
    }

    pub fn exit_code(self) -> i32 {
        match self {
            Self::Service | Self::NotFound | Self::Local => 1,
            Self::Usage => 2,
            Self::Network => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub kind: Kind,
    pub message: String,
    pub retryable: bool,
}

pub type Result<T> = std::result::Result<T, CliError>;

impl CliError {
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retryable: kind == Kind::Network,
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(Kind::Usage, message)
    }

    pub fn local(message: impl Into<String>) -> Self {
        Self::new(Kind::Local, message)
    }

    /// Classify a failure from the Source by what caused it.
    pub fn from_source(error: &anyhow::Error) -> Self {
        let message = format!("{error:#}");
        for cause in error.chain() {
            if cause.downcast_ref::<StoryLookupError>().is_some() {
                return Self::new(Kind::NotFound, message);
            }
            if let Some(http) = cause.downcast_ref::<reqwest::Error>() {
                // "error sending request" (a dropped TLS session, say) is as
                // transient as a refused connection.
                if http.is_timeout() || http.is_connect() || http.is_request() || http.is_body() {
                    return Self::new(Kind::Network, message);
                }
                let server_side = http.status().is_some_and(|s| s.is_server_error());
                return Self {
                    retryable: server_side,
                    ..Self::new(Kind::Service, message)
                };
            }
        }
        Self::new(Kind::Service, message)
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({"error": {
            "kind": self.kind.as_str(),
            "message": self.message,
            "retryable": self.retryable,
        }})
    }
}

/// Print the JSON error to stderr and exit with its kind's code.
pub fn fail(error: &CliError) -> ! {
    eprintln!("{}", error.to_json());
    std::process::exit(error.kind.exit_code());
}
