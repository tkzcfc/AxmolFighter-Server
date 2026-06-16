use bytes::{Buf, BufMut, Bytes, BytesMut};

/// 后端帧头大小：4字节 len + 2字节 msg_id + 4字节 serial + 4字节 session_id
pub const BACKEND_HEADER_SIZE: usize = 14;
/// 最大包体大小
pub const MAX_PACKET_SIZE: u32 = 1024 * 1024;

/// 后端帧（游戏服与网关之间的通信格式）
#[derive(Debug, Clone)]
pub struct BackendFrame {
    pub msg_id: u16,
    /// 序号：<0 请求, >0 回复, =0 推送
    pub serial: i32,
    pub session_id: u32,
    pub payload: Bytes,
}

/// 尝试从缓冲区提取一个后端帧
pub fn try_extract_frame(buf: &mut BytesMut) -> anyhow::Result<Option<BackendFrame>> {
    if buf.len() < BACKEND_HEADER_SIZE {
        return Ok(None);
    }

    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

    if len < BACKEND_HEADER_SIZE {
        anyhow::bail!("invalid frame: len {} < header size", len);
    }
    if len as u32 > MAX_PACKET_SIZE {
        anyhow::bail!("frame too large: {} bytes", len);
    }
    if buf.len() < len {
        return Ok(None);
    }

    let mut frame_data = buf.split_to(len);
    frame_data.advance(4); // 跳过 len
    let msg_id = frame_data.get_u16_le();
    let serial = frame_data.get_i32_le();
    let session_id = frame_data.get_u32_le();
    let payload = frame_data.freeze();

    Ok(Some(BackendFrame {
        msg_id,
        serial,
        session_id,
        payload,
    }))
}

/// 编码后端帧
pub fn encode_frame(msg_id: u16, serial: i32, session_id: u32, payload: &[u8]) -> Bytes {
    let total_len = BACKEND_HEADER_SIZE + payload.len();
    let mut buf = BytesMut::with_capacity(total_len);
    buf.put_u32_le(total_len as u32);
    buf.put_u16_le(msg_id);
    buf.put_i32_le(serial);
    buf.put_u32_le(session_id);
    buf.put_slice(payload);
    buf.freeze()
}
