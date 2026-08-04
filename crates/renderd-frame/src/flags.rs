//! Type-safe bitfield flags for datagram fragment headers per RFC-0002 §12.1.

/// Bit 0: Fragment belongs to an IDR keyframe.
pub const FLAG_KEYFRAME: u8 = 0b0000_0001;

/// Bit 1: Fragment is the first fragment of a frame.
pub const FLAG_FIRST_FRAG: u8 = 0b0000_0010;

/// Bit 2: Fragment is the last fragment of a frame.
pub const FLAG_LAST_FRAG: u8 = 0b0000_0100;

/// Bit 3: Payload is encrypted with AES-256-GCM.
pub const FLAG_ENCRYPTED: u8 = 0b0000_1000;

/// Type-safe wrapper over raw 8-bit fragment header flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct FragmentFlags(pub u8);

impl FragmentFlags {
    /// Creates an empty set of flags with all bits set to 0.
    #[must_use]
    pub const fn new() -> Self {
        Self(0)
    }

    /// Constructs flags from raw `u8` bits.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Returns the raw `u8` bitfield value.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns `true` if the keyframe flag is set.
    #[must_use]
    pub const fn is_keyframe(self) -> bool {
        (self.0 & FLAG_KEYFRAME) != 0
    }

    /// Sets or clears the keyframe flag bit.
    pub fn set_keyframe(&mut self, val: bool) {
        if val {
            self.0 |= FLAG_KEYFRAME;
        } else {
            self.0 &= !FLAG_KEYFRAME;
        }
    }

    /// Returns `true` if the first fragment flag is set.
    #[must_use]
    pub const fn is_first(self) -> bool {
        (self.0 & FLAG_FIRST_FRAG) != 0
    }

    /// Sets or clears the first fragment flag bit.
    pub fn set_first(&mut self, val: bool) {
        if val {
            self.0 |= FLAG_FIRST_FRAG;
        } else {
            self.0 &= !FLAG_FIRST_FRAG;
        }
    }

    /// Returns `true` if the last fragment flag is set.
    #[must_use]
    pub const fn is_last(self) -> bool {
        (self.0 & FLAG_LAST_FRAG) != 0
    }

    /// Sets or clears the last fragment flag bit.
    pub fn set_last(&mut self, val: bool) {
        if val {
            self.0 |= FLAG_LAST_FRAG;
        } else {
            self.0 &= !FLAG_LAST_FRAG;
        }
    }

    /// Returns `true` if the encrypted payload flag is set.
    #[must_use]
    pub const fn is_encrypted(self) -> bool {
        (self.0 & FLAG_ENCRYPTED) != 0
    }

    /// Sets or clears the encrypted flag bit.
    pub fn set_encrypted(&mut self, val: bool) {
        if val {
            self.0 |= FLAG_ENCRYPTED;
        } else {
            self.0 &= !FLAG_ENCRYPTED;
        }
    }
}

impl From<u8> for FragmentFlags {
    fn from(bits: u8) -> Self {
        Self(bits)
    }
}

impl From<FragmentFlags> for u8 {
    fn from(flags: FragmentFlags) -> Self {
        flags.0
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_flag_manipulation() {
        let mut flags = FragmentFlags::new();
        assert!(!flags.is_keyframe());
        assert!(!flags.is_first());

        flags.set_keyframe(true);
        flags.set_first(true);
        assert!(flags.is_keyframe());
        assert!(flags.is_first());
        assert_eq!(flags.bits(), FLAG_KEYFRAME | FLAG_FIRST_FRAG);

        flags.set_keyframe(false);
        assert!(!flags.is_keyframe());
        assert!(flags.is_first());
    }
}
