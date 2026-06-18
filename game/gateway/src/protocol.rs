use bytes::{Buf, BufMut, Bytes, BytesMut};

// cmd 只负责区分这一帧要怎么处理，业务协议号还是放在 msg_id 里。
// 客户端、网关、后端都要按这张表来，别再把网关内部命令塞进 msg_id 段了。
/// 普通业务消息，payload 是业务 protobuf，msg_id 必须有效。
pub const CMD_BUSINESS: u8 = 0;
/// 网关返回给客户端的错误，msg_id 会带回原请求的业务号。
pub const CMD_GATEWAY_ERROR: u8 = 1;
/// 网关推给客户端的服务状态快照，payload 是 ServerStatusPush。
pub const CMD_SERVER_STATUS: u8 = 2;
/// 后端服务连上网关后的注册请求。
pub const CMD_SERVER_REG_REQ: u8 = 10;
/// 网关给后端服务的注册结果。
pub const CMD_SERVER_REG_RESP: u8 = 11;
/// 绑定会话到某类服务实例，实例 ID 可以让网关自动挑。
pub const CMD_BIND_SERVICE: u8 = 12;
/// 取消会话和某类服务的绑定。
pub const CMD_UNBIND_SERVICE: u8 = 13;
/// 后端要求网关踢掉某个客户端会话。
pub const CMD_KICK_SESSION: u8 = 14;
/// 网关通知后端：客户端会话上线了。
pub const CMD_SESSION_ONLINE: u8 = 15;
/// 网关通知后端：客户端会话下线了。
pub const CMD_SESSION_OFFLINE: u8 = 16;
/// 网关通知后端：有新的服务实例上线。
pub const CMD_SERVER_ONLINE: u8 = 17;
/// 网关通知后端：某个服务实例下线。
pub const CMD_SERVER_OFFLINE: u8 = 18;
/// 后端让网关把一段已经封好的帧转给指定服务实例。
pub const CMD_FORWARD_TO_SERVER: u8 = 19;

#[derive(Debug, Clone)]
pub struct ServerRegReq {
    pub service_id: u8,
    pub instance_id: u32,
}

#[derive(Debug, Clone)]
pub struct ServerRegResp {
    pub code: u8,
    pub servers: Vec<(u8, u32)>,
}

#[derive(Debug, Clone)]
pub struct BindService {
    pub session_id: u32,
    pub service_id: u8,
    pub target_instance_id: i32,
}

#[derive(Debug, Clone)]
pub struct UnbindService {
    pub session_id: u32,
    pub service_id: u8,
}

#[derive(Debug, Clone)]
pub struct KickSession {
    pub session_id: u32,
}

#[derive(Debug, Clone)]
pub struct SessionOnline {
    pub session_id: u32,
}

#[derive(Debug, Clone)]
pub struct SessionOffline {
    pub session_id: u32,
}

#[derive(Debug, Clone)]
pub struct ServerOnline {
    pub service_id: u8,
    pub instance_id: u32,
}

#[derive(Debug, Clone)]
pub struct ServerOffline {
    pub service_id: u8,
    pub instance_id: u32,
}

#[derive(Debug, Clone)]
pub struct ForwardToServer {
    pub target_service_id: u8,
    pub target_instance_id: u32,
    pub payload: Bytes,
}

impl ServerRegReq {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5);
        buf.put_u8(self.service_id);
        buf.put_u32(self.instance_id);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 5 {
            anyhow::bail!("ServerRegReq: insufficient data");
        }
        let service_id = data.get_u8();
        let instance_id = data.get_u32();
        Ok(Self {
            service_id,
            instance_id,
        })
    }
}

impl ServerRegResp {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(3 + self.servers.len() * 5);
        buf.put_u8(self.code);
        buf.put_u16(self.servers.len() as u16);
        for &(service_id, instance_id) in &self.servers {
            buf.put_u8(service_id);
            buf.put_u32(instance_id);
        }
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 3 {
            anyhow::bail!("ServerRegResp: insufficient data");
        }
        let code = data.get_u8();
        let count = data.get_u16() as usize;
        let mut servers = Vec::with_capacity(count);
        for _ in 0..count {
            if data.len() < 5 {
                anyhow::bail!("ServerRegResp: truncated server list");
            }
            let service_id = data.get_u8();
            let instance_id = data.get_u32();
            servers.push((service_id, instance_id));
        }
        Ok(Self { code, servers })
    }
}

impl BindService {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(9);
        buf.put_u32(self.session_id);
        buf.put_u8(self.service_id);
        buf.put_i32(self.target_instance_id);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 9 {
            anyhow::bail!("BindService: insufficient data");
        }
        let session_id = data.get_u32();
        let service_id = data.get_u8();
        let target_instance_id = data.get_i32();
        Ok(Self {
            session_id,
            service_id,
            target_instance_id,
        })
    }
}

impl UnbindService {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5);
        buf.put_u32(self.session_id);
        buf.put_u8(self.service_id);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 5 {
            anyhow::bail!("UnbindService: insufficient data");
        }
        let session_id = data.get_u32();
        let service_id = data.get_u8();
        Ok(Self {
            session_id,
            service_id,
        })
    }
}

impl KickSession {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u32(self.session_id);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 4 {
            anyhow::bail!("KickSession: insufficient data");
        }
        let session_id = data.get_u32();
        Ok(Self { session_id })
    }
}

impl SessionOnline {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u32(self.session_id);
        buf.freeze()
    }
}

impl SessionOffline {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u32(self.session_id);
        buf.freeze()
    }
}

impl ServerOnline {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5);
        buf.put_u8(self.service_id);
        buf.put_u32(self.instance_id);
        buf.freeze()
    }
}

impl ServerOffline {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5);
        buf.put_u8(self.service_id);
        buf.put_u32(self.instance_id);
        buf.freeze()
    }
}

impl ForwardToServer {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5 + self.payload.len());
        buf.put_u8(self.target_service_id);
        buf.put_u32(self.target_instance_id);
        buf.put_slice(&self.payload);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 5 {
            anyhow::bail!("ForwardToServer: insufficient data");
        }
        let target_service_id = data.get_u8();
        let target_instance_id = data.get_u32();
        let payload = data;
        Ok(Self {
            target_service_id,
            target_instance_id,
            payload,
        })
    }
}
