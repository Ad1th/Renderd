//! Protocol domain newtypes for safe, strongly-typed operations.

use std::fmt;
use uuid::Uuid;

/// Monotonically increasing 64-bit frame identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameId(pub u64);

impl fmt::Display for FrameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FrameId({})", self.0)
    }
}

impl From<u64> for FrameId {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl From<FrameId> for u64 {
    fn from(v: FrameId) -> Self {
        v.0
    }
}

/// Zero-indexed fragment index within a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FragmentId(pub u16);

impl fmt::Display for FragmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FragmentId({})", self.0)
    }
}

impl From<u16> for FragmentId {
    fn from(v: u16) -> Self {
        Self(v)
    }
}

impl From<FragmentId> for u16 {
    fn from(v: FragmentId) -> Self {
        v.0
    }
}

/// Bitrate expressed in kilobits per second (kbps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitrateKbps(pub u32);

impl fmt::Display for BitrateKbps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} kbps", self.0)
    }
}

impl From<u32> for BitrateKbps {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl From<BitrateKbps> for u32 {
    fn from(v: BitrateKbps) -> Self {
        v.0
    }
}

/// Display vertical synchronization period in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VsyncPeriodNs(pub u64);

impl fmt::Display for VsyncPeriodNs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ns", self.0)
    }
}

impl From<u64> for VsyncPeriodNs {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl From<VsyncPeriodNs> for u64 {
    fn from(v: VsyncPeriodNs) -> Self {
        v.0
    }
}

/// Unique identifier for a Viewer endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewerId(pub Uuid);

impl fmt::Display for ViewerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ViewerId({})", self.0)
    }
}

impl From<Uuid> for ViewerId {
    fn from(v: Uuid) -> Self {
        Self(v)
    }
}

impl From<ViewerId> for Uuid {
    fn from(v: ViewerId) -> Self {
        v.0
    }
}

/// Unique identifier for a Host endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostId(pub Uuid);

impl fmt::Display for HostId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HostId({})", self.0)
    }
}

impl From<Uuid> for HostId {
    fn from(v: Uuid) -> Self {
        Self(v)
    }
}

impl From<HostId> for Uuid {
    fn from(v: HostId) -> Self {
        v.0
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_types_conversions_and_display() {
        let frame_id = FrameId::from(42);
        assert_eq!(u64::from(frame_id), 42);
        assert_eq!(format!("{frame_id}"), "FrameId(42)");

        let frag_id = FragmentId::from(5);
        assert_eq!(u16::from(frag_id), 5);
        assert_eq!(format!("{frag_id}"), "FragmentId(5)");

        let bitrate = BitrateKbps::from(30000);
        assert_eq!(u32::from(bitrate), 30000);
        assert_eq!(format!("{bitrate}"), "30000 kbps");

        let vsync = VsyncPeriodNs::from(16_666_666);
        assert_eq!(u64::from(vsync), 16_666_666);
        assert_eq!(format!("{vsync}"), "16666666 ns");

        let uuid = Uuid::new_v4();
        let viewer_id = ViewerId::from(uuid);
        assert_eq!(Uuid::from(viewer_id), uuid);
        assert_eq!(format!("{viewer_id}"), format!("ViewerId({uuid})"));

        let host_id = HostId::from(uuid);
        assert_eq!(Uuid::from(host_id), uuid);
        assert_eq!(format!("{host_id}"), format!("HostId({uuid})"));
    }
}
