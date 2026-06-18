use bytes::{Buf, BufMut, Bytes, BytesMut};

pub const CLIENT_HEADER_SIZE: usize = 11;
pub const BACKEND_HEADER_SIZE: usize = 15;
pub const MAX_PACKET_SIZE: u32 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ClientFrame {
    pub cmd: u8,
    pub msg_id: u16,
    pub serial: i32,
    pub payload: Bytes,
}

#[derive(Debug, Clone)]
pub struct BackendFrame {
    pub cmd: u8,
    pub msg_id: u16,
    pub serial: i32,
    pub session_id: u32,
    pub payload: Bytes,
}

pub fn try_extract_client_frame(buf: &mut BytesMut) -> anyhow::Result<Option<ClientFrame>> {
    if buf.len() < CLIENT_HEADER_SIZE {
        return Ok(None);
    }

    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len < CLIENT_HEADER_SIZE {
        anyhow::bail!(
            "invalid client frame: len {} < header size {}",
            len,
            CLIENT_HEADER_SIZE
        );
    }
    if len as u32 > MAX_PACKET_SIZE {
        anyhow::bail!("client frame too large: {} bytes", len);
    }
    if buf.len() < len {
        return Ok(None);
    }

    let mut frame_data = buf.split_to(len);
    frame_data.advance(4);
    let cmd = frame_data.get_u8();
    let msg_id = frame_data.get_u16();
    let serial = frame_data.get_i32();
    let payload = frame_data.freeze();

    Ok(Some(ClientFrame {
        cmd,
        msg_id,
        serial,
        payload,
    }))
}

pub fn try_extract_backend_frame(buf: &mut BytesMut) -> anyhow::Result<Option<BackendFrame>> {
    if buf.len() < BACKEND_HEADER_SIZE {
        return Ok(None);
    }

    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len < BACKEND_HEADER_SIZE {
        anyhow::bail!(
            "invalid backend frame: len {} < header size {}",
            len,
            BACKEND_HEADER_SIZE
        );
    }
    if len as u32 > MAX_PACKET_SIZE {
        anyhow::bail!("backend frame too large: {} bytes", len);
    }
    if buf.len() < len {
        return Ok(None);
    }

    let mut frame_data = buf.split_to(len);
    frame_data.advance(4);
    let cmd = frame_data.get_u8();
    let msg_id = frame_data.get_u16();
    let serial = frame_data.get_i32();
    let session_id = frame_data.get_u32();
    let payload = frame_data.freeze();

    Ok(Some(BackendFrame {
        cmd,
        msg_id,
        serial,
        session_id,
        payload,
    }))
}

pub fn encode_client_frame(cmd: u8, msg_id: u16, serial: i32, payload: &[u8]) -> Bytes {
    let total_len = CLIENT_HEADER_SIZE + payload.len();
    let mut buf = BytesMut::with_capacity(total_len);
    buf.put_u32(total_len as u32);
    buf.put_u8(cmd);
    buf.put_u16(msg_id);
    buf.put_i32(serial);
    buf.put_slice(payload);
    buf.freeze()
}

pub fn encode_backend_frame(
    cmd: u8,
    msg_id: u16,
    serial: i32,
    session_id: u32,
    payload: &[u8],
) -> Bytes {
    let total_len = BACKEND_HEADER_SIZE + payload.len();
    let mut buf = BytesMut::with_capacity(total_len);
    buf.put_u32(total_len as u32);
    buf.put_u8(cmd);
    buf.put_u16(msg_id);
    buf.put_i32(serial);
    buf.put_u32(session_id);
    buf.put_slice(payload);
    buf.freeze()
}
