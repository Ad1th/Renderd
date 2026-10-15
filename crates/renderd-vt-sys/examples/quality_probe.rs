//! Encodes a scripted run of frames through the production encoder settings
//! and writes what came out, so picture quality and frame sizes can be
//! measured offline.
//!
//! ```text
//! cargo run -p renderd-vt-sys --example quality_probe -- \
//!     <width> <height> <hevc|h264> <kbps> <fps> <script> <out-dir>
//! ```
//!
//! Each line of `<script>` is `<frame.bgra> <pts_ms> <sleep_ms> [key]`: a raw
//! BGRA frame of `width` × `height`, its presentation time, how long to wait
//! after submitting it, and whether to force a keyframe. `<out-dir>` gets
//! `stream.<codec>` (Annex B, decodable with ffmpeg) and `frames.csv` with one
//! row per submitted frame: whether it came out, its size, and whether it was
//! a keyframe.
#![cfg_attr(not(target_os = "macos"), allow(unused))]
#![allow(unsafe_code)]

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("quality_probe needs VideoToolbox (macOS)");
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), String> {
    macos::run()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use core_foundation::base::TCFType;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use renderd_vt_sys::{CompressionSession, IoSurface, VideoCodec, ENCODE_INFO_FRAME_DROPPED};

    extern "C" {
        fn IOSurfaceCreate(properties: *const std::ffi::c_void) -> *const std::ffi::c_void;
        fn IOSurfaceLock(buffer: *const std::ffi::c_void, options: u32, seed: *mut u32) -> i32;
        fn IOSurfaceUnlock(buffer: *const std::ffi::c_void, options: u32, seed: *mut u32) -> i32;
        fn IOSurfaceGetBaseAddress(buffer: *const std::ffi::c_void) -> *mut u8;
        fn IOSurfaceGetBytesPerRow(buffer: *const std::ffi::c_void) -> usize;
    }

    /// One encoder output, in submission order.
    struct Output {
        landed: bool,
        bytes: usize,
        keyframe: bool,
    }

    struct Step {
        frame: String,
        pts_ms: f64,
        sleep_ms: u64,
        key: bool,
    }

    fn surface_from_file(path: &str, width: usize, height: usize) -> Result<IoSurface, String> {
        let pixels = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        if pixels.len() != width * height * 4 {
            return Err(format!("{path}: expected {width}x{height} BGRA"));
        }
        let pairs = [
            (
                "IOSurfaceWidth",
                CFNumber::from(i32::try_from(width).unwrap_or(0)),
            ),
            (
                "IOSurfaceHeight",
                CFNumber::from(i32::try_from(height).unwrap_or(0)),
            ),
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
        let surface = unsafe { IoSurface::from_raw(raw) }.ok_or("IOSurfaceCreate failed")?;
        // SAFETY: the surface is locked while its rows are written in bounds.
        unsafe {
            IOSurfaceLock(raw, 0, std::ptr::null_mut());
            let base = IOSurfaceGetBaseAddress(raw);
            let stride = IOSurfaceGetBytesPerRow(raw);
            for y in 0..height {
                let row = std::slice::from_raw_parts_mut(base.add(y * stride), width * 4);
                row.copy_from_slice(&pixels[y * width * 4..(y + 1) * width * 4]);
            }
            IOSurfaceUnlock(raw, 0, std::ptr::null_mut());
        }
        Ok(surface)
    }

    fn parse_script(path: &str) -> Result<Vec<Step>, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        text.lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|line| {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() < 3 {
                    return Err(format!("bad script line: {line}"));
                }
                Ok(Step {
                    frame: f[0].to_string(),
                    pts_ms: f[1].parse().map_err(|_| format!("bad pts: {line}"))?,
                    sleep_ms: f[2].parse().map_err(|_| format!("bad sleep: {line}"))?,
                    key: f.get(3) == Some(&"key"),
                })
            })
            .collect()
    }

    pub fn run() -> Result<(), String> {
        let args: Vec<String> = std::env::args().collect();
        if args.len() != 8 {
            return Err(
                "usage: quality_probe <width> <height> <hevc|h264> <kbps> <fps> <script> <out-dir>"
                    .into(),
            );
        }
        let width: usize = args[1].parse().map_err(|_| "bad width")?;
        let height: usize = args[2].parse().map_err(|_| "bad height")?;
        let codec = match args[3].as_str() {
            "h264" => VideoCodec::H264,
            "hevc" => VideoCodec::Hevc,
            other => return Err(format!("unknown codec {other}")),
        };
        let kbps: u32 = args[4].parse().map_err(|_| "bad kbps")?;
        let fps: u32 = args[5].parse().map_err(|_| "bad fps")?;
        let steps = parse_script(&args[6])?;
        let out_dir = std::path::PathBuf::from(&args[7]);
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

        let outputs = Arc::new(Mutex::new(Vec::<Output>::new()));
        let stream = Arc::new(Mutex::new(Vec::<u8>::new()));
        let (sink, stream_sink) = (Arc::clone(&outputs), Arc::clone(&stream));
        let session = CompressionSession::with_frame_rate(
            i32::try_from(width).unwrap_or(0),
            i32::try_from(height).unwrap_or(0),
            codec,
            kbps,
            fps,
            move |err, flags, sample| {
                let mut out = Output {
                    landed: false,
                    bytes: 0,
                    keyframe: false,
                };
                if err.code() == 0 && !sample.is_null() && flags & ENCODE_INFO_FRAME_DROPPED == 0 {
                    // SAFETY: VideoToolbox hands the callback a valid sample buffer.
                    if let Ok((nals, key)) =
                        unsafe { renderd_vt_sys::sample_buffer_extract_nals(sample) }
                    {
                        out = Output {
                            landed: true,
                            bytes: nals.len(),
                            keyframe: key,
                        };
                        stream_sink.lock().unwrap().extend_from_slice(&nals);
                    }
                }
                sink.lock().unwrap().push(out);
            },
        )
        .map_err(|e| format!("encoder: {e}"))?;
        eprintln!("low-latency rate control: {}", session.is_low_latency());

        for step in &steps {
            let surface = surface_from_file(&step.frame, width, height)?;
            #[allow(clippy::cast_possible_truncation)]
            let pts_ns = (step.pts_ms * 1_000_000.0) as i64;
            session
                .encode_surface(&surface, pts_ns, step.key)
                .map_err(|e| format!("encode {}: {e}", step.frame))?;
            std::thread::sleep(std::time::Duration::from_millis(step.sleep_ms));
        }
        drop(session);

        let ext = if args[3] == "h264" { "h264" } else { "hevc" };
        std::fs::write(
            out_dir.join(format!("stream.{ext}")),
            &*stream.lock().unwrap(),
        )
        .map_err(|e| e.to_string())?;
        let mut csv =
            std::fs::File::create(out_dir.join("frames.csv")).map_err(|e| e.to_string())?;
        writeln!(csv, "index,frame,pts_ms,landed,bytes,keyframe").map_err(|e| e.to_string())?;
        for (i, (step, out)) in steps.iter().zip(outputs.lock().unwrap().iter()).enumerate() {
            writeln!(
                csv,
                "{i},{},{},{},{},{}",
                step.frame,
                step.pts_ms,
                u8::from(out.landed),
                out.bytes,
                u8::from(out.keyframe)
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
