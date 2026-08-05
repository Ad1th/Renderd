//! macOS Bonjour (`dns_sd.h`) service advertisement and discovery backend.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::net::{IpAddr, Ipv4Addr};
use std::ptr;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use uuid::Uuid;

use crate::error::DiscoveryError;
use crate::record::{DiscoveryEvent, ServiceRecord};
use crate::traits::{Advertiser, Browser};

type DNSServiceRef = *mut c_void;
type DNSServiceFlags = u32;
type DNSServiceErrorType = i32;

#[derive(Debug, Clone, Copy)]
struct SendDNSServiceRef(usize);

impl SendDNSServiceRef {
    #[inline]
    const fn as_ptr(self) -> DNSServiceRef {
        self.0 as DNSServiceRef
    }
}

const SERVICE_TYPE: &str = "_renderd._udp";

#[link(name = "System", kind = "framework")]
extern "C" {
    fn DNSServiceRegister(
        sdRef: *mut DNSServiceRef,
        flags: DNSServiceFlags,
        interfaceIndex: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        host: *const c_char,
        port: u16,
        txtLen: u16,
        txtRecord: *const c_void,
        callBack: Option<
            unsafe extern "C" fn(
                DNSServiceRef,
                DNSServiceFlags,
                DNSServiceErrorType,
                *const c_char,
                *const c_char,
                *const c_char,
                *mut c_void,
            ),
        >,
        context: *mut c_void,
    ) -> DNSServiceErrorType;

    fn DNSServiceBrowse(
        sdRef: *mut DNSServiceRef,
        flags: DNSServiceFlags,
        interfaceIndex: u32,
        regtype: *const c_char,
        domain: *const c_char,
        callBack: Option<
            unsafe extern "C" fn(
                DNSServiceRef,
                DNSServiceFlags,
                u32,
                DNSServiceErrorType,
                *const c_char,
                *const c_char,
                *const c_char,
                *mut c_void,
            ),
        >,
        context: *mut c_void,
    ) -> DNSServiceErrorType;

    fn DNSServiceProcessResult(sdRef: DNSServiceRef) -> DNSServiceErrorType;
    fn DNSServiceRefDeallocate(sdRef: DNSServiceRef);
}

/// Bonjour service advertiser using macOS system `dns_sd.h`.
#[derive(Debug, Default)]
pub struct BonjourAdvertiser {
    sd_ref: Option<SendDNSServiceRef>,
}

impl BonjourAdvertiser {
    /// Creates a new [`BonjourAdvertiser`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self { sd_ref: None }
    }
}

impl Advertiser for BonjourAdvertiser {
    fn register(&mut self, record: &ServiceRecord) -> Result<(), DiscoveryError> {
        self.unregister()?;

        let regtype = CString::new(SERVICE_TYPE)
            .map_err(|e| DiscoveryError::ServiceRegistrationFailed(e.to_string()))?;
        let name = CString::new(record.name.as_str())
            .map_err(|e| DiscoveryError::ServiceRegistrationFailed(e.to_string()))?;

        let mut txt_bytes = Vec::new();
        let mut add_txt = |key: &str, val: &str| {
            let item = format!("{key}={val}");
            let bytes = item.as_bytes();
            if let Ok(len) = u8::try_from(bytes.len()) {
                txt_bytes.push(len);
                txt_bytes.extend_from_slice(bytes);
            }
        };

        add_txt("version", "1");
        add_txt("id", &record.host_id.to_string());
        add_txt("name", &record.name);
        for (k, v) in &record.txt {
            add_txt(k, v);
        }

        let port_be = record.port.to_be();
        let mut sd_ref: DNSServiceRef = ptr::null_mut();

        // SAFETY: Calling C DNSServiceRegister with null-terminated strings and valid TXT buffer.
        let err = unsafe {
            DNSServiceRegister(
                &mut sd_ref,
                0,
                0,
                name.as_ptr(),
                regtype.as_ptr(),
                ptr::null(),
                ptr::null(),
                port_be,
                u16::try_from(txt_bytes.len()).unwrap_or(0),
                txt_bytes.as_ptr().cast::<c_void>(),
                None,
                ptr::null_mut(),
            )
        };

        if err != 0 {
            return Err(DiscoveryError::ServiceRegistrationFailed(format!(
                "DNSServiceRegister failed with code {err}"
            )));
        }

        self.sd_ref = Some(SendDNSServiceRef(sd_ref as usize));
        Ok(())
    }

    fn unregister(&mut self) -> Result<(), DiscoveryError> {
        if let Some(send_ref) = self.sd_ref.take() {
            // SAFETY: Deallocates valid DNSServiceRef handle.
            unsafe {
                DNSServiceRefDeallocate(send_ref.as_ptr());
            }
        }
        Ok(())
    }
}

impl Drop for BonjourAdvertiser {
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

/// Bonjour service browser using macOS system `dns_sd.h`.
#[derive(Debug, Default)]
pub struct BonjourBrowser {
    sd_ref: Option<SendDNSServiceRef>,
}

impl BonjourBrowser {
    /// Creates a new [`BonjourBrowser`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self { sd_ref: None }
    }
}

unsafe extern "C" fn browse_callback(
    _sd_ref: DNSServiceRef,
    flags: DNSServiceFlags,
    _interface_index: u32,
    error_code: DNSServiceErrorType,
    service_name: *const c_char,
    _regtype: *const c_char,
    _reply_domain: *const c_char,
    context: *mut c_void,
) {
    if error_code != 0 || context.is_null() || service_name.is_null() {
        return;
    }

    let tx = &*(context as *const Sender<DiscoveryEvent>);
    let name_str = match CStr::from_ptr(service_name).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return,
    };

    let host_id = Uuid::new_v5(&Uuid::NAMESPACE_DNS, name_str.as_bytes());
    let is_add = (flags & 0x2) != 0;

    if is_add {
        let record = ServiceRecord {
            host_id,
            name: name_str,
            addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 9000,
            txt: HashMap::new(),
        };
        let _ = tx.try_send(DiscoveryEvent::Found(record));
    } else {
        let _ = tx.try_send(DiscoveryEvent::Lost(host_id));
    }
}

impl Browser for BonjourBrowser {
    fn start_browse(&mut self) -> Result<Receiver<DiscoveryEvent>, DiscoveryError> {
        self.stop_browse()?;

        let (tx, rx) = channel(32);
        let tx_box = Box::new(tx);
        let context_ptr = Box::into_raw(tx_box).cast::<c_void>();
        let context_addr = context_ptr as usize;

        let regtype =
            CString::new(SERVICE_TYPE).map_err(|e| DiscoveryError::BrowseFailed(e.to_string()))?;
        let mut sd_ref: DNSServiceRef = ptr::null_mut();

        // SAFETY: DNSServiceBrowse registers a browse query with macOS system mDNSResponder daemon.
        let err = unsafe {
            DNSServiceBrowse(
                &mut sd_ref,
                0,
                0,
                regtype.as_ptr(),
                ptr::null(),
                Some(browse_callback),
                context_ptr,
            )
        };

        if err != 0 {
            // SAFETY: Reclaim leaked context box on error.
            unsafe {
                let _ = Box::from_raw(context_ptr.cast::<Sender<DiscoveryEvent>>());
            }
            return Err(DiscoveryError::BrowseFailed(format!(
                "DNSServiceBrowse failed with code {err}"
            )));
        }

        let send_ref = SendDNSServiceRef(sd_ref as usize);
        self.sd_ref = Some(send_ref);

        tokio::spawn(async move {
            loop {
                let res = tokio::task::spawn_blocking(move || {
                    let ptr = send_ref.as_ptr();
                    unsafe { DNSServiceProcessResult(ptr) }
                })
                .await;

                if matches!(res, Ok(err) if err != 0) || res.is_err() {
                    break;
                }
            }

            // SAFETY: Reclaim context box on background task exit.
            unsafe {
                let _ =
                    Box::from_raw((context_addr as *mut c_void).cast::<Sender<DiscoveryEvent>>());
            }
        });

        Ok(rx)
    }

    fn stop_browse(&mut self) -> Result<(), DiscoveryError> {
        if let Some(send_ref) = self.sd_ref.take() {
            // SAFETY: Deallocates valid DNSServiceRef handle.
            unsafe {
                DNSServiceRefDeallocate(send_ref.as_ptr());
            }
        }
        Ok(())
    }
}

impl Drop for BonjourBrowser {
    fn drop(&mut self) {
        let _ = self.stop_browse();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bonjour_advertiser_creation() {
        let adv = BonjourAdvertiser::new();
        assert!(adv.sd_ref.is_none());
    }

    #[test]
    fn test_bonjour_browser_creation() {
        let browser = BonjourBrowser::new();
        assert!(browser.sd_ref.is_none());
    }
}
