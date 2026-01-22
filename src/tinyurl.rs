//! URL resolution with rate limiting for TinyURL and similar services

use crate::error::{BridgeError, Result};
use std::thread;
use std::time::Duration;

/// Configuration for URL resolution with rate limiting
pub struct UrlResolver {
    client: reqwest::blocking::Client,
    delay_ms: u64,
    batch_size: usize,
    batch_delay_ms: u64,
    requests_in_batch: usize,
}

impl UrlResolver {
    /// Create a new URL resolver with default settings
    pub fn new() -> Self {
        Self::with_config(200, 10, 2000)
    }

    /// Create a URL resolver with custom rate limiting configuration
    ///
    /// # Arguments
    /// * `delay_ms` - Delay between individual requests in milliseconds
    /// * `batch_size` - Number of requests before a longer pause
    /// * `batch_delay_ms` - Duration of the longer pause in milliseconds
    pub fn with_config(delay_ms: u64, batch_size: usize, batch_delay_ms: u64) -> Self {
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none()) // Don't follow redirects automatically
            .timeout(Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            delay_ms,
            batch_size,
            batch_delay_ms,
            requests_in_batch: 0,
        }
    }

    /// Resolve a shortened URL to its final destination
    ///
    /// This follows redirects manually to capture the final URL.
    pub fn resolve(&mut self, short_url: &str) -> Result<String> {
        // Apply rate limiting
        self.apply_rate_limit();

        let mut current_url = short_url.to_string();
        let mut redirects = 0;
        const MAX_REDIRECTS: usize = 10;

        loop {
            let response = self
                .client
                .get(&current_url)
                .send()
                .map_err(|e| BridgeError::UrlResolution(format!("Request failed: {}", e)))?;

            let status = response.status();

            // Check for rate limiting
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(BridgeError::RateLimited);
            }

            // Check for service unavailable (often indicates rate limiting)
            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                // Check if it's a Cloudflare block
                let body = response.text().unwrap_or_default();
                if body.contains("Just a moment") || body.contains("Cloudflare") {
                    return Err(BridgeError::RateLimited);
                }
                return Err(BridgeError::UrlResolution(
                    "Service unavailable".to_string(),
                ));
            }

            // Handle redirects
            if status.is_redirection() {
                if let Some(location) = response.headers().get(reqwest::header::LOCATION) {
                    let location_str = location
                        .to_str()
                        .map_err(|_| BridgeError::UrlResolution("Invalid redirect URL".to_string()))?;

                    // Handle relative URLs
                    current_url = if location_str.starts_with("http") {
                        location_str.to_string()
                    } else {
                        // Parse the current URL and resolve the relative URL
                        let base = url::Url::parse(&current_url)
                            .map_err(|e| BridgeError::UrlResolution(format!("Invalid URL: {}", e)))?;
                        base.join(location_str)
                            .map_err(|e| BridgeError::UrlResolution(format!("Invalid redirect: {}", e)))?
                            .to_string()
                    };

                    redirects += 1;
                    if redirects > MAX_REDIRECTS {
                        return Err(BridgeError::UrlResolution(
                            "Too many redirects".to_string(),
                        ));
                    }
                    continue;
                }
            }

            // If we get here, we've reached the final URL
            if status.is_success() || !status.is_redirection() {
                return Ok(current_url);
            }

            return Err(BridgeError::UrlResolution(format!(
                "Unexpected status: {}",
                status
            )));
        }
    }

    /// Apply rate limiting based on configuration
    fn apply_rate_limit(&mut self) {
        self.requests_in_batch += 1;

        if self.requests_in_batch >= self.batch_size {
            // Apply longer batch delay
            thread::sleep(Duration::from_millis(self.batch_delay_ms));
            self.requests_in_batch = 0;
        } else {
            // Apply normal delay
            thread::sleep(Duration::from_millis(self.delay_ms));
        }
    }

    /// Reset the batch counter (e.g., after a pause)
    pub fn reset_batch(&mut self) {
        self.requests_in_batch = 0;
    }
}

impl Default for UrlResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires network access
    fn test_resolve_tinyurl() {
        let mut resolver = UrlResolver::with_config(100, 5, 1000);

        // Use a known TinyURL for testing
        // This test should be run manually to avoid hitting rate limits in CI
        let result = resolver.resolve("http://tinyurl.com/2n8bjtmz");

        match result {
            Ok(url) => {
                assert!(url.contains("bridgebase.com"));
                assert!(url.contains("lin="));
            }
            Err(BridgeError::RateLimited) => {
                // Expected if we've hit rate limits
                println!("Rate limited - this is expected behavior");
            }
            Err(e) => {
                panic!("Unexpected error: {}", e);
            }
        }
    }
}
