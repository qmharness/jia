use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use futures::stream::Stream;
use tokio_util::sync::CancellationToken;

use std::task::{Context, Poll};

use super::AppState;

/// In-memory token bucket rate limiter per client IP.
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, TokenBucket>>,
    rate: u32,
    capacity: u32,
    check_count: Mutex<u64>,
    max_buckets: usize,
}

pub struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_minute: u32) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            rate: requests_per_minute,
            capacity: requests_per_minute,
            check_count: Mutex::new(0),
            max_buckets: 10_000,
        }
    }

    /// Returns `true` if the request is allowed, `false` if rate limited.
    /// Performs lazy cleanup of stale entries every 100 checks.
    pub fn check(&self, ip: &str) -> bool {
        if self.rate == 0 {
            return true;
        }
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());

        // Lazy cleanup every 100 checks
        {
            let mut count = self.check_count.lock().unwrap_or_else(|e| e.into_inner());
            *count += 1;
            if (*count).is_multiple_of(100) {
                buckets.retain(|_, b| now.duration_since(b.last_refill) < Duration::from_secs(300));
            }
        }

        // Enforce max bucket count to prevent memory exhaustion DoS
        if buckets.len() >= self.max_buckets && !buckets.contains_key(ip) {
            // Evict the oldest entry (by last_refill) to make room
            if let Some(oldest_key) = buckets
                .iter()
                .min_by_key(|(_, b)| b.last_refill)
                .map(|(k, _)| k.clone())
            {
                buckets.remove(&oldest_key);
            }
        }

        let bucket = buckets
            .entry(ip.to_string())
            .or_insert_with(|| TokenBucket {
                tokens: self.capacity as f64,
                last_refill: now,
            });
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        let refill = elapsed * (self.rate as f64 / 60.0);
        bucket.tokens = (bucket.tokens + refill).min(self.capacity as f64);
        bucket.last_refill = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Constant-time byte comparison to prevent timing side-channel attacks.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Auth middleware — validates Bearer token when api_key is configured.
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    // Serve landing page and static assets without auth so the browser
    // can load the page that contains the injected __JIA_TOKEN__.
    let path = request.uri().path();
    let is_root_asset = path == "/favicon.svg" || path == "/icons.svg";
    if path == "/"
        || path.starts_with("/static/")
        || path.starts_with("/assets/")
        || path == "/auth/session"
        || path == "/health"
        || is_root_asset
    {
        return next.run(request).await;
    }

    match &state.api_key {
        None => {
            // No API key configured: allow loopback, reject remote by default.
            // Set security.allow_unauthenticated = true to override.
            let addr = request
                .extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|ci| ci.0.ip())
                .unwrap_or_else(|| std::net::IpAddr::from([0, 0, 0, 0]));
            if addr.is_loopback() {
                return next.run(request).await;
            }
            tracing::warn!(remote = %addr, "Rejected unauthenticated remote request (no api_key configured)");
            (axum::http::StatusCode::UNAUTHORIZED, "API key required for remote access").into_response()
        }
        Some(expected_key) => {
            let authorized = request
                .headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|token| constant_time_eq(token.as_bytes(), expected_key.as_bytes()))
                .unwrap_or(false);
            if authorized {
                next.run(request).await
            } else {
                StatusCode::UNAUTHORIZED.into_response()
            }
        }
    }
}

/// Rate limit middleware — token-bucket per client IP, applied only to /agent.
pub async fn rate_limit_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    // Prefer the connection's peer socket address (spoof-proof).
    // Fall back to X-Forwarded-For / X-Real-IP when behind a reverse proxy
    // that sets these headers (localhost dev, or production with trusted proxy).
    let ip = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .filter(|addr| addr != "127.0.0.1" && addr != "::1")
        .or_else(|| {
            request
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
                .map(|s| s.trim().to_string())
        })
        .or_else(|| {
            request
                .headers()
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "127.0.0.1".to_string());

    if state.rate_limiter.check(&ip) {
        next.run(request).await
    } else {
        let body = serde_json::json!({"error": "rate limit exceeded", "retry_after_seconds": 60});
        let mut resp = Response::new(axum::body::Body::from(body.to_string()));
        *resp.status_mut() = StatusCode::TOO_MANY_REQUESTS;
        resp.headers_mut().insert(
            axum::http::header::RETRY_AFTER,
            axum::http::HeaderValue::from_static("60"),
        );
        resp.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        resp
    }
}

/// Stream wrapper that cancels a `CancellationToken` when the stream is dropped,
/// signalling that the client has disconnected from the SSE stream.
pub struct CancelOnDropStream<S> {
    pub inner: S,
    pub token: CancellationToken,
}

impl<S: Stream + Unpin> Stream for CancelOnDropStream<S> {
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<S> Drop for CancelOnDropStream<S> {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_equal() {
        assert!(constant_time_eq(b"secret", b"secret"));
    }

    #[test]
    fn constant_time_eq_different() {
        assert!(!constant_time_eq(b"secret", b"wrong!"));
    }

    #[test]
    fn constant_time_eq_length_mismatch() {
        assert!(!constant_time_eq(b"short", b"longer_secret"));
    }

    #[test]
    fn constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }

    #[tokio::test]
    async fn rate_limiter_allows_within_limit() {
        let limiter = RateLimiter::new(60); // 60 requests/min = 1/sec
        assert!(limiter.check("127.0.0.1"));
    }

    #[tokio::test]
    async fn rate_limiter_allows_when_disabled() {
        let limiter = RateLimiter::new(0); // 0 = disabled
        assert!(limiter.check("127.0.0.1"));
        assert!(limiter.check("127.0.0.1"));
    }

    #[tokio::test]
    async fn rate_limiter_different_ips_independent() {
        let limiter = RateLimiter::new(60);
        assert!(limiter.check("10.0.0.1"));
        assert!(limiter.check("10.0.0.2"));
    }

    #[test]
    fn cancel_on_drop_stream_cancels_token() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        {
            let _stream = CancelOnDropStream {
                inner: futures::stream::empty::<()>(),
                token: token.clone(),
            };
        }
        // Token is cancelled on drop
        assert!(token.is_cancelled());
    }

    // ── Auth middleware tests ────────────────────────────────────

    // ── Auth token extraction ────────────────────────────────────

    #[test]
    fn constant_time_eq_rejects_different_lengths() {
        // Verify constant-time comparison is timing-safe for auth
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"wrong!"));
        assert!(!constant_time_eq(b"short", b"longer_secret"));
    }
}
