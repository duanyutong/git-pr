use std::{
    fmt::{self, Debug},
    io::Read as _,
    time::Duration,
};

use anyhow::{Context, Result, bail};

const LIMIT: u64 = 64 * 1024 * 1024;

pub(crate) struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub token: &'a str,
    pub body: Option<&'a [u8]>,
    pub timeout: i64,
}

impl Debug for Request<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("token", &"[redacted]")
            .field("body", &self.body)
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct HttpStatusError {
    pub url: String,
    pub status: reqwest::StatusCode,
    pub body: String,
    pub quota: crate::github::limits::Quota,
}

impl fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut characters = self.body.chars();
        let preview: String = characters.by_ref().take(2048).collect();
        write!(f, "HTTP {} from {}: {}", self.status, self.url, preview)?;
        if characters.next().is_some() {
            write!(f, " [response diagnostic truncated]")?;
        }
        let hint = match self.status.as_u16() {
            401 => {
                "verify GitHub authentication with `gh auth status` and sign in again if necessary"
            }
            403 => {
                "check repository permissions, organisation SSO authorisation and GitHub rate limits"
            }
            404 => {
                "check the repository or resource name, access permissions and server feature availability"
            }
            409 | 422 => "check the request details and current branch or PR state before retrying",
            429 => "wait for the GitHub rate limit to reset before retrying",
            500..=599 => {
                "check GitHub service availability; verify whether any requested mutation completed before retrying"
            }
            _ => "check the server response and the requested operation",
        };
        write!(f, "\nHint: {hint}")
    }
}
impl std::error::Error for HttpStatusError {}

#[derive(Debug)]
pub(crate) struct TransientRead;
impl fmt::Display for TransientRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("temporary connection or response-read failure")
    }
}
impl std::error::Error for TransientRead {}

#[derive(Debug)]
pub(crate) struct Response {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

pub(crate) trait Transport: Debug + Send + Sync {
    fn response(&self, request: Request<'_>) -> Result<Response> {
        let body = self.request(request)?;
        Ok(Response {
            status: if body.is_empty() { 204 } else { 200 },
            headers: reqwest::header::HeaderMap::new(),
            body,
        })
    }

    fn request(&self, request: Request<'_>) -> Result<Vec<u8>>;
}

#[derive(Debug, Default)]
pub(crate) struct HttpTransport {
    client: std::sync::OnceLock<std::result::Result<reqwest::blocking::Client, reqwest::Error>>,
}

impl Transport for HttpTransport {
    fn request(&self, request: Request<'_>) -> Result<Vec<u8>> {
        self.response(request).map(|r| r.body)
    }
    fn response(&self, request: Request<'_>) -> Result<Response> {
        if request.timeout <= 0 {
            bail!(
                "context deadline exceeded: HTTP timeout is zero or negative; set --timeout to a positive number of seconds"
            );
        }
        let timeout = Duration::from_secs(request.timeout.unsigned_abs());
        let mut builder = self
            .client
            .get_or_init(|| reqwest::blocking::Client::builder().build())
            .as_ref()
            .map_err(|error| anyhow::anyhow!("could not initialise HTTP transport: {error}"))?
            .request(
                request
                    .method
                    .parse()
                    .context("invalid HTTP request method")?,
                request.url,
            )
            .timeout(timeout)
            .header("Authorization", format!("Bearer {}", request.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", concat!("git-pr/", env!("CARGO_PKG_VERSION")))
            .header("X-GitHub-Api-Version", "2026-03-10")
            .header("Content-Type", "application/json");
        if let Some(body) = request.body {
            builder = builder.body(body.to_vec());
        }
        let response = builder.send().map_err(|error| {
            let retryable = error.is_connect() || error.is_timeout() || error.is_request();
            let error = anyhow::Error::new(error);
            if retryable { error.context(TransientRead) } else { error }
        }).context("HTTP transport failed; check network access, proxy/TLS configuration and the request timeout. A mutation may have completed even if its response was lost")?;
        let status = response.status();
        let headers = response.headers().clone();
        let mut body = Vec::new();
        response
            .take(LIMIT + 1)
            .read_to_end(&mut body)
            .context(TransientRead)
            .context(
                "could not read the complete HTTP response; the remote result is unconfirmed",
            )?;
        if body.len() as u64 > LIMIT {
            bail!("HTTP response exceeded 64 MiB; refusing an incomplete observation");
        }
        if !status.is_success() {
            return Err(HttpStatusError {
                url: request.url.into(),
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
                quota: quota(&headers),
            }
            .into());
        }
        Ok(Response {
            status: status.as_u16(),
            headers,
            body,
        })
    }
}

pub(crate) fn quota(headers: &reqwest::header::HeaderMap) -> crate::github::limits::Quota {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let number = |name| header(name).and_then(|value| value.parse::<u64>().ok());
    let timestamp = |value: &str| {
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()
            .and_then(|date| u64::try_from(date.timestamp()).ok())
    };
    // Server time avoids converting a clock-skewed reset into an early retry.
    let now = header("date").and_then(timestamp).unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    });
    crate::github::limits::Quota {
        remaining: number("x-ratelimit-remaining"),
        limit: number("x-ratelimit-limit"),
        reset: number("x-ratelimit-reset")
            .map(|time| Duration::from_secs(time.saturating_sub(now))),
        retry_after: header("retry-after")
            .and_then(|value| {
                value
                    .parse::<u64>()
                    .ok()
                    .or_else(|| timestamp(value).map(|time| time.saturating_sub(now)))
            })
            .map(Duration::from_secs),
    }
}

#[cfg(test)]
#[path = "../tests/http.rs"]
mod tests;
