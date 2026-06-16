use bytes::{Buf, BufMut, Bytes, BytesMut};

/// 客户端帧头大小：4字节 len + 2字节 msg_id + 4字节 serial
pub const CLIENT_HEADER_SIZE: usize = 10;

/// 后端帧头大小：4字节 len + 2字节 msg_id + 4字节 serial + 4字节 session_id
pub const BACKEND_HEADER_SIZE: usize = 14;

/// 最大包体大小（防止恶意超大包）
pub const MAX_PACKET_SIZE: u32 = 1024 * 1024; // 1MB

/// 客户端发来的帧
#[derive(Debug, Clone)]
pub struct ClientFrame {
    pub msg_id: u16,
    /// 序号：<0 请求, >0 回复, =0 推送
    pub serial: i32,
    pub payload: Bytes,
}

/// 后端服务间通信的帧（含 session_id）
#[derive(Debug, Clone)]
pub struct BackendFrame {
    pub msg_id: u16,
    /// 序号：<0 请求, >0 回复, =0 推送
    pub serial: i32,
    pub session_id: u32,
    pub payload: Bytes,
}

/// 尝试从缓冲区提取一个客户端帧
/// 返回 None 表示数据不足，需要继续读取
pub fn try_extract_client_frame(buf: &mut BytesMut) -> anyhow::Result<Option<ClientFrame>> {
    if buf.len() < CLIENT_HEADER_SIZE {
        return Ok(None);
    }

    // 读取 len（不消费）
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

    // 合法性检查
    if len < CLIENT_HEADER_SIZE {
        anyhow::bail!("invalid client frame: len {} < header size {}", len, CLIENT_HEADER_SIZE);
    }
    if len as u32 > MAX_PACKET_SIZE {
        anyhow::bail!("client frame too large: {} bytes", len);
    }

    // 数据不足
    if buf.len() < len {
        return Ok(None);
    }

    // 消费整个帧
    let mut frame_data = buf.split_to(len);

    // 跳过 len 字段
    frame_data.advance(4);

    // 读取 msg_id
    let msg_id = frame_data.get_u16_le();

    // 读取 serial
    let serial = frame_data.get_i32_le();

    // 剩余为 payload
    let payload = frame_data.freeze();

    Ok(Some(ClientFrame { msg_id, serial, payload }))
}

/// 尝试从缓冲区提取一个后端帧
pub fn try_extract_backend_frame(buf: &mut BytesMut) -> anyhow::Result<Option<BackendFrame>> {
    if buf.len() < BACKEND_HEADER_SIZE {
        return Ok(None);
    }

    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

    if len < BACKEND_HEADER_SIZE {
        anyhow::bail!("invalid backend frame: len {} < header size {}", len, BACKEND_HEADER_SIZE);
    }
    if len as u32 > MAX_PACKET_SIZE {
        anyhow::bail!("backend frame too large: {} bytes", len);
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

/// 将客户端帧编码为字节（用于发送给客户端）
pub fn encode_client_frame(msg_id: u16, serial: i32, payload: &[u8]) -> Bytes {
    let total_len = CLIENT_HEADER_SIZE + payload.len();
    let mut buf = BytesMut::with_capacity(total_len);
    buf.put_u32_le(total_len as u32);
    buf.put_u16_le(msg_id);
    buf.put_i32_le(serial);
    buf.put_slice(payload);
    buf.freeze()
}

/// 将后端帧编码为字节（用于发送给后端服务）
pub fn encode_backend_frame(msg_id: u16, serial: i32, session_id: u32, payload: &[u8]) -> Bytes {
    let total_len = BACKEND_HEADER_SIZE + payload.len();
    let mut buf = BytesMut::with_capacity(total_len);
    buf.put_u32_le(total_len as u32);
    buf.put_u16_le(msg_id);
    buf.put_i32_le(serial);
    buf.put_u32_le(session_id);
    buf.put_slice(payload);
    buf.freeze()
}
