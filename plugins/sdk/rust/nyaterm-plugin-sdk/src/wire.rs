use crate::{Result, RpcError, Transport};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) const MAX_JSON: usize = 2 * 1024 * 1024;
pub(crate) const MAX_FRAME: usize = 8 * 1024 * 1024;
pub(crate) enum Frame {
    Json(Value),
    Binary(String, Vec<u8>),
}

pub(crate) async fn read(
    reader: &mut (impl AsyncRead + Unpin),
    transport: Transport,
) -> Result<Option<Frame>> {
    let mut first = [0_u8];
    if reader.read(&mut first).await? == 0 {
        return Ok(None);
    }
    if transport == Transport::Jsonl {
        let mut bytes = Vec::new();
        let mut byte = first[0];
        loop {
            if byte == b'\n' {
                break;
            }
            if bytes.len() == MAX_JSON {
                return Err(RpcError::invalid("JSON line exceeds 2 MiB"));
            }
            bytes.push(byte);
            byte = reader.read_u8().await?;
        }
        return Ok(Some(Frame::Json(serde_json::from_slice(&bytes)?)));
    }
    let kind = first[0];
    let length = reader.read_u32_le().await? as usize;
    if length > MAX_FRAME || (kind == 1 && length > MAX_JSON) {
        return Err(RpcError::invalid("Frame is too large"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await?;
    match kind {
        1 => Ok(Some(Frame::Json(serde_json::from_slice(&bytes)?))),
        2 if bytes.len() >= 2 => {
            let length = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
            if length == 0 || length > 128 || length + 2 > bytes.len() {
                return Err(RpcError::invalid("Invalid binary channel"));
            }
            let channel = std::str::from_utf8(&bytes[2..length + 2])
                .map_err(|_| RpcError::invalid("Invalid binary channel encoding"))?
                .to_owned();
            Ok(Some(Frame::Binary(channel, bytes[length + 2..].to_vec())))
        }
        _ => Err(RpcError::invalid("Invalid frame kind")),
    }
}

pub(crate) async fn write(
    writer: &mut (impl AsyncWrite + Unpin),
    transport: Transport,
    frame: Frame,
) -> Result<()> {
    let (kind, bytes) = match frame {
        Frame::Json(value) => {
            let bytes = serde_json::to_vec(&value)?;
            if bytes.len() > MAX_JSON {
                return Err(RpcError::invalid("JSON frame exceeds 2 MiB"));
            }
            (1, bytes)
        }
        Frame::Binary(channel, data) => {
            if transport != Transport::Framed
                || channel.is_empty()
                || channel.len() > 128
                || channel.len() + data.len() + 2 > MAX_FRAME
            {
                return Err(RpcError::invalid("Invalid binary frame"));
            }
            let mut bytes = (channel.len() as u16).to_le_bytes().to_vec();
            bytes.extend_from_slice(channel.as_bytes());
            bytes.extend_from_slice(&data);
            (2, bytes)
        }
    };
    if transport == Transport::Framed {
        writer.write_u8(kind).await?;
        writer.write_u32_le(bytes.len() as u32).await?;
    }
    writer.write_all(&bytes).await?;
    if transport == Transport::Jsonl {
        writer.write_u8(b'\n').await?;
    }
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn distinguishes_clean_eof_from_truncated_frames() {
        assert!(
            read(
                &mut std::io::Cursor::new(Vec::<u8>::new()),
                Transport::Jsonl
            )
            .await
            .unwrap()
            .is_none()
        );
        for (transport, bytes) in [
            (Transport::Jsonl, b"{\"x\":1}".to_vec()),
            (Transport::Framed, vec![1, 2, 0]),
        ] {
            assert!(
                read(&mut std::io::Cursor::new(bytes), transport)
                    .await
                    .is_err()
            );
        }
    }
    #[tokio::test]
    async fn rejects_frame_size_before_reading_payload_and_roundtrips_binary() {
        let mut bytes = vec![1];
        bytes.extend_from_slice(&(MAX_JSON as u32 + 1).to_le_bytes());
        let error = read(&mut std::io::Cursor::new(bytes), Transport::Framed)
            .await
            .err()
            .unwrap();
        assert!(error.message.contains("too large"));
        let mut bytes = Vec::new();
        write(
            &mut bytes,
            Transport::Framed,
            Frame::Binary("test".into(), vec![0, 255]),
        )
        .await
        .unwrap();
        assert!(
            matches!(read(&mut std::io::Cursor::new(bytes),Transport::Framed).await.unwrap(),Some(Frame::Binary(channel,data)) if channel=="test" && data==[0,255])
        );
    }
}
