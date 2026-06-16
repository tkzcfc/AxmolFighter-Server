use bytes::{Buf, BufMut, Bytes, BytesMut};

// ========== 网关内部协议 msg_id 常量 ==========

pub const MSG_SERVER_REG_REQ: u16 = 10001;
pub const MSG_SERVER_REG_RESP: u16 = 10002;
pub const MSG_BIND_BATTLE: u16 = 10003;
pub const MSG_UNBIND_BATTLE: u16 = 10004;
pub const MSG_KICK_SESSION: u16 = 10005;
pub const MSG_SESSION_ONLINE: u16 = 10006;
pub const MSG_SESSION_OFFLINE: u16 = 10007;
pub const MSG_SERVER_ONLINE: u16 = 10008;
pub const MSG_SERVER_OFFLINE: u16 = 10009;
pub const MSG_FORWARD_TO_SERVER: u16 = 10010;
pub const MSG_SERVER_STATUS: u16 = 10011;

/// 内部协议 msg_id 范围
pub const INTERNAL_MSG_RANGE: std::ops::RangeInclusive<u16> = 10000..=19999;

/// 服务类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ServiceType {
    Game = 1,
    Battle = 2,
}

impl ServiceType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Game),
            2 => Some(Self::Battle),
            _ => None,
        }
    }

    /// 从路由配置名称解析
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "game" => Some(Self::Game),
            "battle" => Some(Self::Battle),
            _ => None,
        }
    }
}

// ========== 协议消息结构体 ==========

/// 后端注册请求 (Backend→GW)
#[derive(Debug, Clone)]
pub struct ServerRegReq {
    pub service_type: u8,
    pub instance_id: u32,
}

/// 后端注册响应 (GW→Backend)
#[derive(Debug, Clone)]
pub struct ServerRegResp {
    /// 0=成功, 非0=失败
    pub code: u8,
    /// 当前已注册的其他服务列表: [(service_type, instance_id), ...]
    pub servers: Vec<(u8, u32)>,
}

/// 绑定会话到战斗服 (GameSvr→GW)
#[derive(Debug, Clone)]
pub struct BindBattle {
    pub session_id: u32,
    pub battle_instance_id: u32,
}

/// 解绑会话的战斗服 (GameSvr→GW)
#[derive(Debug, Clone)]
pub struct UnbindBattle {
    pub session_id: u32,
}

/// 踢出客户端连接 (Backend→GW)
#[derive(Debug, Clone)]
pub struct KickSession {
    pub session_id: u32,
}

/// 客户端上线通知 (GW→Backend)
#[derive(Debug, Clone)]
pub struct SessionOnline {
    pub session_id: u32,
}

/// 客户端下线通知 (GW→Backend)
#[derive(Debug, Clone)]
pub struct SessionOffline {
    pub session_id: u32,
}

/// 服务上线广播 (GW→Backend)
#[derive(Debug, Clone)]
pub struct ServerOnline {
    pub service_type: u8,
    pub instance_id: u32,
}

/// 服务下线广播 (GW→Backend)
#[derive(Debug, Clone)]
pub struct ServerOffline {
    pub service_type: u8,
    pub instance_id: u32,
}

/// 服务间转发 (Backend→GW)
#[derive(Debug, Clone)]
pub struct ForwardToServer {
    pub target_service_type: u8,
    pub target_instance_id: u32,
    pub payload: Bytes,
}

/// 服务可用性通知 (GW→Client)
#[derive(Debug, Clone)]
pub struct ServerStatus {
    /// 可用服务列表: [(service_type, online), ...]
    pub services: Vec<(u8, bool)>,
}

// ========== 编解码实现 ==========

impl ServerRegReq {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5);
        buf.put_u8(self.service_type);
        buf.put_u32_le(self.instance_id);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 5 {
            anyhow::bail!("ServerRegReq: insufficient data");
        }
        let service_type = data.get_u8();
        let instance_id = data.get_u32_le();
        Ok(Self { service_type, instance_id })
    }
}

impl ServerRegResp {
    pub fn encode(&self) -> Bytes {
        // code(1) + count(2) + entries(5 each)
        let mut buf = BytesMut::with_capacity(3 + self.servers.len() * 5);
        buf.put_u8(self.code);
        buf.put_u16_le(self.servers.len() as u16);
        for &(stype, iid) in &self.servers {
            buf.put_u8(stype);
            buf.put_u32_le(iid);
        }
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 3 {
            anyhow::bail!("ServerRegResp: insufficient data");
        }
        let code = data.get_u8();
        let count = data.get_u16_le() as usize;
        let mut servers = Vec::with_capacity(count);
        for _ in 0..count {
            if data.len() < 5 {
                anyhow::bail!("ServerRegResp: truncated server list");
            }
            let stype = data.get_u8();
            let iid = data.get_u32_le();
            servers.push((stype, iid));
        }
        Ok(Self { code, servers })
    }
}

impl BindBattle {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(8);
        buf.put_u32_le(self.session_id);
        buf.put_u32_le(self.battle_instance_id);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 8 {
            anyhow::bail!("BindBattle: insufficient data");
        }
        let session_id = data.get_u32_le();
        let battle_instance_id = data.get_u32_le();
        Ok(Self { session_id, battle_instance_id })
    }
}

impl UnbindBattle {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u32_le(self.session_id);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 4 {
            anyhow::bail!("UnbindBattle: insufficient data");
        }
        let session_id = data.get_u32_le();
        Ok(Self { session_id })
    }
}

impl KickSession {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u32_le(self.session_id);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 4 {
            anyhow::bail!("KickSession: insufficient data");
        }
        let session_id = data.get_u32_le();
        Ok(Self { session_id })
    }
}

impl SessionOnline {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u32_le(self.session_id);
        buf.freeze()
    }
}

impl SessionOffline {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u32_le(self.session_id);
        buf.freeze()
    }
}

impl ServerOnline {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5);
        buf.put_u8(self.service_type);
        buf.put_u32_le(self.instance_id);
        buf.freeze()
    }
}

impl ServerOffline {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5);
        buf.put_u8(self.service_type);
        buf.put_u32_le(self.instance_id);
        buf.freeze()
    }
}

impl ForwardToServer {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(5 + self.payload.len());
        buf.put_u8(self.target_service_type);
        buf.put_u32_le(self.target_instance_id);
        buf.put_slice(&self.payload);
        buf.freeze()
    }

    pub fn decode(mut data: Bytes) -> anyhow::Result<Self> {
        if data.len() < 5 {
            anyhow::bail!("ForwardToServer: insufficient data");
        }
        let target_service_type = data.get_u8();
        let target_instance_id = data.get_u32_le();
        let payload = data; // 剩余部分
        Ok(Self {
            target_service_type,
            target_instance_id,
            payload,
        })
    }
}

impl ServerStatus {
    pub fn encode(&self) -> Bytes {
        // count(2) + entries(2 each: service_type + online)
        let mut buf = BytesMut::with_capacity(2 + self.services.len() * 2);
        buf.put_u16_le(self.services.len() as u16);
        for &(stype, online) in &self.services {
            buf.put_u8(stype);
            buf.put_u8(if online { 1 } else { 0 });
        }
        buf.freeze()
    }
}
