//! An HTTP client pool for remote instances: requests go round-robin over
//! a direct connection or a set of proxies, each limited to a request rate,
//! and are retried with backoff.

use anyhow::{Context, anyhow};
use rand::RngExt;
use reqwest::{Client, Proxy, StatusCode, Url, header::HeaderMap, header::RETRY_AFTER};
use serde::de::DeserializeOwned;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{sync::Mutex, time::sleep};
use tracing::{info, warn};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ATTEMPTS: usize = 8;
const DEFAULT_REQUESTS_PER_SECOND: f64 = 2.0;
const MIN_RATE_LIMIT_DELAY: Duration = Duration::from_secs(1);
const MAX_RATE_LIMIT_DELAY: Duration = Duration::from_secs(120);

/// Cheap to clone: clones share the connections and rate limits.
#[derive(Clone)]
pub struct HttpPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    routes: Vec<Route>,
    next_route: AtomicUsize,
}

/// A direct connection or one proxy, with its own rate limit.
struct Route {
    client: Client,
    limiter: RateLimiter,
    /// For logs; never contains proxy credentials.
    label: String,
}

impl HttpPool {
    /// Connects directly when `proxies` is empty. `requests_per_second`
    /// applies to each proxy separately.
    pub fn new(
        proxies: &[String],
        requests_per_second: f64,
        user_agent: &'static str,
        max_idle_connections: usize,
    ) -> anyhow::Result<Self> {
        let requests_per_second = if requests_per_second.is_finite() && requests_per_second > 0.0 {
            requests_per_second
        } else {
            DEFAULT_REQUESTS_PER_SECOND
        };
        let min_interval = Duration::from_secs_f64(1.0 / requests_per_second);

        let proxies: Vec<&str> = proxies
            .iter()
            .map(|proxy| proxy.trim())
            .filter(|proxy| !proxy.is_empty())
            .collect();
        let routes = if proxies.is_empty() {
            vec![Route::new(
                None,
                min_interval,
                user_agent,
                max_idle_connections,
            )?]
        } else {
            proxies
                .into_iter()
                .map(|proxy| {
                    Route::new(Some(proxy), min_interval, user_agent, max_idle_connections)
                })
                .collect::<anyhow::Result<_>>()?
        };

        info!(
            routes = routes.len(),
            requests_per_second_per_route = requests_per_second,
            requests_per_second = requests_per_second * routes.len() as f64,
            "created HTTP pool"
        );

        Ok(Self {
            inner: Arc::new(PoolInner {
                routes,
                next_route: AtomicUsize::new(0),
            }),
        })
    }

    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> anyhow::Result<T> {
        let body = self.get_bytes(url).await?;
        serde_json::from_slice(&body).with_context(|| {
            format!(
                "could not decode JSON ({} bytes), starting with: {:.200}",
                body.len(),
                String::from_utf8_lossy(&body)
            )
        })
    }

    /// Retries connection failures, server errors and rate limits (429, 503,
    /// 408); other error statuses fail at once.
    pub async fn get_bytes(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let mut last_error = None;

        for attempt in 0..MAX_ATTEMPTS {
            let route = self.next_route();
            route.limiter.acquire().await;
            let is_last_attempt = attempt + 1 == MAX_ATTEMPTS;
            let attempt_label = format!("attempt {}/{MAX_ATTEMPTS}", attempt + 1);

            let response = match route.client.get(url).timeout(REQUEST_TIMEOUT).send().await {
                Ok(response) => response,
                Err(error) => {
                    last_error = Some(anyhow!(
                        "request via {} failed ({attempt_label}): {error}",
                        route.label
                    ));
                    if !is_last_attempt {
                        route.back_off(error_backoff(attempt)).await;
                    }
                    continue;
                }
            };

            let status = response.status();
            if is_rate_limit(status) {
                let delay = rate_limit_delay(attempt, retry_after(response.headers()));
                warn!(
                    route = %route.label,
                    status = status.as_u16(),
                    attempt = attempt + 1,
                    max_attempts = MAX_ATTEMPTS,
                    retry_in_ms = delay.as_millis() as u64,
                    "rate limited, backing off"
                );
                route.back_off(delay).await;
                last_error = Some(anyhow!(
                    "HTTP {} via {} ({attempt_label})",
                    status.as_u16(),
                    route.label
                ));
                continue;
            }

            let body = response.bytes().await?;
            if status.is_success() {
                return Ok(body.to_vec());
            }

            last_error = Some(anyhow!(
                "HTTP {status} via {} ({attempt_label}), body starts with: {:.200}",
                route.label,
                String::from_utf8_lossy(&body)
            ));
            if !status.is_server_error() || is_last_attempt {
                break;
            }
            route.back_off(error_backoff(attempt)).await;
        }

        Err(last_error.unwrap_or_else(|| anyhow!("no attempts were made")))
    }

    fn next_route(&self) -> &Route {
        let routes = &self.inner.routes;
        let index = self.inner.next_route.fetch_add(1, Ordering::Relaxed) % routes.len();
        &routes[index]
    }
}

impl Route {
    fn new(
        proxy: Option<&str>,
        min_interval: Duration,
        user_agent: &'static str,
        max_idle_connections: usize,
    ) -> anyhow::Result<Self> {
        let mut builder = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .pool_max_idle_per_host(max_idle_connections.max(1))
            .user_agent(user_agent);
        if let Some(proxy) = proxy {
            builder = builder.proxy(Proxy::all(proxy)?);
        }

        Ok(Self {
            client: builder.build()?,
            limiter: RateLimiter::new(min_interval),
            label: proxy.map_or_else(|| "direct".to_owned(), proxy_label),
        })
    }

    /// Keeps the route idle for `delay`, including for other requests.
    async fn back_off(&self, delay: Duration) {
        self.limiter.delay_next(delay).await;
        sleep(delay).await;
    }
}

/// Spaces requests at least `min_interval` apart.
struct RateLimiter {
    min_interval: Duration,
    next_free: Mutex<Instant>,
}

impl RateLimiter {
    fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            next_free: Mutex::new(Instant::now()),
        }
    }

    async fn acquire(&self) {
        let wait = {
            let mut next_free = self.next_free.lock().await;
            let now = Instant::now();
            let start = (*next_free).max(now);
            *next_free = start + self.min_interval;
            start - now
        };
        if !wait.is_zero() {
            sleep(wait).await;
        }
    }

    async fn delay_next(&self, delay: Duration) {
        let mut next_free = self.next_free.lock().await;
        *next_free = (*next_free).max(Instant::now() + delay);
    }
}

/// The proxy's host and port, without credentials.
fn proxy_label(proxy: &str) -> String {
    let Ok(url) = Url::parse(proxy) else {
        return "proxy".to_owned();
    };
    let host = url.host_str().unwrap_or("proxy");
    match url.port_or_known_default() {
        Some(port) => format!("proxy://{host}:{port}"),
        None => format!("proxy://{host}"),
    }
}

fn is_rate_limit(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::REQUEST_TIMEOUT
    )
}

/// `Retry-After` in seconds; the HTTP date form is not supported.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds))
}

/// `Retry-After` if given, else exponential from 1 second; plus jitter.
fn rate_limit_delay(attempt: usize, retry_after: Option<Duration>) -> Duration {
    let base = retry_after.unwrap_or_else(|| Duration::from_secs(1 << attempt.min(5)));
    let jitter = Duration::from_millis(rand::rng().random_range(0..500));
    base.clamp(MIN_RATE_LIMIT_DELAY, MAX_RATE_LIMIT_DELAY) + jitter
}

/// Exponential from 1 to 16 seconds, plus jitter.
fn error_backoff(attempt: usize) -> Duration {
    let jitter = Duration::from_millis(rand::rng().random_range(0..250));
    Duration::from_secs(1 << attempt.min(4)) + jitter
}

#[cfg(test)]
mod tests {
    use super::{MAX_RATE_LIMIT_DELAY, MIN_RATE_LIMIT_DELAY, proxy_label, rate_limit_delay};
    use std::time::Duration;

    #[test]
    fn proxy_labels_hide_credentials() {
        assert_eq!(
            proxy_label("http://user:secret@10.0.0.1:3128"),
            "proxy://10.0.0.1:3128"
        );
        assert_eq!(
            proxy_label("https://proxy.example"),
            "proxy://proxy.example:443"
        );
        assert_eq!(proxy_label("not a url"), "proxy");
    }

    #[test]
    fn rate_limit_delay_honors_retry_after_within_bounds() {
        let jitter = Duration::from_millis(500);

        let delay = rate_limit_delay(0, Some(Duration::from_secs(10)));
        assert!(delay >= Duration::from_secs(10) && delay < Duration::from_secs(10) + jitter);

        let delay = rate_limit_delay(0, Some(Duration::ZERO));
        assert!(delay >= MIN_RATE_LIMIT_DELAY && delay < MIN_RATE_LIMIT_DELAY + jitter);

        let delay = rate_limit_delay(0, Some(Duration::from_secs(3600)));
        assert!(delay >= MAX_RATE_LIMIT_DELAY && delay < MAX_RATE_LIMIT_DELAY + jitter);
    }
}
