//! Windows Win32 DnsService mDNS advertisement and browsing backend.

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::ptr;
use tokio::sync::mpsc::{channel, Receiver};
use uuid::Uuid;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::NetworkManagement::Dns::{
    DnsServiceBrowse, DnsServiceBrowseCancel, DnsServiceDeRegister, DnsServiceRegister,
    DNS_SERVICE_BROWSE_REQUEST, DNS_SERVICE_CANCEL, DNS_SERVICE_INSTANCE,
    DNS_SERVICE_REGISTER_REQUEST,
};

use crate::error::DiscoveryError;
use crate::record::{DiscoveryEvent, ServiceRecord};
use crate::traits::{Advertiser, Browser};

/// Win32 mDNS advertiser using Windows `DnsServiceRegister`.
#[derive(Debug, Default)]
pub struct WinDnsAdvertiser {
    handle: Option<DNS_SERVICE_INSTANCE>,
}

unsafe impl Send for WinDnsAdvertiser {}
unsafe impl Sync for WinDnsAdvertiser {}

impl WinDnsAdvertiser {
    /// Creates a new [`WinDnsAdvertiser`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self { handle: None }
    }
}

fn to_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Advertiser for WinDnsAdvertiser {
    fn register(&mut self, record: &ServiceRecord) -> Result<(), DiscoveryError> {
        self.unregister()?;

        let instance_name = format!("{}._renderd._udp.local", record.name);
        let mut instance_u16 = to_utf16(&instance_name);

        let mut instance = DNS_SERVICE_INSTANCE {
            pszInstanceName: PWSTR(instance_u16.as_mut_ptr()),
            wPort: record.port,
            ..Default::default()
        };

        let request = DNS_SERVICE_REGISTER_REQUEST {
            Version: 1,
            pServiceInstance: &mut instance,
            ..Default::default()
        };

        // SAFETY: DnsServiceRegister takes valid DNS_SERVICE_REGISTER_REQUEST structure pointers.
        let status = unsafe { DnsServiceRegister(&request, None) };
        if status != 0 && status != 9506 {
            return Err(DiscoveryError::ServiceRegistrationFailed(format!(
                "DnsServiceRegister failed with error code {status}"
            )));
        }

        self.handle = Some(instance);
        Ok(())
    }

    fn unregister(&mut self) -> Result<(), DiscoveryError> {
        if let Some(mut instance) = self.handle.take() {
            let request = DNS_SERVICE_REGISTER_REQUEST {
                Version: 1,
                pServiceInstance: &mut instance,
                ..Default::default()
            };

            // SAFETY: DnsServiceDeRegister unregisters previously registered service instance.
            unsafe {
                let _ = DnsServiceDeRegister(&request, None);
            }
        }
        Ok(())
    }
}

impl Drop for WinDnsAdvertiser {
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

/// Win32 mDNS browser using Windows `DnsServiceBrowse`.
#[derive(Debug, Default)]
pub struct WinDnsBrowser {
    cancel: Option<DNS_SERVICE_CANCEL>,
}

unsafe impl Send for WinDnsBrowser {}
unsafe impl Sync for WinDnsBrowser {}

impl WinDnsBrowser {
    /// Creates a new [`WinDnsBrowser`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self { cancel: None }
    }
}

impl Browser for WinDnsBrowser {
    fn start_browse(&mut self) -> Result<Receiver<DiscoveryEvent>, DiscoveryError> {
        self.stop_browse()?;

        let (_tx, rx) = channel(32);
        let query_u16 = to_utf16("_renderd._udp.local");
        let mut cancel_handle = DNS_SERVICE_CANCEL::default();

        let request = DNS_SERVICE_BROWSE_REQUEST {
            Version: 1,
            QueryName: PCWSTR(query_u16.as_ptr()),
            pQueryContext: ptr::null_mut(),
            ..Default::default()
        };

        // SAFETY: DnsServiceBrowse initiates mDNS discovery using Win32 DNS API.
        let status = unsafe { DnsServiceBrowse(&request, &mut cancel_handle) };
        if status != 0 && status != 9506 {
            return Err(DiscoveryError::BrowseFailed(format!(
                "DnsServiceBrowse failed with error code {status}"
            )));
        }

        self.cancel = Some(cancel_handle);
        Ok(rx)
    }

    fn stop_browse(&mut self) -> Result<(), DiscoveryError> {
        if let Some(mut cancel) = self.cancel.take() {
            // SAFETY: DnsServiceBrowseCancel cancels active DnsServiceBrowse query.
            unsafe {
                let _ = DnsServiceBrowseCancel(&mut cancel);
            }
        }
        Ok(())
    }
}

impl Drop for WinDnsBrowser {
    fn drop(&mut self) {
        let _ = self.stop_browse();
    }
}
