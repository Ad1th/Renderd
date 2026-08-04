//! Control stream framing for length-prefixed Protobuf envelope serialization over QUIC Stream 0.

use prost::Message;
use renderd_proto::Envelope;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::NetError;

/// Maximum allowable control plane message payload size (16 MB).
pub const MAX_CONTROL_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// Serializes and writes a 4-byte length-prefixed [`Envelope`] to an asynchronous stream.
///
/// # Errors
/// Returns [`NetError::Framing`] if encoding fails or [`NetError::Io`] if socket write fails.
pub async fn send_control<W>(stream: &mut W, msg: &Envelope) -> Result<(), NetError>
where
    W: AsyncWriteExt + Unpin,
{
    let mut payload_buf = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut payload_buf)
        .map_err(|e| NetError::Framing(format!("Failed to encode protobuf envelope: {e}")))?;

    if payload_buf.len() > MAX_CONTROL_MESSAGE_SIZE {
        return Err(NetError::Framing(format!(
            "Payload size {} exceeds maximum threshold {}",
            payload_buf.len(),
            MAX_CONTROL_MESSAGE_SIZE
        )));
    }

    let len_prefix = u32::try_from(payload_buf.len())
        .map_err(|_| NetError::Framing("Payload length exceeds u32 limit".to_string()))?
        .to_be_bytes();
    stream.write_all(&len_prefix).await?;
    stream.write_all(&payload_buf).await?;
    stream.flush().await?;

    Ok(())
}

/// Reads and deserializes a 4-byte length-prefixed [`Envelope`] from an asynchronous stream.
///
/// # Errors
/// Returns [`NetError::Framing`] if header decoding, size check, or protobuf decoding fails,
/// or [`NetError::Io`] if socket read fails or EOF is encountered before reading complete message.
pub async fn recv_control<R>(stream: &mut R) -> Result<Envelope, NetError>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            NetError::Framing("Connection closed while reading length prefix".to_string())
        } else {
            NetError::Io(e)
        }
    })?;

    let payload_len = u32::from_be_bytes(len_bytes) as usize;
    if payload_len > MAX_CONTROL_MESSAGE_SIZE {
        return Err(NetError::Framing(format!(
            "Incoming payload size {payload_len} exceeds maximum allowed size {MAX_CONTROL_MESSAGE_SIZE}"
        )));
    }

    let mut payload_buf = vec![0u8; payload_len];
    stream.read_exact(&mut payload_buf).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            NetError::Framing("Connection closed while reading payload bytes".to_string())
        } else {
            NetError::Io(e)
        }
    })?;

    let envelope = Envelope::decode(&payload_buf[..])
        .map_err(|e| NetError::Framing(format!("Failed to decode protobuf envelope: {e}")))?;

    Ok(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderd_proto::generated::renderd::{envelope, SessionHello, VsyncReport};
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_send_recv_control_single_and_multiple() {
        let (mut client, mut server) = duplex(1024);

        let env1 = Envelope {
            payload: Some(envelope::Payload::Hello(SessionHello {
                protocol_version: 1,
                min_required_version: 1,
                viewer_id: "viewer-123".to_string(),
                supported_codecs: vec!["hevc".to_string()],
                max_decode_bitrate_kbps: 50_000,
                display: None,
                hw_decode_available: true,
                session_nonce: "nonce-456".to_string(),
            })),
        };

        let env2 = Envelope {
            payload: Some(envelope::Payload::VsyncReport(VsyncReport {
                vsync_period_ns: 16_666_666,
                vsync_phase_ns: 100_000_000,
                clock_epoch_ns: 200_000_000,
            })),
        };

        // Send sequential envelopes
        send_control(&mut client, &env1).await.unwrap();
        send_control(&mut client, &env2).await.unwrap();

        // Receive sequential envelopes
        let recv1 = recv_control(&mut server).await.unwrap();
        let recv2 = recv_control(&mut server).await.unwrap();

        assert_eq!(recv1, env1);
        assert_eq!(recv2, env2);
    }
}
