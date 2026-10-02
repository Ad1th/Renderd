//! Direct3D 12 graphics rendering subsystem module (`renderd-viewer/src/render/`).

#[cfg(target_os = "windows")]
pub mod d3d11_presenter;
pub mod d3d12_renderer;
pub mod tearing_check;

#[cfg(target_os = "windows")]
pub use d3d11_presenter::D3d11Presenter;

pub use d3d12_renderer::D3D12Renderer;
pub use tearing_check::check_tearing_support;
