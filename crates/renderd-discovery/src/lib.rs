//! mDNS service discovery interface and implementations for Renderd.

#![allow(unsafe_code)]

pub mod error;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod manual;
pub mod record;
pub mod traits;
#[cfg(target_os = "windows")]
pub mod windows;

pub use error::DiscoveryError;
#[cfg(target_os = "macos")]
pub use macos::{BonjourAdvertiser, BonjourBrowser};
pub use manual::ManualBrowser;
pub use record::{DiscoveryEvent, ServiceRecord};
pub use traits::{Advertiser, Browser};
#[cfg(target_os = "windows")]
pub use windows::{WinDnsAdvertiser, WinDnsBrowser};
