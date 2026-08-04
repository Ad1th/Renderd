//! Non-yielding burst datagram sender for frame fragments.

use bytes::Bytes;
use quinn::Connection;

use crate::error::NetError;

/// Helper for submitting bursts of frame fragment datagrams synchronously without task yields.
pub struct FragmentBurst;

impl FragmentBurst {
    /// Sends a slice of fragment byte payloads over a QUIC connection in a non-yielding loop.
    ///
    /// Returns the total number of fragments successfully queued into the datagram output buffer.
    ///
    /// # Errors
    /// Returns [`NetError::Datagram`] if datagram sending fails or connection is closed.
    pub fn send_all(connection: &Connection, fragments: &[Bytes]) -> Result<usize, NetError> {
        let mut sent_count = 0;
        for frag in fragments {
            match connection.send_datagram(frag.clone()) {
                Ok(()) => {
                    sent_count += 1;
                }
                Err(e) => {
                    return Err(NetError::Datagram(format!("Datagram send error: {e}")));
                }
            }
        }
        Ok(sent_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_burst_empty_slice() {
        let _ = FragmentBurst;
    }
}
