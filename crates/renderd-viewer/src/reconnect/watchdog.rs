//! Reconnect watchdog with mDNS re-discovery (`renderd-viewer/src/reconnect/watchdog.rs`).
//!
//! Monitors connection status and automatically re-discovers host IP via mDNS by host UUID
//! when network connection drops (RFC-0002 §18.1).

use renderd_discovery::DiscoveryEvent;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Watchdog state machine status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogState {
    /// Active connection is healthy.
    Connected,
    /// Connection lost; attempting cached IP address once.
    AttemptingCachedIp,
    /// Cached IP failed; performing mDNS re-discovery scan.
    RediscoveringHost,
    /// Reconnect failed after max attempts; backing off.
    Backoff,
}

/// Connection watchdog managing host IP re-discovery and exponential backoff reconnects.
#[derive(Debug)]
pub struct ReconnectWatchdog {
    host_id: Uuid,
    cached_addr: SocketAddr,
    current_addr: SocketAddr,
    state: WatchdogState,
    attempts: u32,
    max_attempts: u32,
    base_backoff: Duration,
    max_backoff: Duration,
    last_attempt_at: Option<Instant>,
}

impl ReconnectWatchdog {
    /// Creates a new `ReconnectWatchdog` for target `host_id` and initial `cached_addr`.
    #[must_use]
    pub const fn new(host_id: Uuid, cached_addr: SocketAddr) -> Self {
        Self {
            host_id,
            cached_addr,
            current_addr: cached_addr,
            state: WatchdogState::Connected,
            attempts: 0,
            max_attempts: 10,
            base_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            last_attempt_at: None,
        }
    }

    /// Returns current watchdog state.
    #[must_use]
    pub const fn state(&self) -> WatchdogState {
        self.state
    }

    /// Returns active target host IP/port address.
    #[must_use]
    pub const fn current_addr(&self) -> SocketAddr {
        self.current_addr
    }

    /// Returns total reconnect attempts count.
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Returns maximum reconnect attempts configured.
    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Returns last attempt instant.
    #[must_use]
    pub const fn last_attempt_at(&self) -> Option<Instant> {
        self.last_attempt_at
    }

    /// Called when network connection drops to initiate reconnect watchdog loop.
    pub fn on_disconnect(&mut self) {
        self.state = WatchdogState::AttemptingCachedIp;
        self.attempts = 1;
        self.last_attempt_at = Some(Instant::now());
        tracing::warn!(
            host_id = %self.host_id,
            cached_addr = %self.cached_addr,
            "Network connection lost; attempting reconnect to cached IP"
        );
    }

    /// Handles failure of reconnect attempt to cached IP address and transitions to mDNS re-discovery.
    pub fn on_cached_ip_failed(&mut self) {
        self.state = WatchdogState::RediscoveringHost;
        self.attempts += 1;
        tracing::warn!(
            host_id = %self.host_id,
            "Cached IP failed; initiating mDNS re-discovery"
        );
    }

    /// Processes mDNS service discovery event; if discovered record matches target `host_id`, updates `current_addr`.
    pub fn process_discovery_event(&mut self, event: &DiscoveryEvent) -> bool {
        if self.state != WatchdogState::RediscoveringHost {
            return false;
        }

        if let DiscoveryEvent::Found(ref record) = event {
            if record.host_id == self.host_id {
                let socket_addr = SocketAddr::new(record.addr, record.port);
                self.current_addr = socket_addr;
                self.cached_addr = socket_addr;
                self.state = WatchdogState::Connected;
                self.attempts = 0;
                tracing::info!(
                    host_id = %self.host_id,
                    new_addr = %socket_addr,
                    "Re-discovered host IP via mDNS; connection restored"
                );
                return true;
            }
        }
        false
    }

    /// Manual IP update fallback when user enters new address directly.
    pub fn update_manual_addr(&mut self, addr: SocketAddr) {
        self.current_addr = addr;
        self.cached_addr = addr;
        self.state = WatchdogState::Connected;
        self.attempts = 0;
    }

    /// Computes exponential backoff duration based on attempt count.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn current_backoff(&self) -> Duration {
        let shift = self.attempts.saturating_sub(1).min(10);
        let multiplier = 1u64.checked_shl(shift).unwrap_or(1024);
        let backoff = self.base_backoff.mul_f64(multiplier as f64);
        if backoff > self.max_backoff {
            self.max_backoff
        } else {
            backoff
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderd_discovery::ServiceRecord;
    use std::collections::HashMap;

    #[test]
    fn test_reconnect_watchdog_state_transitions() {
        let host_id = Uuid::new_v4();
        let addr1: SocketAddr = "192.168.1.50:9000".parse().unwrap();
        let addr2: SocketAddr = "192.168.1.100:9000".parse().unwrap();

        let mut watchdog = ReconnectWatchdog::new(host_id, addr1);
        assert_eq!(watchdog.state(), WatchdogState::Connected);

        watchdog.on_disconnect();
        assert_eq!(watchdog.state(), WatchdogState::AttemptingCachedIp);

        watchdog.on_cached_ip_failed();
        assert_eq!(watchdog.state(), WatchdogState::RediscoveringHost);

        let event = DiscoveryEvent::Found(ServiceRecord {
            host_id,
            name: "TestHost".to_string(),
            addr: addr2.ip(),
            port: addr2.port(),
            txt: HashMap::new(),
        });

        let recovered = watchdog.process_discovery_event(&event);
        assert!(recovered);
        assert_eq!(watchdog.state(), WatchdogState::Connected);
        assert_eq!(watchdog.current_addr(), addr2);
    }

    #[test]
    fn test_reconnect_exponential_backoff() {
        let host_id = Uuid::new_v4();
        let addr: SocketAddr = "127.0.0.1:9000".parse().unwrap();
        let mut watchdog = ReconnectWatchdog::new(host_id, addr);

        watchdog.on_disconnect();
        let b1 = watchdog.current_backoff();
        assert_eq!(b1, Duration::from_millis(500));

        watchdog.on_cached_ip_failed();
        let b2 = watchdog.current_backoff();
        assert_eq!(b2, Duration::from_secs(1));
    }
}
