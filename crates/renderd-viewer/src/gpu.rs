//! Shared GPU objects for hardware decode and presentation (`renderd-viewer/src/gpu.rs`).
//!
//! On Windows the viewer's fast path keeps a frame on the GPU from the moment the
//! hardware decoder writes it until the swapchain shows it: Media Foundation
//! decodes into a D3D11 texture, and the presenter's video processor converts
//! and scales that same texture straight into the back buffer. Both sides must
//! therefore share one D3D11 device, which is what [`D3d11Context`] is.
//!
//! [`GpuSurface`] is a decoded frame that lives in that device's memory. It holds
//! the decoder's output sample, which keeps the decoder from recycling the
//! surface until the presenter has drawn it.

#![allow(unsafe_code)]

/// A decoded frame resident in GPU memory.
///
/// Carries the decoder's output texture array and the slice holding this frame,
/// plus the Media Foundation sample that owns it. Dropping the surface returns
/// the slice to the decoder's pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuSurface {
    #[cfg(target_os = "windows")]
    pub(crate) texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    #[cfg(target_os = "windows")]
    pub(crate) array_index: u32,
    #[cfg(target_os = "windows")]
    pub(crate) _sample: windows::Win32::Media::MediaFoundation::IMFSample,
    #[cfg(not(target_os = "windows"))]
    _unused: (),
}

// SAFETY: D3D11 resources and Media Foundation samples are free-threaded COM
// objects. The device that owns them is put in multithread-protected mode by
// `D3d11Context::new`, so the decoder thread producing a surface and the UI
// thread presenting it may both touch it.
#[cfg(target_os = "windows")]
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for GpuSurface {}
#[cfg(target_os = "windows")]
unsafe impl Sync for GpuSurface {}

#[cfg(target_os = "windows")]
pub use windows_impl::D3d11Context;

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Multithread, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Dxgi::IDXGIDevice1;
    use windows::Win32::Media::MediaFoundation::{IMFDXGIDeviceManager, MFCreateDXGIDeviceManager};

    use crate::error::ViewerError;

    /// One D3D11 device shared by the hardware decoder and the presenter.
    #[derive(Debug)]
    pub struct D3d11Context {
        pub(crate) device_manager: IMFDXGIDeviceManager,
        gpu_frames: AtomicBool,
    }

    // SAFETY: the device is created multithread-protected below, which makes the
    // immediate context safe to call from the decoder thread and the UI thread;
    // the DXGI device manager is itself designed for cross-thread use.
    #[allow(clippy::non_send_fields_in_send_ty)]
    unsafe impl Send for D3d11Context {}
    unsafe impl Sync for D3d11Context {}

    impl D3d11Context {
        /// Creates a hardware D3D11 device with video support and a Media
        /// Foundation device manager wrapping it.
        ///
        /// # Errors
        /// Returns [`ViewerError::Renderer`] if there is no suitable GPU or any
        /// of the device objects cannot be created.
        pub fn new() -> Result<Arc<Self>, ViewerError> {
            let err = |what: &str, e: windows::core::Error| {
                ViewerError::Renderer(format!("{what} failed: {e}"))
            };
            // SAFETY: plain D3D11/MF object creation with valid out-pointers.
            unsafe {
                let mut device = None;
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    None,
                    D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    None,
                )
                .map_err(|e| err("D3D11CreateDevice", e))?;
                let device = device
                    .ok_or_else(|| ViewerError::Renderer("D3D11CreateDevice: no device".into()))?;
                // The decoder (on a tokio worker) and the presenter (on the UI
                // thread) share the immediate context.
                let multithread: ID3D11Multithread =
                    device.cast().map_err(|e| err("ID3D11Multithread", e))?;
                let _ = multithread.SetMultithreadProtected(true);

                // Never let the driver queue more than one frame ahead of the
                // display: every queued frame is a frame of latency.
                if let Ok(dxgi) = device.cast::<IDXGIDevice1>() {
                    let _ = dxgi.SetMaximumFrameLatency(1);
                }

                let mut reset_token = 0u32;
                let mut manager = None;
                MFCreateDXGIDeviceManager(&mut reset_token, &mut manager)
                    .map_err(|e| err("MFCreateDXGIDeviceManager", e))?;
                let device_manager = manager.ok_or_else(|| {
                    ViewerError::Renderer("MFCreateDXGIDeviceManager: no manager".into())
                })?;
                device_manager
                    .ResetDevice(&device, reset_token)
                    .map_err(|e| err("IMFDXGIDeviceManager::ResetDevice", e))?;

                Ok(Arc::new(Self {
                    device_manager,
                    gpu_frames: AtomicBool::new(false),
                }))
            }
        }

        /// Whether decoded frames should stay on the GPU.
        ///
        /// Off until something that can show a GPU frame says so: the software
        /// renderer needs frames in system memory, so without a GPU presenter
        /// the decoder still decodes on the GPU but reads each frame back.
        #[must_use]
        pub fn gpu_frames(&self) -> bool {
            self.gpu_frames.load(Ordering::Relaxed)
        }

        /// Sets whether the decoder hands out GPU-resident frames. Takes effect
        /// from the next decoded frame.
        pub fn set_gpu_frames(&self, enabled: bool) {
            self.gpu_frames.store(enabled, Ordering::Relaxed);
        }
    }
}
