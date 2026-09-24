//! What a passalong-server answers when it refuses, and what this client
//! does about it.
//!
//! Errors are `application/problem+json` with a stable `code`; the client
//! decides from the code alone. A single command repeats only what the
//! server marks retryable, at most three attempts, one, two, then four
//! seconds apart. `RATE_LIMITED` waits for `Retry-After` when that is a
//! minute or less. A 401 is never repeated: an address that sends too many
//! bad keys is locked out, and every device behind it with it.

use std::time::Duration;

use serde::Deserialize;

/// The most attempts a single request gets.
pub const ATTEMPTS: u32 = 3;
/// The longest `Retry-After` a command waits out instead of failing.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// A problem's stable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Code {
    /// No key, or an unknown one.
    Unauthenticated,
    /// The API key has expired.
    KeyExpired,
    /// The API key was revoked.
    KeyRevoked,
    /// A read-only key tried to write.
    ForbiddenRole,
    /// The workspace's data key changed.
    KeyIdMismatch,
    /// The workspace's encryption is being changed.
    RewriteInProgress,
    /// Another key holds the rewrite session.
    LeaseHeld,
    /// `commitRewrite` before every item was staged.
    RewriteIncomplete,
    /// A rewrite with this new key id was aborted.
    RewriteEnded,
    /// The workspace is full.
    QuotaExceeded,
    /// The item is larger than the server takes.
    ItemTooLarge,
    /// The content does not match what was announced.
    ContentMismatch,
    /// As named.
    NotFound,
    /// A malformed id.
    InvalidId,
    /// A malformed request.
    InvalidRequest,
    /// Too many failed authentications from this address.
    RateLimited,
    /// The server is failing closed.
    ServiceUnavailable,
    /// A code this client does not know.
    Unknown,
}

impl Code {
    /// The code a server sent.
    pub fn parse(code: &str) -> Self {
        match code {
            "UNAUTHENTICATED" => Self::Unauthenticated,
            "KEY_EXPIRED" => Self::KeyExpired,
            "KEY_REVOKED" => Self::KeyRevoked,
            "FORBIDDEN_ROLE" => Self::ForbiddenRole,
            "KEY_ID_MISMATCH" => Self::KeyIdMismatch,
            "REWRITE_IN_PROGRESS" => Self::RewriteInProgress,
            "LEASE_HELD" => Self::LeaseHeld,
            "REWRITE_INCOMPLETE" => Self::RewriteIncomplete,
            "REWRITE_ENDED" => Self::RewriteEnded,
            "QUOTA_EXCEEDED" => Self::QuotaExceeded,
            "ITEM_TOO_LARGE" => Self::ItemTooLarge,
            "CONTENT_MISMATCH" => Self::ContentMismatch,
            "NOT_FOUND" => Self::NotFound,
            "INVALID_ID" => Self::InvalidId,
            "INVALID_REQUEST" => Self::InvalidRequest,
            "RATE_LIMITED" => Self::RateLimited,
            "SERVICE_UNAVAILABLE" => Self::ServiceUnavailable,
            _ => Self::Unknown,
        }
    }
}

/// An `application/problem+json` answer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// The stable code, as sent.
    pub code: String,
    /// The HTTP status.
    pub status: u16,
    /// A short description.
    pub title: String,
    /// More about this occurrence.
    #[serde(default)]
    pub detail: Option<String>,
    /// Whether repeating the request later may succeed.
    #[serde(default)]
    pub retryable: bool,
    /// For `LEASE_HELD` and `REWRITE_IN_PROGRESS`: when the lease ends.
    #[serde(default)]
    pub lease_expires_at: Option<String>,
}

impl Problem {
    /// The code, parsed.
    pub fn code(&self) -> Code {
        Code::parse(&self.code)
    }
}

/// Why a request to a server failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HttpsError {
    /// The server refused, saying why.
    #[error("the server refused ({}): {}", .problem.code, describe(.problem))]
    Refused {
        /// What it said.
        problem: Problem,
        /// Its `Retry-After`, when it sent one.
        retry_after: Option<Duration>,
    },
    /// The server could not be reached, or the connection broke.
    #[error("cannot reach {url}: {reason}")]
    Transport {
        /// The server.
        url: String,
        /// What happened, including a TLS pin mismatch.
        reason: String,
        /// Whether trying again may help.
        retryable: bool,
    },
    /// The server answered something this client cannot read.
    #[error("unexpected answer from the server: {0}")]
    Protocol(String),
}

impl HttpsError {
    /// The problem's code, when the server refused.
    pub fn code(&self) -> Option<Code> {
        match self {
            Self::Refused { problem, .. } => Some(problem.code()),
            _ => None,
        }
    }
}

/// The problem in words, with when a lease ends where it says so.
fn describe(problem: &Problem) -> String {
    let mut text = problem
        .detail
        .clone()
        .unwrap_or_else(|| problem.title.clone());
    if let Some(until) = &problem.lease_expires_at {
        text.push_str(&format!(" (until {until})"));
    }
    text
}

/// What to do after a failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Give up and report the error.
    Stop,
    /// Try again after this long.
    RetryAfter(Duration),
}

/// Whether to repeat a request after `attempts` attempts ended with `err`.
pub fn next(err: &HttpsError, attempts: u32) -> Next {
    if attempts >= ATTEMPTS {
        return Next::Stop;
    }
    let backoff = Duration::from_secs(1 << (attempts.max(1) - 1));
    match err {
        HttpsError::Refused {
            problem,
            retry_after,
        } => match problem.code() {
            Code::RateLimited => match retry_after {
                Some(wait) if *wait <= MAX_RETRY_AFTER => Next::RetryAfter(*wait),
                Some(_) => Next::Stop,
                None => Next::RetryAfter(backoff),
            },
            _ if problem.status == 401 => Next::Stop,
            _ if problem.retryable => Next::RetryAfter(backoff),
            _ => Next::Stop,
        },
        HttpsError::Transport { retryable, .. } if *retryable => Next::RetryAfter(backoff),
        _ => Next::Stop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODES: [&str; 17] = [
        "CONTENT_MISMATCH",
        "FORBIDDEN_ROLE",
        "INVALID_ID",
        "INVALID_REQUEST",
        "ITEM_TOO_LARGE",
        "KEY_EXPIRED",
        "KEY_ID_MISMATCH",
        "KEY_REVOKED",
        "LEASE_HELD",
        "NOT_FOUND",
        "QUOTA_EXCEEDED",
        "RATE_LIMITED",
        "REWRITE_ENDED",
        "REWRITE_INCOMPLETE",
        "REWRITE_IN_PROGRESS",
        "SERVICE_UNAVAILABLE",
        "UNAUTHENTICATED",
    ];

    fn refused(code: &str, status: u16, retryable: bool) -> HttpsError {
        HttpsError::Refused {
            problem: Problem {
                code: code.to_owned(),
                status,
                title: "t".to_owned(),
                detail: None,
                retryable,
                lease_expires_at: None,
            },
            retry_after: None,
        }
    }

    #[test]
    fn every_documented_code_is_known_and_a_new_one_is_not_an_error() {
        for code in CODES {
            assert_ne!(Code::parse(code), Code::Unknown, "{code}");
        }
        assert_eq!(Code::parse("SOMETHING_NEW"), Code::Unknown);
    }

    #[test]
    fn a_problem_parses_with_or_without_its_optional_fields() {
        let full: Problem = serde_json::from_str(
            r#"{"code":"LEASE_HELD","status":409,"title":"held","detail":"another key holds it","retryable":true,"leaseExpiresAt":"2026-09-19T16:00:00Z","extra":1}"#,
        )
        .unwrap();
        assert_eq!(full.code(), Code::LeaseHeld);
        let err = HttpsError::Refused {
            problem: full,
            retry_after: None,
        };
        assert_eq!(
            err.to_string(),
            "the server refused (LEASE_HELD): another key holds it (until 2026-09-19T16:00:00Z)"
        );
        let bare: Problem =
            serde_json::from_str(r#"{"code":"NOT_FOUND","status":404,"title":"no such item"}"#)
                .unwrap();
        assert!(!bare.retryable);
        assert_eq!(
            HttpsError::Refused {
                problem: bare,
                retry_after: None
            }
            .to_string(),
            "the server refused (NOT_FOUND): no such item"
        );
    }

    #[test]
    fn retries_follow_the_server_and_never_repeat_a_401() {
        assert_eq!(
            next(&refused("SERVICE_UNAVAILABLE", 503, true), 1),
            Next::RetryAfter(Duration::from_secs(1))
        );
        assert_eq!(
            next(&refused("REWRITE_IN_PROGRESS", 409, true), 2),
            Next::RetryAfter(Duration::from_secs(2))
        );
        assert_eq!(
            next(&refused("SERVICE_UNAVAILABLE", 503, true), 3),
            Next::Stop
        );
        for (code, status) in [
            ("UNAUTHENTICATED", 401),
            ("KEY_EXPIRED", 401),
            ("KEY_REVOKED", 401),
        ] {
            // Even if a server marked it retryable.
            assert_eq!(next(&refused(code, status, true), 1), Next::Stop, "{code}");
        }
        assert_eq!(next(&refused("NOT_FOUND", 404, false), 1), Next::Stop);
        assert_eq!(next(&refused("QUOTA_EXCEEDED", 413, false), 1), Next::Stop);
    }

    #[test]
    fn a_rate_limit_waits_out_a_short_retry_after_only() {
        let limited = |after: Option<u64>| HttpsError::Refused {
            problem: Problem {
                code: "RATE_LIMITED".to_owned(),
                status: 429,
                title: "slow down".to_owned(),
                detail: None,
                retryable: true,
                lease_expires_at: None,
            },
            retry_after: after.map(Duration::from_secs),
        };
        assert_eq!(
            next(&limited(Some(30)), 1),
            Next::RetryAfter(Duration::from_secs(30))
        );
        assert_eq!(next(&limited(Some(600)), 1), Next::Stop);
        assert_eq!(
            next(&limited(None), 1),
            Next::RetryAfter(Duration::from_secs(1))
        );
    }

    #[test]
    fn broken_connections_are_retried_and_protocol_errors_are_not() {
        let transport = |retryable| HttpsError::Transport {
            url: "https://x".to_owned(),
            reason: "reset".to_owned(),
            retryable,
        };
        assert_eq!(
            next(&transport(true), 1),
            Next::RetryAfter(Duration::from_secs(1))
        );
        assert_eq!(next(&transport(false), 1), Next::Stop);
        assert_eq!(
            next(&HttpsError::Protocol("garbage".to_owned()), 1),
            Next::Stop
        );
    }
}
