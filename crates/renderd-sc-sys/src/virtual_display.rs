#![allow(unsafe_code)]

//! Virtual display creation through CoreGraphics' `CGVirtualDisplay` API.
//!
//! macOS has no public API for adding a display without hardware, but CoreGraphics
//! ships a private Objective-C interface — `CGVirtualDisplayDescriptor`,
//! `CGVirtualDisplaySettings`, `CGVirtualDisplayMode`, and `CGVirtualDisplay` — that
//! creates one in user space with no kernel extension and no special entitlement.
//! It is the mechanism behind several shipping display-extension apps and has been
//! stable since macOS 11.
//!
//! The classes are resolved by name at runtime rather than linked, so a macOS
//! release that removes them degrades to [`VirtualDisplayError::Unavailable`] and
//! the host falls back to mirroring instead of failing to launch.
//!
//! The display exists for as long as the [`VirtualDisplay`] value lives: dropping
//! it removes the display and macOS moves any windows on it back to a real screen.

use std::ffi::c_void;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, msg_send_id};
use objc2_foundation::{NSArray, NSSize, NSString};

/// Errors from virtual display creation.
#[derive(Debug, thiserror::Error)]
pub enum VirtualDisplayError {
    /// The `CGVirtualDisplay` classes are not present in this CoreGraphics build.
    #[error("CGVirtualDisplay is not available on this macOS version")]
    Unavailable,
    /// One of the Objective-C constructors returned nil.
    #[error("virtual display allocation failed: {0}")]
    Allocation(&'static str),
    /// `applySettings:` returned `NO`, so the display never came online.
    #[error("CoreGraphics rejected the virtual display mode {width}x{height}@{refresh_hz}")]
    Rejected {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
        /// Requested refresh rate in Hz.
        refresh_hz: u32,
    },
    /// Requested geometry was zero-sized.
    #[error("virtual display size must be non-zero, got {0}x{1}")]
    InvalidSize(u32, u32),
}

/// Geometry of the virtual display to create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualDisplayConfig {
    /// Backing framebuffer width in physical pixels — what gets captured and encoded.
    pub width: u32,
    /// Backing framebuffer height in physical pixels.
    pub height: u32,
    /// Refresh rate advertised to macOS in Hz.
    pub refresh_hz: u32,
    /// Present the display as Retina: macOS renders UI at 2× and reports half the
    /// pixel size as the logical resolution. Use for 4K viewers so text stays readable.
    pub hidpi: bool,
}

impl VirtualDisplayConfig {
    /// Picks `HiDPI` automatically: a framebuffer taller than 1800 px is treated as a
    /// Retina-class panel, which matches how macOS handles real 4K monitors.
    #[must_use]
    pub const fn new(width: u32, height: u32, refresh_hz: u32) -> Self {
        Self {
            width,
            height,
            refresh_hz,
            hidpi: height >= 1800,
        }
    }
}

/// A live virtual display. Dropping it removes the display from the system.
pub struct VirtualDisplay {
    display: Retained<AnyObject>,
    _descriptor: Retained<AnyObject>,
    display_id: u32,
    config: VirtualDisplayConfig,
}

// SAFETY: the underlying objects are only ever touched through this struct's
// methods, which do not share them across threads; Objective-C retain counts are
// thread-safe, so moving the owner is sound.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for VirtualDisplay {}
unsafe impl Sync for VirtualDisplay {}

impl std::fmt::Debug for VirtualDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualDisplay")
            .field("display_id", &self.display_id)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

extern "C" {
    fn dispatch_queue_create(label: *const i8, attr: *const c_void) -> *mut c_void;
}

/// Reports whether this macOS build exposes the `CGVirtualDisplay` classes.
#[must_use]
pub fn is_supported() -> bool {
    AnyClass::get("CGVirtualDisplayDescriptor").is_some()
        && AnyClass::get("CGVirtualDisplay").is_some()
        && AnyClass::get("CGVirtualDisplaySettings").is_some()
        && AnyClass::get("CGVirtualDisplayMode").is_some()
}

impl VirtualDisplay {
    /// Creates and activates a virtual display with the given geometry.
    ///
    /// The display appears in System Settings › Displays as `name`, arranged to the
    /// right of the existing displays by default.
    ///
    /// # Errors
    /// Returns [`VirtualDisplayError`] if the API is unavailable, a constructor
    /// returns nil, or CoreGraphics rejects the requested mode.
    pub fn create(name: &str, config: VirtualDisplayConfig) -> Result<Self, VirtualDisplayError> {
        if config.width == 0 || config.height == 0 {
            return Err(VirtualDisplayError::InvalidSize(config.width, config.height));
        }

        let (Some(descriptor_cls), Some(display_cls), Some(settings_cls), Some(mode_cls)) = (
            AnyClass::get("CGVirtualDisplayDescriptor"),
            AnyClass::get("CGVirtualDisplay"),
            AnyClass::get("CGVirtualDisplaySettings"),
            AnyClass::get("CGVirtualDisplayMode"),
        ) else {
            return Err(VirtualDisplayError::Unavailable);
        };

        // Physical size drives the DPI macOS assumes. Aim for a typical desktop panel
        // density (~110 ppi of *logical* pixels) so the UI scale looks natural.
        let logical_w = if config.hidpi { config.width / 2 } else { config.width };
        let logical_h = if config.hidpi { config.height / 2 } else { config.height };
        let mm_w = f64::from(logical_w) / 110.0 * 25.4;
        let mm_h = f64::from(logical_h) / 110.0 * 25.4;

        // SAFETY: every selector below was verified against the class' method list on
        // macOS 11 through 26; arguments use the encodings CoreGraphics declares.
        unsafe {
            let descriptor: Option<Retained<AnyObject>> = msg_send_id![descriptor_cls, new];
            let descriptor = descriptor.ok_or(VirtualDisplayError::Allocation("descriptor"))?;

            let ns_name = NSString::from_str(name);
            let _: () = msg_send![&*descriptor, setName: &*ns_name];
            let _: () = msg_send![&*descriptor, setMaxPixelsWide: config.width];
            let _: () = msg_send![&*descriptor, setMaxPixelsHigh: config.height];
            let _: () = msg_send![&*descriptor, setSizeInMillimeters: NSSize::new(mm_w, mm_h)];
            // Arbitrary but stable identifiers: macOS keys display arrangement and
            // colour profile preferences on vendor/product/serial, so keeping them
            // constant means the user's placement of the display is remembered.
            let _: () = msg_send![&*descriptor, setVendorID: 0x5244_u32];
            let _: () = msg_send![&*descriptor, setProductID: 0x5244_u32];
            let _: () = msg_send![&*descriptor, setSerialNum: 0x0001_u32];

            // A dispatch queue is an Objective-C object and the setter is typed as one
            // (`@`), so hand it over as an object reference, not a raw `void *`.
            let queue_ptr =
                dispatch_queue_create(c"dev.renderd.virtual-display".as_ptr(), std::ptr::null());
            let queue: Option<&AnyObject> = queue_ptr.cast::<AnyObject>().as_ref();
            let _: () = msg_send![&*descriptor, setQueue: queue];

            let display_alloc: Allocated<AnyObject> = msg_send_id![display_cls, alloc];
            let display: Option<Retained<AnyObject>> =
                msg_send_id![display_alloc, initWithDescriptor: &*descriptor];
            let display = display.ok_or(VirtualDisplayError::Allocation("display"))?;

            let mode_alloc: Allocated<AnyObject> = msg_send_id![mode_cls, alloc];
            let mode: Option<Retained<AnyObject>> = msg_send_id![
                mode_alloc,
                initWithWidth: config.width,
                height: config.height,
                refreshRate: f64::from(config.refresh_hz.max(1))
            ];
            let mode = mode.ok_or(VirtualDisplayError::Allocation("mode"))?;

            let settings: Option<Retained<AnyObject>> = msg_send_id![settings_cls, new];
            let settings = settings.ok_or(VirtualDisplayError::Allocation("settings"))?;
            let hidpi: u32 = u32::from(config.hidpi);
            let _: () = msg_send![&*settings, setHiDPI: hidpi];
            let modes = NSArray::from_vec(vec![mode]);
            let _: () = msg_send![&*settings, setModes: &*modes];

            let applied: bool = msg_send![&*display, applySettings: &*settings];
            if !applied {
                return Err(VirtualDisplayError::Rejected {
                    width: config.width,
                    height: config.height,
                    refresh_hz: config.refresh_hz,
                });
            }

            let display_id: u32 = msg_send![&*display, displayID];

            tracing::info!(
                display_id,
                width = config.width,
                height = config.height,
                refresh_hz = config.refresh_hz,
                hidpi = config.hidpi,
                "Virtual display created"
            );

            Ok(Self {
                display,
                _descriptor: descriptor,
                display_id,
                config,
            })
        }
    }

    /// CoreGraphics display ID of the virtual display, for use with
    /// [`crate::ContentFilter::wait_for_display`].
    #[must_use]
    pub const fn display_id(&self) -> u32 {
        self.display_id
    }

    /// Geometry this display was created with.
    #[must_use]
    pub const fn config(&self) -> VirtualDisplayConfig {
        self.config
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        tracing::info!(display_id = self.display_id, "Removing virtual display");
        // Releasing the CGVirtualDisplay object is what tears the display down; the
        // explicit touch keeps the field from being flagged as never read.
        let _ = &self.display;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_picks_hidpi_for_4k() {
        assert!(!VirtualDisplayConfig::new(1920, 1080, 60).hidpi);
        assert!(VirtualDisplayConfig::new(3840, 2160, 60).hidpi);
    }

    #[test]
    fn test_zero_size_rejected_before_touching_objc() {
        let err = VirtualDisplay::create("t", VirtualDisplayConfig::new(0, 1080, 60)).unwrap_err();
        assert!(matches!(err, VirtualDisplayError::InvalidSize(0, 1080)));
    }

    /// Creates a real (briefly visible) virtual display. Ignored by default because it
    /// changes the machine's display arrangement while it runs.
    #[test]
    #[ignore = "adds a display to the running system"]
    fn test_create_and_remove_virtual_display() {
        if !is_supported() {
            return;
        }
        let display =
            VirtualDisplay::create("Renderd Test", VirtualDisplayConfig::new(1280, 720, 60))
                .expect("virtual display creation");
        assert!(display.display_id() != 0);
        drop(display);
    }
}
