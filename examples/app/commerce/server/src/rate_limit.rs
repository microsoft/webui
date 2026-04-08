// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

/// Simple in-memory IP-based rate limiter using a fixed-window counter.
pub(crate) struct RateLimiter {
    state: Mutex<LimiterState>,
    max_requests: u32,
    window_secs: u64,
}

struct LimiterState {
    buckets: HashMap<IpAddr, Bucket>,
    last_prune: Instant,
}

struct Bucket {
    count: u32,
    window_start: Instant,
}

impl RateLimiter {
    /// Create a new rate limiter that allows `max_requests` per `window_secs`
    /// per IP address.
    pub(crate) fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            state: Mutex::new(LimiterState {
                buckets: HashMap::new(),
                last_prune: Instant::now(),
            }),
            max_requests,
            window_secs,
        }
    }

    /// Returns `true` if the request is allowed, `false` if rate-limited.
    /// If `ip` is `None` (e.g. in tests or when behind a proxy that strips
    /// peer info), the request is allowed unconditionally.
    pub(crate) fn check(&self, ip: Option<IpAddr>) -> bool {
        let Some(ip) = ip else {
            return true;
        };

        let Ok(mut state) = self.state.lock() else {
            // Poisoned mutex — fail open rather than blocking all traffic.
            return true;
        };

        let now = Instant::now();
        let window = std::time::Duration::from_secs(self.window_secs);

        // Prune expired entries every 60 seconds to prevent unbounded growth.
        if now.duration_since(state.last_prune).as_secs() > 60 {
            state.buckets.retain(|_, bucket| now.duration_since(bucket.window_start) < window);
            state.last_prune = now;
        }

        let bucket = state.buckets.entry(ip).or_insert(Bucket {
            count: 0,
            window_start: now,
        });

        if now.duration_since(bucket.window_start) >= window {
            // Window expired — reset.
            bucket.count = 1;
            bucket.window_start = now;
            return true;
        }

        bucket.count += 1;
        bucket.count <= self.max_requests
    }
}

#[cfg(test)]
mod tests {
    use super::RateLimiter;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn allows_requests_within_limit() {
        let limiter = RateLimiter::new(3, 60);
        let ip = Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
    }

    #[test]
    fn rejects_requests_over_limit() {
        let limiter = RateLimiter::new(2, 60);
        let ip = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(limiter.check(ip));
        assert!(limiter.check(ip));
        assert!(!limiter.check(ip));
    }

    #[test]
    fn allows_none_ip() {
        let limiter = RateLimiter::new(1, 60);
        assert!(limiter.check(None));
        assert!(limiter.check(None));
    }

    #[test]
    fn tracks_ips_independently() {
        let limiter = RateLimiter::new(1, 60);
        let ip_a = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        let ip_b = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));
        assert!(limiter.check(ip_a));
        assert!(limiter.check(ip_b));
        assert!(!limiter.check(ip_a));
        assert!(!limiter.check(ip_b));
    }
}
