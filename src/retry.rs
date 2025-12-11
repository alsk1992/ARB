use anyhow::Result;
use backoff::ExponentialBackoff;
use std::future::Future;
use std::time::Duration;
use tracing::{debug, warn};

/// Retry configuration for API calls
pub fn get_backoff() -> ExponentialBackoff {
    ExponentialBackoff {
        initial_interval: Duration::from_millis(100),
        max_interval: Duration::from_secs(5),
        max_elapsed_time: Some(Duration::from_secs(30)),
        multiplier: 2.0,
        randomization_factor: 0.1,
        ..Default::default()
    }
}

/// Retry an async operation with exponential backoff
pub async fn retry_async<F, Fut, T, E>(
    operation_name: &str,
    max_retries: u32,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0;
    let mut delay = Duration::from_millis(100);

    loop {
        attempt += 1;
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if attempt >= max_retries {
                    return Err(anyhow::anyhow!(
                        "{} failed after {} attempts: {}",
                        operation_name,
                        attempt,
                        e
                    ));
                }
                warn!(
                    "{} attempt {}/{} failed: {}. Retrying in {:?}",
                    operation_name, attempt, max_retries, e, delay
                );
                tokio::time::sleep(delay).await;
                delay = std::cmp::min(delay * 2, Duration::from_secs(5));
            }
        }
    }
}

/// Retry with custom backoff settings
pub async fn retry_with_config<F, Fut, T, E>(
    operation_name: &str,
    max_retries: u32,
    initial_delay_ms: u64,
    mut operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0;
    let mut delay = Duration::from_millis(initial_delay_ms);

    loop {
        attempt += 1;
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                if attempt >= max_retries {
                    return Err(anyhow::anyhow!(
                        "{} failed after {} attempts: {}",
                        operation_name,
                        attempt,
                        e
                    ));
                }
                warn!(
                    "{} attempt {}/{} failed: {}. Retrying in {:?}",
                    operation_name, attempt, max_retries, e, delay
                );
                tokio::time::sleep(delay).await;
                delay = std::cmp::min(delay * 2, Duration::from_secs(5));
            }
        }
    }
}

/// Circuit breaker state
pub struct CircuitBreaker {
    failures: std::sync::atomic::AtomicU32,
    last_failure: parking_lot::Mutex<Option<std::time::Instant>>,
    threshold: u32,
    reset_timeout: Duration,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            failures: std::sync::atomic::AtomicU32::new(0),
            last_failure: parking_lot::Mutex::new(None),
            threshold,
            reset_timeout,
        }
    }

    pub fn is_open(&self) -> bool {
        let failures = self.failures.load(std::sync::atomic::Ordering::Relaxed);
        if failures < self.threshold {
            return false;
        }

        // Check if we should reset
        if let Some(last) = *self.last_failure.lock() {
            if last.elapsed() > self.reset_timeout {
                self.reset();
                return false;
            }
        }

        true
    }

    pub fn record_success(&self) {
        self.failures.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        self.failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.last_failure.lock() = Some(std::time::Instant::now());
    }

    pub fn reset(&self) {
        self.failures.store(0, std::sync::atomic::Ordering::Relaxed);
        *self.last_failure.lock() = None;
    }
}
