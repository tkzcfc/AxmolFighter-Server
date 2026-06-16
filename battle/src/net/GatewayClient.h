#pragma once

#include "YasioClient.h"
#include <unordered_map>
#include <functional>
#include <chrono>

// 后端帧格式（Gateway ↔ Backend）：
// [u32 len][u16 msg_id][i32 serial][u32 session_id][payload...]
// len 包含帧头自身（14字节）
constexpr size_t BACKEND_FRAME_HEADER_SIZE = 14;
// yasio 剥离 len(4字节) 后，剩余帧体头大小
constexpr size_t BACKEND_FRAME_BODY_HEADER_SIZE = 10;  // msg_id(2) + serial(4) + session_id(4)

// 网关内部协议消息 ID
constexpr uint16_t MSG_SERVER_REG_REQ    = 10001;
constexpr uint16_t MSG_SERVER_REG_RESP   = 10002;
constexpr uint16_t MSG_BIND_BATTLE       = 10003;
constexpr uint16_t MSG_UNBIND_BATTLE     = 10004;
constexpr uint16_t MSG_KICK_SESSION      = 10005;
constexpr uint16_t MSG_SESSION_ONLINE    = 10006;
constexpr uint16_t MSG_SESSION_OFFLINE   = 10007;
constexpr uint16_t MSG_SERVER_ONLINE     = 10008;
constexpr uint16_t MSG_SERVER_OFFLINE    = 10009;
constexpr uint16_t MSG_FORWARD_TO_SERVER = 10010;
constexpr uint16_t MSG_SERVER_STATUS     = 10011;

// 服务类型
constexpr uint8_t SERVICE_TYPE_GAME   = 1;
constexpr uint8_t SERVICE_TYPE_BATTLE = 2;

/// 连接回调
typedef std::function<void(bool, const std::string_view&)> GatewayConnectCallback;
/// 断线回调
typedef std::function<void()> GatewayDisconnectCallback;
/// 消息回调：msg_id, serial, session_id, payload
typedef std::function<void(uint16_t, int32_t, uint32_t, const std::string_view&)> GatewayMsgCallback;

/// 网关客户端（后端服务使用）
/// 连接到网关内部端口，使用 14 字节帧格式
class GatewayClient
{
public:
    struct Config
    {
        std::string host;
        int port;
        uint8_t serviceType;
        uint32_t instanceId;
        float reconnectInterval;  // 秒
    };

    GatewayClient();
    ~GatewayClient();

    void init(const Config& config);

    /// 连接网关（自动注册+断线重连）
    void start();

    /// 断开连接
    void stop();

    /// 驱动网络事件（需要外部循环调用）
    void update(float dt);

    bool isConnected() const { return m_state == State::Registered; }

    /// 发送消息给指定客户端
    int sendToClient(uint32_t sessionId, uint16_t msgId, int32_t serial,
                     const char* data, size_t length);

    /// 踢出客户端
    int kickSession(uint32_t sessionId);

    /// 设置消息回调
    void setMsgCallback(const GatewayMsgCallback& callback) { m_onMsgCallback = callback; }

    /// 设置断线回调
    void setDisconnectCallback(const GatewayDisconnectCallback& callback) { m_onDisconnect = callback; }

private:
    void onEvent(int eventType, int id, const std::string_view& data);
    void onRecvFrame(const std::string_view& data);
    void sendRegisterReq();
    int sendFrame(uint16_t msgId, int32_t serial, uint32_t sessionId,
                  const char* data, size_t length);
    void tryReconnect(float dt);

    enum State : uint8_t
    {
        Disconnected,
        Connecting,
        Connected,      // TCP 已连接，等待注册响应
        Registered,     // 注册成功
    };

private:
    YasioClient* m_impl;
    Config m_config;
    int m_connectionId;
    State m_state;
    bool m_running;
    float m_reconnectTimer;

    GatewayMsgCallback m_onMsgCallback;
    GatewayDisconnectCallback m_onDisconnect;
};
