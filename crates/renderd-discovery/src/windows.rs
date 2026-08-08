//! `Win32` `DnsService` mDNS advertisement and browsing backend.

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::ptr;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use uuid::Uuid;
use windows::core::PCWSTR;
use windows::Win32::NetworkManagement::Dns::{
    DnsServiceBrowse, DnsServiceBrowseCancel, DnsServiceRegister, DNS_RECORDW,
    DNS_SERVICE_BROWSE_REQUEST, DNS_SERVICE_BROWSE_REQUEST_0, DNS_SERVICE_CANCEL,
    DNS_SERVICE_INSTANCE, DNS_SERVICE_REGISTER_REQUEST,
};

use crate::error::DiscoveryError;
use crate::record::{DiscoveryEvent, ServiceRecord};
use crate::traits::{Advertiser, Browser};

/// `Win32` mDNS advertiser using Windows `DnsServiceRegister`.
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

fn pwstr_to_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: Dereferences raw UTF-16 pointer up to null terminator.
    unsafe {
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        if len == 0 {
            return None;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16(slice).ok()
    }
}

/// Asynchronous callback invoked by Windows `DnsServiceBrowse` when mDNS services appear/update.
///
/// # SAFETY
/// Called by Win32 DNS API runtime (`dnsapi.dll`). `pquerycontext` points to a valid heap-allocated
/// `Sender<DiscoveryEvent>`. `pdnsrecord` points to a linked list of `DNS_RECORDW` structures.
unsafe extern "system" fn browse_callback(
    status: u32,
    pquerycontext: *const std::ffi::c_void,
    pdnsrecord: *const DNS_RECORDW,
) {
    if pquerycontext.is_null() {
        return;
    }

    let tx = &*(pquerycontext.cast::<Sender<DiscoveryEvent>>());

    if status != 0 || pdnsrecord.is_null() {
        return;
    }

    let mut curr = pdnsrecord;
    let mut discovered_name: Option<String> = None;
    let mut discovered_port: Option<u16> = None;
    let mut discovered_addr: Option<IpAddr> = None;

    while !curr.is_null() {
        let rec = &*curr;

        if discovered_name.is_none() && !rec.pName.0.is_null() {
            if let Some(s) = pwstr_to_string(rec.pName.0) {
                if !s.is_empty() {
                    discovered_name = Some(s);
                }
            }
        }

        match rec.wType {
            // PTR record (12)
            12 => {
                let ptr_data = rec.Data.PTR;
                if !ptr_data.pNameHost.0.is_null() {
                    if let Some(host_name) = pwstr_to_string(ptr_data.pNameHost.0) {
                        let clean_name = host_name
                            .strip_suffix("._renderd._udp.local")
                            .unwrap_or(&host_name)
                            .to_string();
                        discovered_name = Some(clean_name);
                    }
                }
            }
            // SRV record (33)
            33 => {
                let srv_data = rec.Data.SRV;
                discovered_port = Some(srv_data.wPort);
                if discovered_name.is_none() && !srv_data.pNameTarget.0.is_null() {
                    if let Some(target_name) = pwstr_to_string(srv_data.pNameTarget.0) {
                        let clean_name = target_name
                            .strip_suffix(".local")
                            .unwrap_or(&target_name)
                            .to_string();
                        discovered_name = Some(clean_name);
                    }
                }
            }
            // A record (1)
            1 => {
                let a_data = rec.Data.A;
                let ip_bytes = a_data.IpAddress.to_ne_bytes();
                discovered_addr = Some(IpAddr::V4(Ipv4Addr::from(ip_bytes)));
            }
            // AAAA record (28)
            28 => {
                let aaaa_data = rec.Data.AAAA;
                let ip6_bytes = aaaa_data.Ip6Address.IP6Byte;
                discovered_addr = Some(IpAddr::V6(std::net::Ipv6Addr::from(ip6_bytes)));
            }
            _ => {}
        }

        curr = rec.pNext;
    }

    if let Some(addr) = discovered_addr {
        let name = discovered_name.unwrap_or_else(|| "renderd-host".to_string());
        let port = discovered_port.unwrap_or(4433);
        let host_id = Uuid::new_v5(&Uuid::NAMESPACE_DNS, name.as_bytes());

        let record = ServiceRecord {
            host_id,
            name,
            addr,
            port,
            txt: HashMap::new(),
        };

        let _ = tx.try_send(DiscoveryEvent::Found(record));
    }
}

impl Advertiser for WinDnsAdvertiser {
    fn register(&mut self, record: &crate::record::ServiceRecord) -> Result<(), DiscoveryError> {
        self.unregister()?;

        let instance_name = format!("{}._renderd._udp.local", record.name);
        let mut instance_u16 = to_utf16(&instance_name);

        let mut instance = DNS_SERVICE_INSTANCE {
            pszInstanceName: windows::core::PWSTR(instance_u16.as_mut_ptr()),
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

            // SAFETY: DnsServiceRegister with empty/de-registration parameters.
            unsafe {
                let _ = DnsServiceRegister(&request, None);
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

/// `Win32` mDNS browser using Windows `DnsServiceBrowse`.
#[derive(Debug, Default)]
pub struct WinDnsBrowser {
    cancel: Option<DNS_SERVICE_CANCEL>,
    context: Option<usize>,
    query_buf: Vec<u16>,
}

unsafe impl Send for WinDnsBrowser {}
unsafe impl Sync for WinDnsBrowser {}

impl WinDnsBrowser {
    /// Creates a new [`WinDnsBrowser`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cancel: None,
            context: None,
            query_buf: Vec::new(),
        }
    }
}

impl Browser for WinDnsBrowser {
    fn start_browse(&mut self) -> Result<Receiver<DiscoveryEvent>, DiscoveryError> {
        self.stop_browse()?;

        let (tx, rx) = channel(32);
        let tx_box = Box::new(tx);
        let context_ptr = Box::into_raw(tx_box).cast::<std::ffi::c_void>();
        let context_addr = context_ptr as usize;

        let query_u16 = to_utf16("_renderd._udp.local");
        let mut cancel_handle = DNS_SERVICE_CANCEL::default();

        let request = DNS_SERVICE_BROWSE_REQUEST {
            Version: 1,
            InterfaceIndex: 0,
            QueryName: PCWSTR(query_u16.as_ptr()),
            Anonymous: DNS_SERVICE_BROWSE_REQUEST_0 {
                pBrowseCallback: Some(browse_callback),
            },
            pQueryContext: context_ptr,
        };

        // SAFETY: DnsServiceBrowse initiates mDNS discovery using Win32 DNS API.
        let status = unsafe { DnsServiceBrowse(&request, &mut cancel_handle) };
        if status != 0 && status != 9506 {
            // SAFETY: Reclaim leaked context box on error.
            unsafe {
                let _ = Box::from_raw(context_ptr.cast::<Sender<DiscoveryEvent>>());
            }
            return Err(DiscoveryError::BrowseFailed(format!(
                "DnsServiceBrowse failed with error code {status}"
            )));
        }

        self.cancel = Some(cancel_handle);
        self.context = Some(context_addr);
        self.query_buf = query_u16;

        Ok(rx)
    }

    fn stop_browse(&mut self) -> Result<(), DiscoveryError> {
        if let Some(cancel) = self.cancel.take() {
            // SAFETY: DnsServiceBrowseCancel cancels active DnsServiceBrowse query.
            unsafe {
                let _ = DnsServiceBrowseCancel(&cancel);
            }
        }
        if let Some(context_addr) = self.context.take() {
            // SAFETY: Reclaim context box after browse cancellation.
            unsafe {
                let _ = Box::from_raw(
                    (context_addr as *mut std::ffi::c_void).cast::<Sender<DiscoveryEvent>>(),
                );
            }
        }
        self.query_buf.clear();
        Ok(())
    }
}

impl Drop for WinDnsBrowser {
    fn drop(&mut self) {
        let _ = self.stop_browse();
    }
}
