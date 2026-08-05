//! Direct3D 12 graphics rendering subsystem module (`renderd-viewer/src/render/`).

pub mod d3d12_renderer;
pub mod tearing_check;

pub use d3d12_renderer::D3D12Renderer;
pub use tearing_check::check_tearing_support;
