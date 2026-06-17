use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::codec::{encode_frame, try_extract_frame, BackendFrame};
use crate::handler::MessageHandler;

/// 网关内部协议 msg_id
const MSG_SERVER_REG_REQ: u16 = 10001;
const MSG_SERVER_REG_RESP: u16 = 10002;
const MSG_SESSION_ONLINE: u16 = 10006;
const MSG_SESSION_OFFLINE: u16 = 10007;
const MSG_SERVER_ONLINE: u16 = 10008;
const MSG_SERVER_OFFLINE: u16 = 10009;

/// 服务类型：游戏服 = 1
const SERVICE_TYPE_GAME: u8 = 1;

/// 向网关发送数据的通道
pub type GatewaySender = mpsc::UnboundedSender<Bytes>;

/// 网关客户端：负责连接网关、注册、收发消息
pub struct GatewayClient {
    addr: String,
    instance_id: u32,
    reconnect_interval: Duration,
    shutdown: watch::Receiver<bool>,
}

impl GatewayClient {
    pub fn new(addr: String, instance_id: u32, reconnect_interval: Duration, shutdown: watch::Receiver<bool>) -> Self {
        Self {
            addr,
            instance_id,
            reconnect_interval,
            shutdown,
        }
    }

    /// 启动连接循环（自动重连）
    /// 返回发送通道，调用方可通过该通道向网关发送帧
    pub async fn run(mut self, handler: Arc<dyn MessageHandler>) {
        loop {
            // 先检查是否已经收到关闭信号
            if *self.shutdown.borrow() {
                info!("gateway client shutting down");
                return;
            }

            match TcpStream::connect(&self.addr).await {
                Ok(stream) => {
                    info!("connected to gateway at {}", self.addr);
                    let (tx, rx) = mpsc::unbounded_channel::<Bytes>();

                    // 通知 handler 连接建立
                    handler.on_gateway_connected(tx.clone());

                    // 发送注册请求
                    let reg_payload = self.build_reg_req();
                    let reg_frame = encode_frame(MSG_SERVER_REG_REQ, 0, 0, &reg_payload);
                    let _ = tx.send(reg_frame);

                    // 运行读写循环
                    self.run_connection(stream, tx, rx, handler.clone()).await;

                    // 连接断开
                    handler.on_gateway_disconnected();
                    warn!("disconnected from gateway, reconnecting in {}s...", self.reconnect_interval.as_secs());
                }
                Err(e) => {
                    error!("failed to connect gateway at {}: {}", self.addr, e);
                }
            }

            // 等待重连间隔或关闭信号
            tokio::select! {
                _ = tokio::time::sleep(self.reconnect_interval) => {}
                _ = self.shutdown.changed() => {
                    info!("gateway client shutting down");
                    return;
                }
            }
        }
    }

    /// 单次连接的读写循环
    async fn run_connection(
        &self,
        stream: TcpStream,
        _tx: mpsc::UnboundedSender<Bytes>,
        mut rx: mpsc::UnboundedReceiver<Bytes>,
        handler: Arc<dyn MessageHandler>,
    ) {
        let (mut reader, mut writer) = stream.into_split();
        let mut shutdown = self.shutdown.clone();

        // 写任务
        let mut write_shutdown = self.shutdown.clone();
        let write_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Some(data) => {
                                if let Err(e) = writer.write_all(&data).await {
                                    debug!("write error: {}", e);
                                    break;
                                }
                            }
                            None => break, // 通道关闭
                        }
                    }
                    _ = write_shutdown.changed() => break,
                }
            }
        });

        // 读循环
        let mut buf = BytesMut::with_capacity(8192);
        loop {
            tokio::select! {
                result = reader.read_buf(&mut buf) => {
                    match result {
                        Ok(0) => {
                            debug!("gateway connection EOF");
                            break;
                        }
                        Ok(_) => {
                            // 提取所有完整帧
                            loop {
                                match try_extract_frame(&mut buf) {
                                    Ok(Some(frame)) => {
                                        self.dispatch_frame(frame, &handler).await;
                                    }
                                    Ok(None) => break,
                                    Err(e) => {
                                        error!("frame decode error: {}", e);
                                        // 关闭连接
                                        write_task.abort();
                                        return;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            debug!("read error: {}", e);
                            break;
                        }
                    }
                }
                _ = shutdown.changed() => break,
            }
        }

        write_task.abort();
    }

    /// 分发帧到 handler
    async fn dispatch_frame(&self, frame: BackendFrame, handler: &Arc<dyn MessageHandler>) {
        match frame.msg_id {
            MSG_SERVER_REG_RESP => {
                // 注册响应
                if !frame.payload.is_empty() {
                    let code = frame.payload[0];
                    if code == 0 {
                        info!("registered to gateway successfully");
                    } else {
                        error!("gateway registration failed, code={}", code);
                    }
                }
            }
            MSG_SESSION_ONLINE => {
                // 客户端上线
                if frame.payload.len() >= 4 {
                    let sid = u32::from_be_bytes([
                        frame.payload[0], frame.payload[1],
                        frame.payload[2], frame.payload[3],
                    ]);
                    handler.on_session_online(sid).await;
                }
            }
            MSG_SESSION_OFFLINE => {
                // 客户端下线
                if frame.payload.len() >= 4 {
                    let sid = u32::from_be_bytes([
                        frame.payload[0], frame.payload[1],
                        frame.payload[2], frame.payload[3],
                    ]);
                    handler.on_session_offline(sid).await;
                }
            }
            MSG_SERVER_ONLINE => {
                // 其他服务上线
                if frame.payload.len() >= 5 {
                    let stype = frame.payload[0];
                    let iid = u32::from_be_bytes([
                        frame.payload[1], frame.payload[2],
                        frame.payload[3], frame.payload[4],
                    ]);
                    debug!("server online: type={}, instance={}", stype, iid);
                }
            }
            MSG_SERVER_OFFLINE => {
                // 其他服务下线
                if frame.payload.len() >= 5 {
                    let stype = frame.payload[0];
                    let iid = u32::from_be_bytes([
                        frame.payload[1], frame.payload[2],
                        frame.payload[3], frame.payload[4],
                    ]);
                    debug!("server offline: type={}, instance={}", stype, iid);
                }
            }
            _ => {
                // 业务消息：来自客户端的请求（经网关转发）
                // 每个请求独立 spawn，避免慢请求阻塞读循环
                let handler = handler.clone();
                tokio::spawn(async move {
                    handler.on_message(frame.msg_id, frame.serial, frame.session_id, frame.payload).await;
                });
            }
        }
    }

    /// 构建注册请求 payload: service_type(1) + instance_id(4)
    fn build_reg_req(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5);
        buf.push(SERVICE_TYPE_GAME);
        buf.extend_from_slice(&self.instance_id.to_be_bytes());
        buf
    }
}
