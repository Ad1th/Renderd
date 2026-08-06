//! `VideoToolbox` hardware encode FFI wrapper for Renderd.
//!
//! This crate provides safe Rust bindings to Apple's `VideoToolbox` framework,
//! enabling hardware-accelerated H.265 (HEVC) and H.264 (AVC) video encoding
//! via `VTCompressionSession`.
//!
//! # Platform
//!
//! This crate **only compiles functional code on macOS** (`target_os = "macos"`).
//! On all other targets the crate is present but exports no items, allowing the
//! full workspace to compile in CI on Linux and Windows runners.
//!
//! # Architecture
//!
//! `VideoToolbox` is a pure C API built on `CoreFoundation` types. It cannot be driven
//! through `objc2` because it is not an Objective-C API. Instead, this crate uses:
//!
//! 1. A thin C bridge shim (`c-shims/videotoolbox_shim.c`, added in issue #037)
//!    that converts `VideoToolbox`'s C-function-pointer output callback into a form
//!    that Rust's FFI can call safely without closure capture issues.
//! 2. Safe Rust wrappers in `src/error.rs`, `src/surface.rs`, and `src/session.rs`
//!    (added in issues #041, #040, #038) that provide RAII lifecycle management over
//!    opaque C types.
//!
//! # Safety
//!
//! All `unsafe` blocks in this crate include a `// SAFETY:` comment explaining the
//! invariants that make each operation sound. The crate lint policy sets
//! `unsafe_code = "warn"` (not `"deny"`) because FFI is the explicit purpose of
//! this crate, but every unsafe block is individually justified.

#![cfg_attr(not(target_os = "macos"), allow(unused))]

#[cfg(target_os = "macos")]
pub mod bindings;

#[cfg(target_os = "macos")]
pub mod error;

#[cfg(target_os = "macos")]
pub mod session;

#[cfg(target_os = "macos")]
pub mod surface;

#[cfg(target_os = "macos")]
pub use error::VtError;

#[cfg(target_os = "macos")]
pub use session::{
    copy_pixel_buffer_bgra, get_pixel_buffer_dimensions, CompressionSession, DecompressionSession,
    VideoCodec,
};

#[cfg(target_os = "macos")]
pub use surface::IoSurface;
