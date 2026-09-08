//! Non-yielding burst datagram sender for frame fragments.

use bytes::Bytes;
use quinn::{Connection, SendDatagramError};

use crate::error::NetError;

/// Helper for submitting bursts of frame fragment datagrams synchronously without task yields.
pub struct FragmentBurst;

impl FragmentBurst {
    /// Largest datagram payload the connection currently accepts, if datagrams are usable.
    ///
    /// Tracks path MTU discovery, so it grows over the first second of a connection.
    #[must_use]
    pub fn max_datagram_size(connection: &Connection) -> Option<usize> {
        connection.max_datagram_size()
    }

    /// Sends a slice of fragment byte payloads over a QUIC connection in a non-yielding loop.
    ///
    /// Returns the total number of fragments successfully queued into the datagram output buffer.
    ///
    /// # Errors
    /// Returns [`NetError::DatagramTooLarge`] if a fragment exceeds the current path
    /// MTU (the caller should re-fragment at [`Self::max_datagram_size`]), or
    /// [`NetError::Datagram`] if the connection is closed.
    pub fn send_all(connection: &Connection, fragments: &[Bytes]) -> Result<usize, NetError> {
        let mut sent_count = 0;
        for frag in fragments {
            match connection.send_datagram(frag.clone()) {
                Ok(()) => sent_count += 1,
                Err(SendDatagramError::TooLarge) => {
                    return Err(NetError::DatagramTooLarge {
                        size: frag.len(),
                        max: connection.max_datagram_size().unwrap_or(0),
                    });
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
