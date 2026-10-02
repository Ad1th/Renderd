//! What the low-latency rate controller does with a big change on a slow link.
//!
//! The host re-submits the last captured surface once the screen goes still
//! (`renderd-host::refine`). Measured here: at 2 Mbps a full-screen change of
//! 1080p text comes out as one frame of tens of kilobytes, and the encoder
//! then *drops* the frames after it until its budget has recovered. A screen
//! that stops moving inside that window would leave the viewer on a stale
//! frame. Re-submitting the still surface, spaced out, gets it encoded once
//! the budget allows — which is what the refiner relies on.
#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use std::sync::{Arc, Mutex};

use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use renderd_vt_sys::{CompressionSession, IoSurface, VideoCodec};

extern "C" {
    fn IOSurfaceCreate(properties: *const std::ffi::c_void) -> *const std::ffi::c_void;
    fn IOSurfaceLock(buffer: *const std::ffi::c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(buffer: *const std::ffi::c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceGetBaseAddress(buffer: *const std::ffi::c_void) -> *mut u8;
    fn IOSurfaceGetBytesPerRow(buffer: *const std::ffi::c_void) -> usize;
}

/// A BGRA surface full of text-like detail: thin dark strokes on a light page.
/// `shift` moves the page, the way a scroll does.
fn text_surface(width: i32, height: i32, shift: usize) -> IoSurface {
    let pairs = [
        ("IOSurfaceWidth", CFNumber::from(width)),
        ("IOSurfaceHeight", CFNumber::from(height)),
        ("IOSurfaceBytesPerElement", CFNumber::from(4)),
        ("IOSurfacePixelFormat", CFNumber::from(0x4247_5241_i32)),
    ];
    let pairs: Vec<_> = pairs
        .iter()
        .map(|(k, v)| (CFString::new(k).as_CFType(), v.as_CFType()))
        .collect();
    let dict = CFDictionary::from_CFType_pairs(&pairs);
    // SAFETY: dict is a valid CFDictionary of IOSurface properties.
    let raw = unsafe { IOSurfaceCreate(dict.as_concrete_TypeRef().cast()) };
    // SAFETY: raw is a fresh +1 IOSurfaceRef (or null).
    let surface = unsafe { IoSurface::from_raw(raw) }.expect("IOSurfaceCreate");
    // SAFETY: the surface is locked while its rows are written in bounds.
    unsafe {
        IOSurfaceLock(raw, 0, std::ptr::null_mut());
        let base = IOSurfaceGetBaseAddress(raw);
        let stride = IOSurfaceGetBytesPerRow(raw);
        let (w, h) = (
            usize::try_from(width).unwrap(),
            usize::try_from(height).unwrap(),
        );
        let mut seed = 0x9E37_79B9_u32;
        for y in 0..h {
            let row = std::slice::from_raw_parts_mut(base.add(y * stride), w * 4);
            let ys = y + shift;
            for x in 0..w {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                // Lines of "glyphs": 12 px rows, 8 px cells, ragged strokes.
                let in_line = ys % 12 < 9;
                let glyph = (x / 8 + ys / 12) % 5 != 0;
                let stroke = glyph && in_line && (x % 8 < 2 || (ys % 12 == 4 && seed % 3 == 0));
                let v = if stroke { 30 } else { 235 };
                row[x * 4..x * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        IOSurfaceUnlock(raw, 0, std::ptr::null_mut());
    }
    surface
}

#[test]
#[ignore = "Requires hardware VideoToolbox (unavailable in virtualized CI)"]
fn test_still_frame_lands_after_the_encoder_repays_a_burst() {
    let sizes = Arc::new(Mutex::new(Vec::<usize>::new()));
    let sink = Arc::clone(&sizes);
    let session = CompressionSession::with_frame_rate(
        1920,
        1080,
        VideoCodec::Hevc,
        2_000,
        60,
        move |err, _flags, sample| {
            if err.code() == 0 && !sample.is_null() {
                // SAFETY: VideoToolbox hands the callback a valid sample buffer.
                if let Ok((nals, _)) = unsafe { renderd_vt_sys::sample_buffer_extract_nals(sample) }
                {
                    sink.lock().unwrap().push(nals.len());
                }
            }
        },
    )
    .expect("hardware encoder");

    // A page, then — once the encoder has paid off its keyframe — a scroll
    // that replaces every line, then the scrolled page sitting still.
    let page = text_surface(1920, 1080, 0);
    let scrolled = text_surface(1920, 1080, 7);
    let frame_ns = 16_666_667i64;
    let still_from = 4_000_000_000i64;
    session.encode_surface(&page, 0, true).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    for i in 0..8i64 {
        session
            .encode_surface(&scrolled, still_from + i * 9 * frame_ns, false)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(session);

    let sizes = sizes.lock().unwrap().clone();
    println!("2 Mbps 1080p text: keyframe, scrolled page, then re-encodes of it: {sizes:?}");
    assert!(sizes.len() >= 2, "no frame after the keyframe: {sizes:?}");
    assert!(
        sizes.len() < 9,
        "expected the rate controller to drop re-encodes after a large frame: {sizes:?}"
    );
    assert!(
        sizes.len() >= 3,
        "a re-encode of the still frame should land once the budget recovers: {sizes:?}"
    );
}
