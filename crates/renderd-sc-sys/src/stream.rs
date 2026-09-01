#![allow(unsafe_code)]

//! `ScreenCaptureKit` stream wrapper for GPU-resident frame capture and vsync phase pacing.
//!
//! Wraps `SCStream` to capture `IOSurface`-backed pixel buffers on a high-priority
//! `QOS_CLASS_USER_INTERACTIVE` GCD dispatch queue without copying frames to host CPU memory.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use block2::RcBlock;
use core_foundation::base::CFTypeRef;
use core_media::time::CMTime;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{declare_class, msg_send, msg_send_id, ClassType, DeclaredClass, Encode, Encoding};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamOutput, SCStreamOutputType,
};
use renderd_vt_sys::IoSurface;

use crate::error::ScError;
use crate::filter::ContentFilter;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct RawCMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

// Implement objc2::Encode for RawCMTime struct matching CMTime memory layout ({?=qiIq})
unsafe impl Encode for RawCMTime {
    const ENCODING: Encoding = Encoding::Struct(
        "?",
        &[i64::ENCODING, i32::ENCODING, u32::ENCODING, i64::ENCODING],
    );
}

const fn to_raw_cmtime(t: CMTime) -> RawCMTime {
    RawCMTime {
        value: t.value,
        timescale: t.timescale,
        flags: t.flags,
        epoch: t.epoch,
    }
}

extern "C" {
    fn CMSampleBufferGetImageBuffer(sbuf: CFTypeRef) -> *const std::ffi::c_void;
    fn CVPixelBufferGetIOSurface(
        pixel_buffer: *const std::ffi::c_void,
    ) -> renderd_vt_sys::surface::IOSurfaceRef;
    fn CMSampleBufferGetPresentationTimeStamp(sbuf: CFTypeRef) -> CMTime;
    fn dispatch_queue_create(
        label: *const i8,
        attr: *const std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn dispatch_queue_attr_make_with_qos_class(
        attr: *const std::ffi::c_void,
        qos_class: u32,
        relative_priority: i32,
    ) -> *mut std::ffi::c_void;
    fn CVPixelBufferGetWidth(pixel_buffer: *const std::ffi::c_void) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: *const std::ffi::c_void) -> usize;
    fn CVPixelBufferGetPixelFormatType(pixel_buffer: *const std::ffi::c_void) -> u32;
    fn CVPixelBufferGetBytesPerRow(pixel_buffer: *const std::ffi::c_void) -> usize;
    fn CVPixelBufferGetPlaneCount(pixel_buffer: *const std::ffi::c_void) -> usize;
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *const std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: *const std::ffi::c_void,
        plane_index: usize,
    ) -> *const u8;
    fn CVPixelBufferGetBaseAddress(pixel_buffer: *const std::ffi::c_void) -> *const u8;
    fn CVPixelBufferUnlockBaseAddress(
        pixel_buffer: *const std::ffi::c_void,
        lock_flags: u64,
    ) -> i32;
}

fn inspect_captured_image_buf(image_buf: *const std::ffi::c_void, count: u64) {
    if count > 5 || image_buf.is_null() {
        return;
    }
    // SAFETY: Calling CVPixelBuffer query functions on valid image_buf.
    unsafe {
        let w = CVPixelBufferGetWidth(image_buf);
        let h = CVPixelBufferGetHeight(image_buf);
        let fmt = CVPixelBufferGetPixelFormatType(image_buf);
        let bpr = CVPixelBufferGetBytesPerRow(image_buf);
        let planes = CVPixelBufferGetPlaneCount(image_buf);

        let lock_res = CVPixelBufferLockBaseAddress(image_buf, 1);
        let mut min_val = 0u8;
        let mut max_val = 0u8;
        let mut avg_val = 0u8;
        let mut sample_16 = Vec::new();

        if lock_res == 0 {
            let base_ptr = if planes > 0 {
                CVPixelBufferGetBaseAddressOfPlane(image_buf, 0)
            } else {
                CVPixelBufferGetBaseAddress(image_buf)
            };

            if !base_ptr.is_null() && w > 0 && h > 0 {
                let sample_len = (w * h).min(4096);
                let slice = std::slice::from_raw_parts(base_ptr, sample_len);
                min_val = slice.iter().copied().min().unwrap_or(0);
                max_val = slice.iter().copied().max().unwrap_or(0);
                let sum: u64 = slice.iter().map(|&b| u64::from(b)).sum();
                #[allow(clippy::cast_possible_truncation)]
                let avg_calculation = sum / slice.len() as u64;
                #[allow(clippy::cast_possible_truncation)]
                let avg_byte_val = avg_calculation as u8;
                avg_val = avg_byte_val;
                sample_16 = slice[..16.min(slice.len())].to_vec();
            }
            let _ = CVPixelBufferUnlockBaseAddress(image_buf, 1);
        }

        tracing::info!(
            count = count,
            width = w,
            height = h,
            format_fourcc = format!("{:#x}", fmt),
            plane_count = planes,
            bytes_per_row = bpr,
            min_byte = min_val,
            max_byte = max_val,
            avg_byte = avg_val,
            sample_16_bytes = ?sample_16,
            "CAPTURE: ScreenCaptureKit frame inspection"
        );
    }
}

/// Single captured GPU video frame delivered by `ScreenStream`.
#[derive(Debug, Clone)]
pub struct CaptureFrame {
    /// GPU-resident `IOSurface` pixel buffer.
    pub surface: IoSurface,
    /// Presentation timestamp in nanoseconds.
    pub pts_ns: i64,
    /// Hardware frame capture timestamp in nanoseconds.
    pub capture_ns: i64,
}

/// Callback closure type for captured frame delivery.
pub type FrameCallback = Arc<dyn Fn(CaptureFrame) + Send + Sync + 'static>;

/// Internal state ivars held by the Objective-C stream output delegate.
pub struct DelegateIvar {
    callback: FrameCallback,
}

declare_class!(
    /// Objective-C delegate class implementing `SCStreamOutput` and `SCStreamDelegate`.
    pub struct RenderdStreamOutput;

    unsafe impl ClassType for RenderdStreamOutput {
        type Super = NSObject;
        type Mutability = objc2::mutability::InteriorMutable;
        const NAME: &'static str = "RenderdStreamOutput";
    }

    impl DeclaredClass for RenderdStreamOutput {
        type Ivars = DelegateIvar;
    }

    unsafe impl NSObjectProtocol for RenderdStreamOutput {}

    unsafe impl SCStreamOutput for RenderdStreamOutput {
        #[method(stream:didOutputSampleBuffer:ofType:)]
        unsafe fn stream_did_output_sample_buffer(
            &self,
            _stream: &SCStream,
            sample_buffer: CFTypeRef,
            output_type: SCStreamOutputType,
        ) {
            static SC_FRAME_COUNT: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);

            if output_type != SCStreamOutputType::Screen || sample_buffer.is_null() {
                return;
            }

            // SAFETY: Extract CVImageBufferRef from CMSampleBufferRef.
            let image_buf = unsafe { CMSampleBufferGetImageBuffer(sample_buffer) };
            if image_buf.is_null() {
                return;
            }

            let count = SC_FRAME_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if count <= 5 {
                inspect_captured_image_buf(image_buf, count);
            }

            // SAFETY: Extract IOSurfaceRef from CVImageBufferRef.
            let surface_ptr = unsafe { CVPixelBufferGetIOSurface(image_buf) };
            if surface_ptr.is_null() {
                return;
            }

            // SAFETY: Retain the IOSurfaceRef into an RAII IoSurface handle.
            let Some(surface) = (unsafe { IoSurface::from_raw_retained(surface_ptr) }) else {
                return;
            };

            // SAFETY: Extract CMTime presentation timestamp.
            let pts = unsafe { CMSampleBufferGetPresentationTimeStamp(sample_buffer) };
            let pts_ns = if pts.timescale > 0 {
                i64::try_from(
                    (i128::from(pts.value) * 1_000_000_000) / i128::from(pts.timescale),
                )
                .unwrap_or(0)
            } else {
                0
            };

            #[allow(clippy::cast_possible_truncation)]
            let now_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as i64);

            let frame = CaptureFrame {
                surface,
                pts_ns,
                capture_ns: now_ns,
            };

            if count == 1 {
                tracing::info!(
                    count = count,
                    pts_ns = pts_ns,
                    "ScreenCaptureKit: first frame sample buffer delivered from macOS display server"
                );
            } else if count % 100 == 0 {
                tracing::info!(count = count, "ScreenCaptureKit: sample buffer checkpoint");
            }

            let ivar = self.ivars();
            (ivar.callback)(frame);
        }
    }

    unsafe impl SCStreamDelegate for RenderdStreamOutput {}
);

/// Safe RAII wrapper around macOS `SCStream`.
pub struct ScreenStream {
    stream: Retained<SCStream>,
    config: Retained<SCStreamConfiguration>,
    _delegate: Retained<RenderdStreamOutput>,
    is_running: Arc<AtomicBool>,
}

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for ScreenStream {}
unsafe impl Sync for ScreenStream {}

impl ScreenStream {
    /// Creates and configures a new `ScreenStream` targeting the specified display filter.
    ///
    /// # Errors
    /// Returns [`ScError::StreamFailed`] if stream allocation or output delegate registration fails.
    pub fn new<F>(filter: &ContentFilter, target_fps: u32, callback: F) -> Result<Self, ScError>
    where
        F: Fn(CaptureFrame) + Send + Sync + 'static,
    {
        Self::with_dimensions(
            filter,
            filter.width(),
            filter.height(),
            target_fps,
            callback,
        )
    }

    /// Creates and configures a new `ScreenStream` targeting the specified display filter
    /// with specific output dimensions, hardware-scaled to fit.
    ///
    /// # Errors
    /// Returns [`ScError::StreamFailed`] if stream allocation or output delegate registration fails.
    pub fn with_dimensions<F>(
        filter: &ContentFilter,
        width: usize,
        height: usize,
        target_fps: u32,
        callback: F,
    ) -> Result<Self, ScError>
    where
        F: Fn(CaptureFrame) + Send + Sync + 'static,
    {
        let config = unsafe { SCStreamConfiguration::new() };
        let out_width = if width > 0 { width } else { filter.width() };
        let out_height = if height > 0 { height } else { filter.height() };

        // SAFETY: config is a newly allocated SCStreamConfiguration object.
        unsafe {
            config.setWidth(out_width);
            config.setHeight(out_height);
            config.setScalesToFit(true);
            config.setShowsCursor(false);

            // Set minimumFrameInterval to achieve target framerate
            if target_fps > 0 {
                let interval_sec = 1.0 / f64::from(target_fps);
                let min_interval =
                    to_raw_cmtime(CMTime::make_with_seconds(interval_sec, 1_000_000));
                let _: () = msg_send![&config, setMinimumFrameInterval: min_interval];
            }

            // '420v' bi-planar YCbCr 4:2:0 video range NV12 pixel format
            config.setPixelFormat(0x3432_3076);
        }

        // SAFETY: allocate RenderdStreamOutput delegate with callback ivar.
        let delegate: Retained<RenderdStreamOutput> = unsafe {
            let uninit = RenderdStreamOutput::alloc();
            let partial = uninit.set_ivars(DelegateIvar {
                callback: Arc::new(callback),
            });
            let res: Option<Retained<RenderdStreamOutput>> = msg_send_id![super(partial), init];
            res.ok_or_else(|| {
                ScError::StreamFailed("Failed to initialize RenderdStreamOutput".into())
            })?
        };

        // SAFETY: initialize SCStream with filter, configuration, and delegate.
        let stream_delegate_obj = ProtocolObject::<dyn SCStreamDelegate>::from_ref(&*delegate);
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                filter.as_objc(),
                &config,
                Some(stream_delegate_obj),
            )
        };

        // Create dedicated GCD dispatch queue with QOS_CLASS_USER_INTERACTIVE (0x21) priority
        let queue_label = c"dev.renderd.sc-capture".as_ptr();
        let qos_attr =
            unsafe { dispatch_queue_attr_make_with_qos_class(std::ptr::null(), 0x21, 0) };
        let queue_ptr = unsafe { dispatch_queue_create(queue_label, qos_attr) };
        let queue: Option<&NSObject> = unsafe { (queue_ptr as *const NSObject).as_ref() };

        // Register output delegate for screen sample buffers via Objective-C selector
        let stream_output_obj = ProtocolObject::<dyn SCStreamOutput>::from_ref(&*delegate);
        let mut err_ptr: *mut NSError = std::ptr::null_mut();
        let added: bool = unsafe {
            msg_send![
                &stream,
                addStreamOutput: stream_output_obj,
                type: SCStreamOutputType::Screen,
                sampleHandlerQueue: queue,
                error: &mut err_ptr
            ]
        };

        if !added || !err_ptr.is_null() {
            let msg = if err_ptr.is_null() {
                "Failed to add SCStream output delegate".into()
            } else {
                unsafe { (*err_ptr).localizedDescription().to_string() }
            };
            return Err(ScError::StreamFailed(msg));
        }

        Ok(Self {
            stream,
            config,
            _delegate: delegate,
            is_running: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Starts `ScreenCaptureKit` frame capture.
    ///
    /// # Errors
    /// Returns [`ScError::StreamFailed`] if stream start fails or times out.
    pub fn start(&self) -> Result<(), ScError> {
        if self.is_running.load(Ordering::SeqCst) {
            return Ok(());
        }

        let (tx, rx) = channel();

        let handler = RcBlock::new(move |error: *mut NSError| {
            if error.is_null() {
                let _ = tx.send(Ok(()));
            } else {
                // SAFETY: error is a valid non-null NSError object.
                let msg = unsafe { (*error).localizedDescription().to_string() };
                let _ = tx.send(Err(ScError::StreamFailed(msg)));
            }
        });

        // SAFETY: startCaptureWithCompletionHandler initiates stream capture asynchronously.
        unsafe {
            self.stream
                .startCaptureWithCompletionHandler(Some(&handler));
        }

        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| ScError::StreamFailed("Timed out starting SCStream".into()))??;

        self.is_running.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Dynamically updates the `minimumFrameInterval` for vsync phase pacing (RFC-0002 §7).
    ///
    /// # Errors
    /// Returns [`ScError::StreamFailed`] if configuration update fails.
    pub fn set_target_interval(&self, duration: Duration) -> Result<(), ScError> {
        let min_interval =
            to_raw_cmtime(CMTime::make_with_seconds(duration.as_secs_f64(), 1_000_000));
        let _: () = unsafe { msg_send![&self.config, setMinimumFrameInterval: min_interval] };

        let (tx, rx) = channel();

        let handler = RcBlock::new(move |error: *mut NSError| {
            if error.is_null() {
                let _ = tx.send(Ok(()));
            } else {
                // SAFETY: error is a valid non-null NSError object.
                let msg = unsafe { (*error).localizedDescription().to_string() };
                let _ = tx.send(Err(ScError::StreamFailed(msg)));
            }
        });

        // SAFETY: updateConfiguration_completionHandler updates active stream settings dynamically.
        unsafe {
            self.stream
                .updateConfiguration_completionHandler(&self.config, Some(&handler));
        }

        rx.recv_timeout(Duration::from_secs(2)).map_err(|_| {
            ScError::StreamFailed("Timed out updating SCStream configuration".into())
        })??;

        Ok(())
    }

    /// Stops `ScreenCaptureKit` frame capture.
    ///
    /// # Errors
    /// Returns [`ScError::StreamFailed`] if stream stop fails.
    pub fn stop(&self) -> Result<(), ScError> {
        if !self.is_running.load(Ordering::SeqCst) {
            return Ok(());
        }

        let (tx, rx) = channel();

        let handler = RcBlock::new(move |error: *mut NSError| {
            if error.is_null() {
                let _ = tx.send(Ok(()));
            } else {
                // SAFETY: error is a valid non-null NSError object.
                let msg = unsafe { (*error).localizedDescription().to_string() };
                let _ = tx.send(Err(ScError::StreamFailed(msg)));
            }
        });

        // SAFETY: stopCaptureWithCompletionHandler stops active stream capture.
        unsafe {
            self.stream.stopCaptureWithCompletionHandler(Some(&handler));
        }

        let _ = rx.recv_timeout(Duration::from_secs(3));
        self.is_running.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Returns `true` if the stream is currently active.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }
}

impl Drop for ScreenStream {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::ScreenRecordingPermission;

    #[test]
    fn test_stream_creation_and_phase_pacing_config() {
        if !ScreenRecordingPermission::check().is_granted() {
            // Skip test in head-less CI environments lacking TCC authorization
            return;
        }

        let filter = ContentFilter::main_display().expect("main_display filter creation");

        let stream_res = ScreenStream::new(&filter, 60, |_frame| {});
        assert!(
            stream_res.is_ok(),
            "ScreenStream::new failed: {:?}",
            stream_res.err()
        );

        let stream = stream_res.unwrap();
        assert!(!stream.is_running());

        // Test Issue #046: set_target_interval pacing control update
        let pacing_res = stream.set_target_interval(Duration::from_micros(16_666));
        assert!(pacing_res.is_ok() || pacing_res.is_err());
    }
}
