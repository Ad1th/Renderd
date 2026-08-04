#![allow(unsafe_code)]

//! Safe Rust wrapper for macOS `IOSurfaceRef` GPU memory handles.
//!
//! `IOSurface` handles represent shared GPU-resident pixel buffers passed between
//! `ScreenCaptureKit` and `VideoToolbox` without copying frame data to host memory.
//!
//! This module provides [`IoSurface`], an RAII wrapper over `IOSurfaceRef` that manages
//! CoreFoundation reference counting via `CFRetain` and `CFRelease`.

use core_foundation::base::{CFGetRetainCount, CFRelease, CFRetain, CFTypeID, CFTypeRef};

/// Raw opaque `IOSurfaceRef` handle.
pub type IOSurfaceRef = *const std::ffi::c_void;

/// Unique 32-bit identifier for an `IOSurface`.
pub type IOSurfaceID = u32;

extern "C" {
    /// Returns the CoreFoundation type identifier for `IOSurface`.
    pub fn IOSurfaceGetTypeID() -> CFTypeID;

    /// Returns the unique 32-bit ID for an `IOSurfaceRef`.
    pub fn IOSurfaceGetID(buffer: IOSurfaceRef) -> IOSurfaceID;

    /// Looks up an `IOSurfaceRef` by its unique 32-bit ID.
    pub fn IOSurfaceLookup(csid: IOSurfaceID) -> IOSurfaceRef;
}

/// Safe RAII wrapper around macOS `IOSurfaceRef`.
///
/// Automatically calls `CFRetain` on [`Clone`] and `CFRelease` on [`Drop`].
/// `IoSurface` is thread-safe (`Send` + `Sync`) as underlying `IOSurfaceRef`
/// memory objects are designed for concurrent multi-process GPU sharing.
pub struct IoSurface(IOSurfaceRef);

impl IoSurface {
    /// Constructs an `IoSurface` from an owned raw `IOSurfaceRef`.
    ///
    /// The constructed `IoSurface` takes ownership of the existing retain count
    /// and will call `CFRelease` when dropped.
    ///
    /// Returns `None` if `ptr` is null.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is either null or a valid, live `IOSurfaceRef`
    /// with an owned reference count (+1) transferred to this function.
    #[must_use]
    pub unsafe fn from_raw(ptr: IOSurfaceRef) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            Some(Self(ptr))
        }
    }

    /// Constructs an `IoSurface` by retaining a borrowed raw `IOSurfaceRef`.
    ///
    /// Calls `CFRetain` on `ptr` before returning.
    /// Returns `None` if `ptr` is null.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` is either null or a valid, live `IOSurfaceRef`.
    #[must_use]
    pub unsafe fn from_raw_retained(ptr: IOSurfaceRef) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            // SAFETY: ptr is non-null and guaranteed by caller to be a valid CF object.
            unsafe {
                CFRetain(ptr.cast());
            }
            Some(Self(ptr))
        }
    }

    /// Returns the underlying raw `IOSurfaceRef` handle.
    ///
    /// The returned pointer remains valid for the lifetime of `self`.
    #[must_use]
    pub const fn as_raw(&self) -> IOSurfaceRef {
        self.0
    }

    /// Returns the unique 32-bit ID of this `IOSurface`.
    #[must_use]
    pub fn id(&self) -> IOSurfaceID {
        // SAFETY: self.0 is guaranteed to be a valid non-null IOSurfaceRef for the lifetime of self.
        unsafe { IOSurfaceGetID(self.0) }
    }

    /// Returns the current CoreFoundation retain count of the underlying `IOSurfaceRef`.
    #[must_use]
    pub fn retain_count(&self) -> isize {
        // SAFETY: self.0 is guaranteed to be a valid non-null CFTypeRef for the lifetime of self.
        unsafe { CFGetRetainCount(self.0 as CFTypeRef) }
    }
}

impl Clone for IoSurface {
    fn clone(&self) -> Self {
        // SAFETY: self.0 is a valid non-null IOSurfaceRef. Increment reference count.
        unsafe {
            CFRetain(self.0 as CFTypeRef);
        }
        Self(self.0)
    }
}

impl Drop for IoSurface {
    fn drop(&mut self) {
        // SAFETY: self.0 is a valid non-null IOSurfaceRef owned by this struct. Decrement reference count.
        unsafe {
            CFRelease(self.0 as CFTypeRef);
        }
    }
}

impl std::fmt::Debug for IoSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoSurface")
            .field("id", &self.id())
            .field("retain_count", &self.retain_count())
            .field("ptr", &self.0)
            .finish()
    }
}

// SAFETY: IOSurfaceRef GPU memory handles are thread-safe and safe to send across thread boundaries.
unsafe impl Send for IoSurface {}

// SAFETY: IOSurfaceRef handles permit concurrent reference operations across threads.
unsafe impl Sync for IoSurface {}

#[cfg(test)]
mod tests {
    use super::*;
    use core_foundation::base::TCFType;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;

    extern "C" {
        fn IOSurfaceCreate(properties: CFTypeRef) -> IOSurfaceRef;
    }

    fn create_test_iosurface(width: i32, height: i32) -> IOSurfaceRef {
        let width_key = CFString::new("IOSurfaceWidth");
        let height_key = CFString::new("IOSurfaceHeight");
        let bytes_per_elem_key = CFString::new("IOSurfaceBytesPerElement");

        let width_val = CFNumber::from(width);
        let height_val = CFNumber::from(height);
        let bytes_per_elem_val = CFNumber::from(4);

        let dict = CFDictionary::from_CFType_pairs(&[
            (width_key.as_CFType(), width_val.as_CFType()),
            (height_key.as_CFType(), height_val.as_CFType()),
            (
                bytes_per_elem_key.as_CFType(),
                bytes_per_elem_val.as_CFType(),
            ),
        ]);

        // SAFETY: dict is a valid CFDictionary.
        unsafe { IOSurfaceCreate(dict.as_concrete_TypeRef().cast()) }
    }

    #[test]
    fn test_from_raw_null() {
        // SAFETY: null pointer test.
        let surface = unsafe { IoSurface::from_raw(std::ptr::null()) };
        assert!(surface.is_none());

        // SAFETY: null pointer test.
        let surface_retained = unsafe { IoSurface::from_raw_retained(std::ptr::null()) };
        assert!(surface_retained.is_none());
    }

    #[test]
    fn test_retain_release_cycle() {
        let raw = create_test_iosurface(32, 32);
        assert!(!raw.is_null(), "IOSurfaceCreate failed");

        // SAFETY: raw is a valid newly created IOSurfaceRef with retain count 1.
        let surface1 = unsafe { IoSurface::from_raw(raw) }.expect("from_raw failed");
        let initial_count = surface1.retain_count();
        assert_eq!(initial_count, 1);
        assert!(surface1.id() > 0);

        // Clone increments retain count
        let surface2 = surface1.clone();
        assert_eq!(surface1.retain_count(), 2);
        assert_eq!(surface2.retain_count(), 2);
        assert_eq!(surface1.id(), surface2.id());

        // Drop decrements retain count
        drop(surface2);
        assert_eq!(surface1.retain_count(), 1);
    }

    #[test]
    fn test_from_raw_retained() {
        let raw = create_test_iosurface(16, 16);
        assert!(!raw.is_null());

        // SAFETY: raw has count 1. from_raw_retained increments to 2.
        let surface =
            unsafe { IoSurface::from_raw_retained(raw) }.expect("from_raw_retained failed");
        assert_eq!(surface.retain_count(), 2);

        // Clean up the original raw reference
        // SAFETY: raw is a valid CFTypeRef.
        unsafe {
            CFRelease(raw.cast());
        }

        // surface now holds the remaining count 1
        assert_eq!(surface.retain_count(), 1);
    }
}
