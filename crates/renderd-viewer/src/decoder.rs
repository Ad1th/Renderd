//! Video decoder abstraction for converting compressed video packets into uncompressed frames.

use crate::error::ViewerError;
use std::fmt::Debug;
use std::time::Duration;

/// Pixel format representation of a decoded video frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PixelFormat {
    /// 32-bit BGRA uncompressed color format.
    #[default]
    Bgra8,
    /// NV12 bi-planar YUV 4:2:0 format.
    Nv12,
    /// P010 10-bit YUV 4:2:0 format.
    P010,
}

/// Uncompressed decoded video frame ready for rendering.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// Frame sequence identifier.
    pub frame_id: u64,
    /// Presentation timestamp in nanoseconds.
    pub pts_ns: u64,
    /// Frame width in physical pixels.
    pub width: u32,
    /// Frame height in physical pixels.
    pub height: u32,
    /// Pixel format of raw buffer.
    pub format: PixelFormat,
    /// Raw uncompressed image buffer bytes.
    pub buffer: Vec<u8>,
    /// Time spent by hardware decoder to decode this frame.
    pub decode_duration: Duration,
}

/// Trait abstraction for hardware video decoders (e.g. Windows Media Foundation / D3D11VA / NVDEC).
pub trait Decoder: Send + Sync {
    /// Initializes the hardware decoder with target codec and resolution parameters.
    ///
    /// # Errors
    /// Returns [`ViewerError::Decoder`] if initialization fails.
    fn initialize(&mut self, codec: &str, width: u32, height: u32) -> Result<(), ViewerError>;

    /// Submits a compressed video bitstream packet for decoding.
    ///
    /// # Errors
    /// Returns [`ViewerError::Decoder`] if packet submission fails.
    fn decode_packet(
        &mut self,
        packet: &[u8],
        frame_id: u64,
        pts_ns: u64,
    ) -> Result<(), ViewerError>;

    /// Fetches the next available [`DecodedFrame`] from the decoder output pipeline.
    ///
    /// # Errors
    /// Returns [`ViewerError::Decoder`] if output retrieval fails.
    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, ViewerError>;

    /// Resets the decoder pipeline, clearing all queued reference frames and bitstream buffers.
    ///
    /// # Errors
    /// Returns [`ViewerError::Decoder`] if pipeline reset fails.
    fn reset(&mut self) -> Result<(), ViewerError>;
}

/// Null / Mock implementation of [`Decoder`] for testing and headless execution.
#[derive(Debug, Default)]
pub struct NullDecoder {
    initialized: bool,
    width: u32,
    height: u32,
}

impl NullDecoder {
    /// Creates a new [`NullDecoder`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            initialized: false,
            width: 0,
            height: 0,
        }
    }

    /// Checks if the decoder has been initialized.
    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }
}

impl Decoder for NullDecoder {
    fn initialize(&mut self, _codec: &str, width: u32, height: u32) -> Result<(), ViewerError> {
        self.width = width;
        self.height = height;
        self.initialized = true;
        Ok(())
    }

    fn decode_packet(
        &mut self,
        _packet: &[u8],
        _frame_id: u64,
        _pts_ns: u64,
    ) -> Result<(), ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Decoder("Decoder not initialized".to_string()));
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, ViewerError> {
        if !self.initialized {
            return Err(ViewerError::Decoder("Decoder not initialized".to_string()));
        }
        Ok(None)
    }

    fn reset(&mut self) -> Result<(), ViewerError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_decoder_lifecycle() {
        let mut decoder = NullDecoder::new();
        assert!(!decoder.is_initialized());
        assert!(decoder.decode_packet(&[1, 2, 3], 1, 100).is_err());

        decoder.initialize("hevc", 1920, 1080).unwrap();
        assert!(decoder.is_initialized());
        assert!(decoder.decode_packet(&[1, 2, 3], 1, 100).is_ok());
        assert!(decoder.receive_frame().unwrap().is_none());
        assert!(decoder.reset().is_ok());
    }
}
