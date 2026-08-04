/// Build script for `renderd-vt-sys`.
///
/// On macOS, links the four Apple frameworks required by `VideoToolbox` and compiles
/// the C bridge shim (`c-shims/videotoolbox_shim.c`).
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
    // CoreVideo: CVPixelBufferRef, CVPixelBufferCreateWithIOSurface
    println!("cargo:rustc-link-lib=framework=CoreVideo");
    // CoreFoundation: CFTypeRef, CFRetain, CFRelease, CFDictionaryRef, etc.
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    // IOSurface: GPU-resident surface handles (IOSurfaceRef)
    println!("cargo:rustc-link-lib=framework=IOSurface");

    // Compile C bridge shim
    cc::Build::new()
        .file("c-shims/videotoolbox_shim.c")
        .flag("-Wall")
        .flag("-Wextra")
        .flag("-Werror")
        .flag("-std=c99")
        .compile("videotoolbox_shim");

    // Re-run this build script if build.rs or C shim sources change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=c-shims/videotoolbox_shim.c");
    println!("cargo:rerun-if-changed=c-shims/videotoolbox_shim.h");
}
