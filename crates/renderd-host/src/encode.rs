//! Video encoding dispatch pipeline scaffold for `renderd-host`.

/// Host video encoding pipeline manager scaffold.
#[derive(Debug, Default)]
pub struct EncodePipeline;

impl EncodePipeline {
    /// Create a new encode pipeline scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
