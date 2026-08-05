//! DXGI capability detection for variable refresh rate and low-latency tearing present flags.

#![allow(unsafe_code)]

/// Queries whether the host display driver and GPU support DXGI variable refresh rate tearing (`DXGI_FEATURE_PRESENT_ALLOW_TEARING`).
///
/// On Windows systems, this queries `IDXGIFactory5::CheckFeatureSupport`.
/// On non-Windows platforms, this function safely returns `false`.
#[must_use]
#[allow(clippy::missing_const_for_fn)]
pub fn check_tearing_support() -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::core::Interface;
        use windows::Win32::Foundation::BOOL;
        use windows::Win32::Graphics::Dxgi::{
            CreateDXGIFactory1, IDXGIFactory1, IDXGIFactory5, DXGI_FEATURE_PRESENT_ALLOW_TEARING,
        };

        // SAFETY: CreateDXGIFactory1 and CheckFeatureSupport call standard DXGI COM interfaces.
        unsafe {
            let factory_res = CreateDXGIFactory1::<IDXGIFactory1>();
            let Ok(factory1) = factory_res else {
                return false;
            };

            let Ok(factory5) = factory1.cast::<IDXGIFactory5>() else {
                return false;
            };

            let mut allow_tearing = BOOL(0);
            let size = std::mem::size_of::<BOOL>() as u32;

            if factory5
                .CheckFeatureSupport(
                    DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                    std::ptr::addr_of_mut!(allow_tearing).cast(),
                    size,
                )
                .is_ok()
            {
                allow_tearing.as_bool()
            } else {
                false
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_tearing_support_does_not_panic() {
        // Must execute cleanly without crashing on any platform.
        let _supported = check_tearing_support();
    }
}
