use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use protocol::gateway::ServerRegReq;
use protocol::message_map::{MessageType, decode_message, encode_message};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use crate::codec::{BackendFrame, encode_frame, try_extract_frame};
use crate::handler::MessageHandler;

// cmd 只描述帧类别；具体网关控制消息类型由 msg_id 对应的 PB 协议号决定。
const CMD_BUSINESS: u8 = 0;
const CMD_GATEWAY_CONTROL: u8 = 2;

const SERVICE_ID_GAME: u8 = 0;

pub type GatewaySender = mpsc::UnboundedSender<Bytes>;

pub struct GatewayClient {
    addr: String,
    instance_id: u32,
    reconnect_interval: Duration,
    shutdown: watch::Receiver<bool>,
}

impl GatewayClient {
    pub fn new(
        addr: String,
        instance_id: u32,
        reconnect_interval: Duration,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self {
            addr,
            instance_id,
            reconnect_interval,
            shutdown,
        }
    }

    // 启动连接循环，断线后按配置间隔自动重连。
    pub async fn run(mut self, handler: Arc<dyn MessageHandler>) {
        loop {
            if *self.shutdown.borrow() {
                info!("gateway client shutting down");
                return;
            }

            match TcpStream::connect(&self.addr).await {
                Ok(stream) => {
                    info!("connected to gateway at {}", self.addr);
                    let (tx, rx) = mpsc::unbounded_channel::<Bytes>();

                    handler.on_gateway_connected(tx.clone());

                    let (reg_msg_id, reg_payload) = self.build_reg_req();
                    let reg_frame =
                        encode_frame(CMD_GATEWAY_CONTROL, reg_msg_id, 0, 0, &reg_payload);
                    let _ = tx.send(reg_frame);

                    self.run_connection(stream, rx, handler.clone()).await;

                    handler.on_gateway_disconnected();
                    warn!(
                        "disconnected from gateway, reconnecting in {}s...",
                        self.reconnect_interval.as_secs()
                    );
                }
                Err(err) => {
                    error!("failed to connect gateway at {}: {}", self.addr, err);
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(self.reconnect_interval) => {}
                _ = self.shutdown.changed() => {
                    info!("gateway client shutting down");
                    return;
                }
            }
        }
    }

    async fn run_connection(
        &self,
        stream: TcpStream,
        mut rx: mpsc::UnboundedReceiver<Bytes>,
        handler: Arc<dyn MessageHandler>,
    ) {
        let (mut reader, mut writer) = stream.into_split();
        let mut shutdown = self.shutdown.clone();

        let mut write_shutdown = self.shutdown.clone();
        let write_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Some(data) => {
                                if let Err(err) = writer.write_all(&data).await {
                                    debug!("write error: {}", err);
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    _ = write_shutdown.changed() => break,
                }
            }
        });

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
                            while let Ok(Some(frame)) = try_extract_frame(&mut buf) {
                                self.dispatch_frame(frame, &handler).await;
                            }
                        }
                        Err(err) => {
                            debug!("read error: {}", err);
                            break;
                        }
                    }
                }
                _ = shutdown.changed() => break,
            }
        }

        write_task.abort();
    }

    async fn dispatch_frame(&self, frame: BackendFrame, handler: &Arc<dyn MessageHandler>) {
        match frame.cmd {
            CMD_GATEWAY_CONTROL => self.dispatch_control_frame(frame, handler).await,
            CMD_BUSINESS => {
                let handler = handler.clone();
                tokio::spawn(async move {
                    handler
                        .on_message(frame.msg_id, frame.serial, frame.session_id, frame.payload)
                        .await;
                });
            }
            _ => {
                debug!("unhandled gateway cmd={}", frame.cmd);
            }
        }
    }

    async fn dispatch_control_frame(&self, frame: BackendFrame, handler: &Arc<dyn MessageHandler>) {
        match decode_message(frame.msg_id as u32, &frame.payload) {
            Ok(MessageType::GatewayServerRegResp(resp)) => {
                if resp.code == 0 {
                    info!("registered to gateway successfully");
                } else {
                    error!("gateway registration failed, code={}", resp.code);
                }
            }
            Ok(MessageType::GatewaySessionOnlinePush(push)) => {
                handler.on_session_online(push.session_id).await;
            }
            Ok(MessageType::GatewaySessionOfflinePush(push)) => {
                handler.on_session_offline(push.session_id).await;
            }
            Ok(MessageType::GatewayServerOnlinePush(push)) => {
                debug!(
                    "server online: type={}, instance={}",
                    push.service_id, push.instance_id
                );
            }
            Ok(MessageType::GatewayServerOfflinePush(push)) => {
                debug!(
                    "server offline: type={}, instance={}",
                    push.service_id, push.instance_id
                );
            }
            Ok(_) => {
                debug!("unhandled gateway control msg_id={}", frame.msg_id);
            }
            Err(err) => {
                debug!(
                    "failed to decode gateway control msg_id={}: {}",
                    frame.msg_id, err
                );
            }
        }
    }

    fn build_reg_req(&self) -> (u16, Vec<u8>) {
        let message = MessageType::GatewayServerRegReq(ServerRegReq {
            service_id: SERVICE_ID_GAME as u32,
            instance_id: self.instance_id,
        });
        let (msg_id, payload) = encode_message(&message).unwrap();
        (msg_id as u16, payload)
    }
}
