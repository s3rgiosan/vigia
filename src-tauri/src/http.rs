//! Shared HTTP client: no redirects, request logging, rate limit headers, error classification.

use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, IF_NONE_MATCH, USER_AGENT};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::providers::ProviderError;

pub const USER_AGENT_VALUE: &str = concat!("vigia/", env!("CARGO_PKG_VERSION"));

/// Largest response body read, in bytes. A larger body fails as `Decode`.
pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Primary rate limit status read from response headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    pub limit: u64,
    pub remaining: u64,
    /// Unix time when the window resets.
    pub reset_at: i64,
}

/// A response the caller still has to interpret: 200 with a body, or 304.
#[derive(Debug)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn not_modified(&self) -> bool {
        self.status == StatusCode::NOT_MODIFIED
    }

    pub fn etag(&self) -> Option<String> {
        header_str(&self.headers, "etag").map(str::to_string)
    }

    /// Parses `x-ratelimit-*` (GitHub) or `RateLimit-*` (GitLab) headers.
    pub fn rate_limit(&self) -> Option<RateLimit> {
        parse_rate_limit(&self.headers)
    }

    /// The `rel="next"` URL from a `Link` header, only when it stays on `base`'s origin.
    /// A next page on another scheme, host or port is reported as a redirect.
    pub fn next_link_on(&self, base: &Url) -> Result<Option<Url>, ProviderError> {
        let Some(next) = self.next_link() else {
            return Ok(None);
        };
        let url = Url::parse(&next).map_err(|e| ProviderError::Decode(e.to_string()))?;
        let same = url.scheme() == base.scheme()
            && url.host_str() == base.host_str()
            && url.port_or_known_default() == base.port_or_known_default();
        if same {
            Ok(Some(url))
        } else {
            Err(ProviderError::Redirected { location: next })
        }
    }

    /// The `rel="next"` URL from a `Link` header.
    pub fn next_link(&self) -> Option<String> {
        let link = header_str(&self.headers, "link")?;
        link.split(',').find_map(|part| {
            let (url, rel) = part.split_once(';')?;
            if rel.trim() == "rel=\"next\"" {
                Some(
                    url.trim()
                        .trim_matches(|c| c == '<' || c == '>')
                        .to_string(),
                )
            } else {
                None
            }
        })
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, ProviderError> {
        serde_json::from_slice(&self.body).map_err(|e| ProviderError::Decode(e.to_string()))
    }
}

/// One client per account, holding the token.
#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    auth: HeaderValue,
}

impl HttpClient {
    pub fn new(token: &str) -> Result<HttpClient, ProviderError> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| ProviderError::Unauthorized)?;
        auth.set_sensitive(true);
        Ok(HttpClient { client, auth })
    }

    /// Sends a GET and classifies anything that is not 200 or 304 into a `ProviderError`.
    pub async fn get(
        &self,
        url: &Url,
        extra_headers: &HeaderMap,
        etag: Option<&str>,
    ) -> Result<HttpResponse, ProviderError> {
        let mut headers = extra_headers.clone();
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
        headers.insert(AUTHORIZATION, self.auth.clone());
        headers
            .entry(ACCEPT)
            .or_insert(HeaderValue::from_static("application/json"));
        if let Some(etag) = etag {
            if let Ok(value) = HeaderValue::from_str(etag) {
                headers.insert(IF_NONE_MATCH, value);
            }
        }

        let started = Instant::now();
        let result = self.client.get(url.clone()).headers(headers).send().await;
        let logged_url = url_without_query(url);

        let response = match result {
            Ok(r) => r,
            Err(e) => {
                let message = e.without_url().to_string();
                log::warn!("GET {logged_url} failed: {message}");
                return Err(ProviderError::Network(message));
            }
        };

        let status = response.status();
        let headers = response.headers().clone();
        let declared = header_u64(&headers, "content-length").unwrap_or(0);
        if declared > MAX_BODY_BYTES as u64 {
            log::warn!("GET {logged_url} declared a {declared} byte body; not reading it");
            return Err(body_too_large());
        }
        let body = read_capped(response).await.inspect_err(|e| {
            log::warn!("GET {logged_url} body not read: {e}");
        })?;
        log::debug!(
            "GET {logged_url} -> {} in {} ms",
            status.as_u16(),
            started.elapsed().as_millis()
        );

        classify(status, &headers, &body)?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// Maps a non-success status to an error. 200 and 304 pass through.
pub fn classify(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> Result<(), ProviderError> {
    if status == StatusCode::OK || status == StatusCode::NOT_MODIFIED {
        return Ok(());
    }
    if status.is_redirection() {
        let location = header_str(headers, "location").unwrap_or("").to_string();
        return Err(ProviderError::Redirected { location });
    }
    match status {
        StatusCode::UNAUTHORIZED => Err(ProviderError::Unauthorized),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
            let retry_after = header_str(headers, "retry-after").and_then(|v| v.parse().ok());
            let rate_limit = parse_rate_limit(headers);
            let exhausted = rate_limit.is_some_and(|r| r.remaining == 0);
            let secondary = body_mentions_secondary_limit(body);
            let is_rate_limit = status == StatusCode::TOO_MANY_REQUESTS
                || retry_after.is_some()
                || exhausted
                || secondary;
            if is_rate_limit {
                Err(ProviderError::RateLimited {
                    retry_after,
                    reset_at: rate_limit.map(|r| r.reset_at),
                    remaining: rate_limit.map(|r| r.remaining),
                })
            } else {
                Err(ProviderError::Forbidden)
            }
        }
        StatusCode::NOT_FOUND => Err(ProviderError::NotFound),
        s if s.is_server_error() => Err(ProviderError::Server { status: s.as_u16() }),
        s => Err(ProviderError::Decode(format!(
            "unexpected status {}",
            s.as_u16()
        ))),
    }
}

/// Reads the body chunk by chunk and stops once it passes `MAX_BODY_BYTES`.
async fn read_capped(mut response: reqwest::Response) -> Result<Vec<u8>, ProviderError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ProviderError::Network(e.without_url().to_string()))?
    {
        let total = body.len() + chunk.len();
        if total > MAX_BODY_BYTES {
            return Err(body_too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn body_too_large() -> ProviderError {
    ProviderError::Decode(format!(
        "the response is larger than {} MB",
        MAX_BODY_BYTES / (1024 * 1024)
    ))
}

fn body_mentions_secondary_limit(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    text.contains("secondary rate limit")
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    header_str(headers, name).and_then(|v| v.trim().parse().ok())
}

fn parse_rate_limit(headers: &HeaderMap) -> Option<RateLimit> {
    let (limit, remaining, reset) = (
        "x-ratelimit-limit",
        "x-ratelimit-remaining",
        "x-ratelimit-reset",
    );
    let (limit, remaining, reset_at) = match header_u64(headers, limit) {
        Some(l) => (
            l,
            header_u64(headers, remaining)?,
            header_u64(headers, reset)? as i64,
        ),
        None => (
            header_u64(headers, "ratelimit-limit")?,
            header_u64(headers, "ratelimit-remaining")?,
            header_u64(headers, "ratelimit-reset")? as i64,
        ),
    };
    Some(RateLimit {
        limit,
        remaining,
        reset_at,
    })
}

fn url_without_query(url: &Url) -> String {
    let mut copy = url.clone();
    copy.set_query(None);
    copy.set_fragment(None);
    copy.to_string()
}
