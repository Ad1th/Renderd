//! Capture pipeline scaffold for `renderd-host`.

/// Host capture pipeline manager scaffold.
#[derive(Debug, Default)]
pub struct CapturePipeline;

impl CapturePipeline {
    /// Create a new capture pipeline scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
