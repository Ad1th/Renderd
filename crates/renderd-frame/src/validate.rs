//! Validation rules for incoming datagram fragment headers.

use crate::error::FrameError;
use crate::flags::FragmentFlags;
use crate::header::FragmentHeader;

/// Extension trait for validating `FragmentHeader` field constraints.
pub trait ValidateHeader {
    /// Validates `FragmentHeader` index bounds and flag consistency.
    ///
    /// # Errors
    /// Returns [`FrameError::InvalidFragmentBounds`] if fragment indices or flag bits are invalid.
    fn validate(&self) -> Result<(), FrameError>;
}

impl ValidateHeader for FragmentHeader {
    fn validate(&self) -> Result<(), FrameError> {
        if self.frag_total == 0 || self.frag_id >= self.frag_total {
            return Err(FrameError::InvalidFragmentBounds {
                frag_id: self.frag_id,
                frag_total: self.frag_total,
            });
        }

        let flags = FragmentFlags::from_bits(self.flags);

        let should_be_first = self.frag_id == 0;
        if flags.is_first() != should_be_first {
            return Err(FrameError::InvalidFragmentBounds {
                frag_id: self.frag_id,
                frag_total: self.frag_total,
            });
        }

        let should_be_last = self.frag_id == self.frag_total - 1;
        if flags.is_last() != should_be_last {
            return Err(FrameError::InvalidFragmentBounds {
                frag_id: self.frag_id,
                frag_total: self.frag_total,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::flags::{FLAG_FIRST_FRAG, FLAG_KEYFRAME, FLAG_LAST_FRAG};

    #[test]
    fn test_valid_single_fragment_frame() {
        let header = FragmentHeader {
            frame_id: 1,
            frag_id: 0,
            frag_total: 1,
            flags: FLAG_FIRST_FRAG | FLAG_LAST_FRAG | FLAG_KEYFRAME,
            pts_offset_us: 100,
        };
        assert!(header.validate().is_ok());
    }

    #[test]
    fn test_valid_multi_fragment_frame() {
        let h0 = FragmentHeader {
            frame_id: 1,
            frag_id: 0,
            frag_total: 2,
            flags: FLAG_FIRST_FRAG,
            pts_offset_us: 100,
        };
        assert!(h0.validate().is_ok());

        let h1 = FragmentHeader {
            frame_id: 1,
            frag_id: 1,
            frag_total: 2,
            flags: FLAG_LAST_FRAG,
            pts_offset_us: 100,
        };
        assert!(h1.validate().is_ok());
    }

    #[test]
    fn test_invalid_frag_id_out_of_bounds() {
        let header = FragmentHeader {
            frame_id: 1,
            frag_id: 2,
            frag_total: 2,
            flags: FLAG_LAST_FRAG,
            pts_offset_us: 100,
        };
        assert_eq!(
            header.validate(),
            Err(FrameError::InvalidFragmentBounds {
                frag_id: 2,
                frag_total: 2
            })
        );
    }
}
