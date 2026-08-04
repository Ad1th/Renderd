/// Build script for `renderd-vt-sys`.
///
/// On macOS, links the three Apple frameworks required by `VideoToolbox` and compiles
/// the C bridge shim (`c-shims/videotoolbox_shim.c`) once it is added in issue #037.
/// On non-macOS targets the script is a no-op, ensuring the crate remains compilable
/// in workspace-wide `cargo check` runs on Linux/Windows CI agents.
fn main() {
    #[cfg(target_os = "macos")]
    link_macos_frameworks();
}

#[cfg(target_os = "macos")]
fn link_macos_frameworks() {
    // VideoToolbox: hardware encode/decode API (VTCompressionSession, etc.)
    println!("cargo:rustc-link-lib=framework=VideoToolbox");
    // CoreMedia: CMSampleBufferRef, CMTime, CMBlockBufferRef
    println!("cargo:rustc-link-lib=framework=CoreMedia");
    // CoreFoundation: CFTypeRef, CFRetain, CFRelease, CFDictionaryRef, etc.
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    // IOSurface: GPU-resident surface handles (IOSurfaceRef)
    println!("cargo:rustc-link-lib=framework=IOSurface");

    // Re-run this build script only if the build script itself changes.
    // The C shim compilation step (added in #037) will add its own rerun-if-changed
    // directives once the shim file exists.
    println!("cargo:rerun-if-changed=build.rs");
}
