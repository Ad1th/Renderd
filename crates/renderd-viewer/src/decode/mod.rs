//! Direct3D 12 video decoding subsystem module (`renderd-viewer/src/decode/`).

pub mod d3d12_decode;
pub mod videotoolbox_decode;

pub use d3d12_decode::D3D12Decoder;
pub use videotoolbox_decode::VideoToolboxDecoder;
