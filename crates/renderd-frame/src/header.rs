//! 16-byte fixed binary fragment header codec per RFC-0002 §12.1.

use crate::error::FrameError;

/// Fixed length of a datagram fragment header in bytes.
pub const HEADER_SIZE: usize = 16;

/// Maximum value representable in a 24-bit unsigned integer (16,777,215 us ~ 16.7s).
pub const MAX_PTS_OFFSET_US: u32 = 0x00FF_FFFF;

/// Datagram fragment header prefixed to every data plane video packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentHeader {
    /// Monotonically increasing frame sequence number.
    pub frame_id: u64,
    /// Zero-indexed fragment index within frame (0 .. `frag_total` - 1).
    pub frag_id: u16,
    /// Total number of fragments composing this frame.
    pub frag_total: u16,
    /// Bitfield flags (Keyframe, First, Last, Crypt).
    pub flags: u8,
    /// Presentation timestamp offset in microseconds (24-bit LE).
    pub pts_offset_us: u32,
}

impl FragmentHeader {
    /// Encodes this header into a 16-byte buffer slice in little-endian format.
    ///
    /// # Errors
    /// Returns [`FrameError::BufferTooShort`] if `out.len() < HEADER_SIZE`, or
    /// [`FrameError::PtsOffsetOverflow`] if `pts_offset_us > 0x00FF_FFFF`.
    pub fn encode(&self, out: &mut [u8]) -> Result<(), FrameError> {
        if out.len() < HEADER_SIZE {
            return Err(FrameError::BufferTooShort {
                expected: HEADER_SIZE,
                got: out.len(),
            });
        }

        if self.pts_offset_us > MAX_PTS_OFFSET_US {
            return Err(FrameError::PtsOffsetOverflow(self.pts_offset_us));
        }

        out[0..8].copy_from_slice(&self.frame_id.to_le_bytes());
        out[8..10].copy_from_slice(&self.frag_id.to_le_bytes());
        out[10..12].copy_from_slice(&self.frag_total.to_le_bytes());
        out[12] = self.flags;

        // Encode 24-bit pts_offset_us in little-endian (3 bytes)
        let pts_bytes = self.pts_offset_us.to_le_bytes();
        out[13..16].copy_from_slice(&pts_bytes[0..3]);

        Ok(())
    }

    /// Decodes a 16-byte header from a binary slice in little-endian format.
    ///
    /// # Errors
    /// Returns [`FrameError::BufferTooShort`] if `buf.len() < HEADER_SIZE`.
    pub fn decode(buf: &[u8]) -> Result<Self, FrameError> {
        if buf.len() < HEADER_SIZE {
            return Err(FrameError::BufferTooShort {
                expected: HEADER_SIZE,
                got: buf.len(),
            });
        }

        let frame_id = u64::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
        ]);
        let frag_id = u16::from_le_bytes([buf[8], buf[9]]);
        let frag_total = u16::from_le_bytes([buf[10], buf[11]]);
        let flags = buf[12];

        // Decode 24-bit pts_offset_us from little-endian bytes
        let pts_offset_us =
            u32::from(buf[13]) | (u32::from(buf[14]) << 8) | (u32::from(buf[15]) << 16);

        Ok(Self {
            frame_id,
            frag_id,
            frag_total,
            flags,
            pts_offset_us,
        })
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let header = FragmentHeader {
            frame_id: 0x1234_5678_9ABC_DEF0,
            frag_id: 42,
            frag_total: 100,
            flags: 0b0000_0111,
            pts_offset_us: 12_345_678,
        };

        let mut buf = [0u8; 16];
        header.encode(&mut buf).unwrap();

        let decoded = FragmentHeader::decode(&buf).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn test_buffer_too_short() {
        let header = FragmentHeader {
            frame_id: 1,
            frag_id: 0,
            frag_total: 1,
            flags: 0,
            pts_offset_us: 0,
        };

        let mut short_buf = [0u8; 15];
        assert_eq!(
            header.encode(&mut short_buf),
            Err(FrameError::BufferTooShort {
                expected: 16,
                got: 15
            })
        );
        assert_eq!(
            FragmentHeader::decode(&short_buf),
            Err(FrameError::BufferTooShort {
                expected: 16,
                got: 15
            })
        );
    }

    #[test]
    fn test_pts_overflow() {
        let header = FragmentHeader {
            frame_id: 1,
            frag_id: 0,
            frag_total: 1,
            flags: 0,
            pts_offset_us: 0x0100_0000,
        };

        let mut buf = [0u8; 16];
        assert_eq!(
            header.encode(&mut buf),
            Err(FrameError::PtsOffsetOverflow(0x0100_0000))
        );
    }
}
