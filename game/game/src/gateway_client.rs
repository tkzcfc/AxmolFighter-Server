use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use protocol::gateway::{ServerPongResp, ServerRegReq};
use protocol::message_map::{MessageType, decode_message, encode_message};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::codec::{BackendFrame, encode_frame, try_extract_frame};
use crate::handler::MessageHandler;
use crate::wire::{CMD_BUSINESS, CMD_GATEWAY_CONTROL};

const SERVICE_ID_GAME: u8 = 0;
const REGISTER_SERIAL: i32 = -1;
const REGISTER_TIMEOUT: Duration = Duration::from_secs(10);

pub type GatewaySender = mpsc::UnboundedSender<Bytes>;

pub struct GatewayClient {
    addr: String,
    instance_id: u32,
    reconnect_interval: Duration,
    shutdown: CancellationToken,
}

impl GatewayClient {
    pub fn new(
        addr: String,
        instance_id: u32,
        reconnect_interval: Duration,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            addr,
            instance_id,
            reconnect_interval,
            shutdown,
        }
    }

    // 启动连接循环，断线后按配置间隔自动重连。
    pub async fn run(self, handler: Arc<dyn MessageHandler>) {
        loop {
            if self.shutdown.is_cancelled() {
                info!("gateway client shutting down");
                return;
            }

            match TcpStream::connect(&self.addr).await {
                Ok(mut stream) => {
                    info!("connected to gateway at {}", self.addr);
                    if let Err(err) = self.register_stream(&mut stream).await {
                        warn!("gateway registration failed: {}", err);
                        drop(stream);
                        warn!(
                            "disconnected from gateway, reconnecting in {}s...",
                            self.reconnect_interval.as_secs()
                        );
                    } else {
                        info!("registered to gateway successfully");

                        let (tx, rx) = mpsc::unbounded_channel::<Bytes>();
                        handler.on_gateway_connected(tx.clone());

                        self.run_connection(stream, rx, tx, handler.clone()).await;

                        handler.on_gateway_disconnected();
                        warn!(
                            "disconnected from gateway, reconnecting in {}s...",
                            self.reconnect_interval.as_secs()
                        );
                    }
                }
                Err(err) => {
                    error!("failed to connect gateway at {}: {}", self.addr, err);
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(self.reconnect_interval) => {}
                _ = self.shutdown.cancelled() => {
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
        tx: GatewaySender,
        handler: Arc<dyn MessageHandler>,
    ) {
        let (reader, writer) = stream.into_split();

        tokio::select! {
            result = self.read_loop(reader, tx, handler) => {
                if let Err(err) = result {
                    debug!("gateway read loop ended: {}", err);
                }
            }
            result = self.write_loop(writer, &mut rx) => {
                if let Err(err) = result {
                    debug!("gateway write loop ended: {}", err);
                }
            }
            _ = self.shutdown.cancelled() => {}
        }
    }

    async fn read_loop(
        &self,
        mut reader: OwnedReadHalf,
        tx: GatewaySender,
        handler: Arc<dyn MessageHandler>,
    ) -> anyhow::Result<()> {
        let mut buf = BytesMut::with_capacity(8192);
        loop {
            if reader.read_buf(&mut buf).await? == 0 {
                debug!("gateway connection EOF");
                return Ok(());
            }

            while let Some(frame) = try_extract_frame(&mut buf)? {
                self.dispatch_frame(frame, &tx, &handler).await;
            }
        }
    }

    async fn write_loop(
        &self,
        mut writer: OwnedWriteHalf,
        rx: &mut mpsc::UnboundedReceiver<Bytes>,
    ) -> anyhow::Result<()> {
        while let Some(data) = rx.recv().await {
            writer.write_all(&data).await?;
        }

        Ok(())
    }

    async fn register_stream(&self, stream: &mut TcpStream) -> anyhow::Result<()> {
        let (reg_msg_id, reg_payload) = self.build_reg_req();
        let reg_frame = encode_frame(
            CMD_GATEWAY_CONTROL,
            reg_msg_id,
            REGISTER_SERIAL,
            0,
            &reg_payload,
        );
        stream.write_all(&reg_frame).await?;
        stream.flush().await?;
        info!(
            "sent gateway register request service_id={} instance_id={} msg_id={}",
            SERVICE_ID_GAME, self.instance_id, reg_msg_id
        );

        let mut buf = BytesMut::with_capacity(8192);
        tokio::time::timeout(REGISTER_TIMEOUT, async {
            loop {
                if stream.read_buf(&mut buf).await? == 0 {
                    anyhow::bail!("gateway connection closed during registration");
                }

                while let Some(frame) = try_extract_frame(&mut buf)? {
                    if frame.cmd != CMD_GATEWAY_CONTROL {
                        debug!("ignored frame before registration cmd={}", frame.cmd);
                        continue;
                    }

                    match decode_message(frame.msg_id as u32, &frame.payload)? {
                        MessageType::GatewayServerRegResp(resp) => {
                            if resp.code == 0 {
                                return Ok(());
                            }
                            anyhow::bail!("gateway registration failed, code={}", resp.code);
                        }
                        MessageType::GatewayServerPingReq(ping) => {
                            let message = self.build_pong_resp(ping.nonce);
                            if let Some((msg_id, payload)) = encode_message(&message) {
                                let data = encode_frame(
                                    CMD_GATEWAY_CONTROL,
                                    msg_id as u16,
                                    0,
                                    0,
                                    &payload,
                                );
                                stream.write_all(&data).await?;
                                stream.flush().await?;
                            }
                        }
                        _ => {
                            debug!("ignored pre-registration control msg_id={}", frame.msg_id);
                        }
                    }
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("gateway registration timeout"))?
    }

    async fn dispatch_frame(
        &self,
        frame: BackendFrame,
        tx: &GatewaySender,
        handler: &Arc<dyn MessageHandler>,
    ) {
        match frame.cmd {
            CMD_GATEWAY_CONTROL => self.dispatch_control_frame(frame, tx, handler).await,
            CMD_BUSINESS => {
                handler
                    .on_message(frame.msg_id, frame.serial, frame.session_id, frame.payload)
                    .await;
            }
            _ => {
                debug!("unhandled gateway cmd={}", frame.cmd);
            }
        }
    }

    async fn dispatch_server_forward(
        &self,
        req: protocol::gateway::ForwardToServerReq,
        handler: &Arc<dyn MessageHandler>,
    ) {
        let mut buf = BytesMut::from(req.payload.as_slice());
        let Ok(Some(inner)) = try_extract_frame(&mut buf) else {
            debug!(
                "invalid forwarded backend frame from service_id={} instance_id={}",
                req.source_service_id, req.source_instance_id
            );
            return;
        };

        match inner.cmd {
            CMD_BUSINESS => {
                handler
                    .on_message(inner.msg_id, inner.serial, inner.session_id, inner.payload)
                    .await;
            }
            CMD_GATEWAY_CONTROL => {
                debug!("ignored forwarded gateway control msg_id={}", inner.msg_id)
            }
            _ => debug!("unhandled forwarded cmd={}", inner.cmd),
        }
    }

    async fn dispatch_control_frame(
        &self,
        frame: BackendFrame,
        tx: &GatewaySender,
        handler: &Arc<dyn MessageHandler>,
    ) {
        match decode_message(frame.msg_id as u32, &frame.payload) {
            Ok(MessageType::GatewayGatewayErrorResp(resp)) => {
                handler.on_gateway_error_resp(frame.serial, resp).await;
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
            Ok(MessageType::GatewayBindServiceResp(resp)) => {
                handler.on_bind_service_resp(frame.serial, resp).await;
            }
            Ok(MessageType::GatewayUnbindServiceResp(resp)) => {
                handler.on_unbind_service_resp(frame.serial, resp).await;
            }
            Ok(MessageType::GatewayForwardToServerReq(req)) => {
                self.dispatch_server_forward(req, handler).await;
            }
            Ok(MessageType::GatewayServerPingReq(ping)) => {
                let message =
                    MessageType::GatewayServerPongResp(ServerPongResp { nonce: ping.nonce });
                if let Some((msg_id, payload)) = encode_message(&message) {
                    let data = encode_frame(CMD_GATEWAY_CONTROL, msg_id as u16, 0, 0, &payload);
                    let _ = tx.send(data);
                }
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
            load_score: 0,
            accepting_bindings: true,
            load_message: String::new(),
        });
        let (msg_id, payload) = encode_message(&message).unwrap();
        (msg_id as u16, payload)
    }

    fn build_pong_resp(&self, nonce: u64) -> MessageType {
        MessageType::GatewayServerPongResp(ServerPongResp { nonce })
    }
}
