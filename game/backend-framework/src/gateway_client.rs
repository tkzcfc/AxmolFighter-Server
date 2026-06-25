use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use protocol::gateway::ServerRegReq;
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

const REGISTER_SERIAL: i32 = -1;
const REGISTER_TIMEOUT: Duration = Duration::from_secs(5);

pub type GatewaySender = mpsc::UnboundedSender<Bytes>;

pub struct GatewayClient {
    addr: String,
    service_id: u32,
    instance_id: u32,
    reconnect_interval: Duration,
    shutdown: CancellationToken,
}

impl GatewayClient {
    pub fn new(
        addr: String,
        service_id: u32,
        instance_id: u32,
        reconnect_interval: Duration,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            addr,
            service_id,
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

            info!("connecting to gateway at {}...", self.addr);
            match TcpStream::connect(&self.addr).await {
                Ok(mut stream) => {
                    info!("connected to gateway at {}", self.addr);
                    match self.register_stream(&mut stream).await {
                        Err(err) => {
                            warn!("gateway registration failed: {}", err);
                            drop(stream);
                            warn!(
                                "disconnected from gateway, reconnecting in {}s...",
                                self.reconnect_interval.as_secs()
                            );
                        }
                        Ok(remaining_buf) => {
                            info!("registered to gateway successfully");

                            let (tx, rx) = mpsc::unbounded_channel::<Bytes>();
                            handler.on_gateway_connected(tx.clone());

                            self.run_connection(stream, rx, tx, handler.clone(), remaining_buf)
                                .await;

                            handler.on_gateway_disconnected();
                            warn!(
                                "disconnected from gateway, reconnecting in {}s...",
                                self.reconnect_interval.as_secs()
                            );
                        }
                    }
                }
                Err(err) => {
                    error!(
                        "failed to connect gateway at {}: {}, retrying in {}s...",
                        self.addr,
                        err,
                        self.reconnect_interval.as_secs()
                    );
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
        initial_buf: BytesMut,
    ) {
        let (reader, writer) = stream.into_split();

        tokio::select! {
            result = self.read_loop(reader, tx, handler, initial_buf) => {
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

    /// 开始读循环
    ///
    /// `buf` 是注册后读取的多余数据(只有在极端情况下才会出现),从 register_stream() 返回结果传入
    async fn read_loop(
        &self,
        mut reader: OwnedReadHalf,
        tx: GatewaySender,
        handler: Arc<dyn MessageHandler>,
        mut buf: BytesMut,
    ) -> anyhow::Result<()> {
        loop {
            if reader.read_buf(&mut buf).await? == 0 {
                warn!("gateway connection EOF");
                return Ok(());
            }

            while let Some(frame) = try_extract_frame(&mut buf)? {
                self.dispatch_frame(frame, &tx, &handler);
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

    /// 向网关注册，返回残留的未完成字节缓冲（注册成功后立即退出，后续帧仍在 buf 中）。
    async fn register_stream(&self, stream: &mut TcpStream) -> anyhow::Result<BytesMut> {
        let reg_msg = MessageType::GatewayServerRegReq(ServerRegReq {
            service_id: self.service_id,
            instance_id: self.instance_id,
            load_score: 0,
            accepting_bindings: true,
            load_message: String::new(),
        });
        let (reg_msg_id, reg_payload) = encode_message(&reg_msg)
            .ok_or_else(|| anyhow::anyhow!("failed to encode register message"))?;
        let reg_msg_id = reg_msg_id as u16;
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
            self.service_id, self.instance_id, reg_msg_id
        );

        let mut buf = BytesMut::with_capacity(8192);

        tokio::time::timeout(REGISTER_TIMEOUT, async {
            loop {
                if stream.read_buf(&mut buf).await? == 0 {
                    anyhow::bail!("gateway connection closed during registration");
                }

                while let Some(frame) = try_extract_frame(&mut buf)? {
                    if frame.cmd != CMD_GATEWAY_CONTROL {
                        warn!("ignored frame before registration cmd={}", frame.cmd);
                        continue;
                    }

                    match decode_message(frame.msg_id as u32, &frame.payload)? {
                        MessageType::GatewayServerRegResp(resp) => {
                            if resp.code == 0 {
                                return Ok(());
                            }
                            anyhow::bail!("gateway registration failed, code={}", resp.code);
                        }
                        _ => {
                            warn!("ignored pre-registration control msg_id={}", frame.msg_id);
                        }
                    }
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("gateway registration timeout"))??;

        Ok(buf)
    }

    fn dispatch_frame(
        &self,
        frame: BackendFrame,
        _tx: &GatewaySender,
        handler: &Arc<dyn MessageHandler>,
    ) {
        match frame.cmd {
            CMD_GATEWAY_CONTROL => handler.on_gateway_control_frame(frame),
            CMD_BUSINESS => handler.on_business_frame(frame),
            _ => {
                debug!("unhandled gateway cmd={}", frame.cmd);
            }
        }
    }
}
