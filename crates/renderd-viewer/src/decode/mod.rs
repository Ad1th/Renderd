//! Direct3D 12 video decoding subsystem module (`renderd-viewer/src/decode/`).

pub mod d3d12_decode;
pub mod videotoolbox_decode;

pub use d3d12_decode::D3D12Decoder;
pub use videotoolbox_decode::VideoToolboxDecoder;

/// Codecs this build can decode, most preferred first.
///
/// The host picks the first entry it can encode, so the order here is what actually
/// decides the wire format. On Windows H.264 leads deliberately: the Media Foundation
/// H.264 decoder ships with every Windows 10 and later install, whereas HEVC decoding
/// needs the HEVC Video Extensions from the Store and is absent on a stock machine.
/// Offering HEVC first there produces a stream that connects and then shows nothing.
#[must_use]
pub fn preferred_codecs() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        vec!["h264".to_string(), "hevc".to_string()]
    }
    #[cfg(not(target_os = "windows"))]
    {
        vec!["hevc".to_string(), "h264".to_string()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preferred_codecs_are_negotiable_and_ordered() {
        let codecs = preferred_codecs();
        assert_eq!(codecs.len(), 2, "both codecs must be offered as a fallback");
        for codec in &codecs {
            assert!(
                codec == "h264" || codec == "hevc",
                "offered codec must be one the host can encode: {codec}"
            );
        }
        if cfg!(target_os = "windows") {
            assert_eq!(
                codecs[0], "h264",
                "Windows must lead with the codec that always has a decoder present"
            );
        }
    }
}
