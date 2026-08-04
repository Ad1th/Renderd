//! `ScreenCaptureKit` macOS capture FFI wrapper for Renderd.
//!
//! This crate provides safe Rust bindings to Apple's `ScreenCaptureKit` framework,
//! enabling high-performance GPU-resident screen capture via `SCStream`.

#![cfg_attr(not(target_os = "macos"), allow(unused))]

pub mod error;

#[cfg(target_os = "macos")]
pub mod filter;

#[cfg(target_os = "macos")]
pub mod permission;

#[cfg(target_os = "macos")]
pub mod stream;

pub use error::ScError;

#[cfg(target_os = "macos")]
pub use filter::ContentFilter;

#[cfg(target_os = "macos")]
pub use permission::{PermissionStatus, ScreenRecordingPermission};

#[cfg(target_os = "macos")]
pub use stream::{CaptureFrame, FrameCallback, ScreenStream};
