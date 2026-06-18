#pragma once

#include "yasio/yasio.hpp"
#include <chrono>
#include <cstdint>
#include <functional>
#include <string>
#include <string_view>
#include <unordered_map>

// Backend frame: [u32 len][u8 cmd][u16 msg_id][i32 serial][u32 session_id][payload...]
constexpr size_t BACKEND_FRAME_HEADER_SIZE = 15;
// yasio strips len(4), leaving cmd(1) + msg_id(2) + serial(4) + session_id(4).
constexpr size_t BACKEND_FRAME_BODY_HEADER_SIZE = 11;

// cmd 只说明这一帧走哪条处理分支，业务协议号还是看 msg_id。
// 这里的数值要和 Rust 网关保持一致，改的时候两边一起改。
constexpr uint8_t CMD_BUSINESS = 0;  // 普通业务消息，payload 是业务 protobuf。
constexpr uint8_t CMD_GATEWAY_ERROR = 1;  // 网关返回给客户端的错误，后端一般不会处理。
constexpr uint8_t CMD_SERVER_STATUS = 2;  // 网关推给客户端的服务状态，后端一般不会处理。
constexpr uint8_t CMD_SERVER_REG_REQ = 10;  // 后端连上网关后发注册请求。
constexpr uint8_t CMD_SERVER_REG_RESP = 11;  // 网关返回注册结果。
constexpr uint8_t CMD_BIND_SERVICE = 12;  // 绑定会话到某类服务实例。
constexpr uint8_t CMD_UNBIND_SERVICE = 13;  // 取消会话的服务绑定。
constexpr uint8_t CMD_KICK_SESSION = 14;  // 后端要求网关踢掉客户端。
constexpr uint8_t CMD_SESSION_ONLINE = 15;  // 网关通知后端：客户端上线。
constexpr uint8_t CMD_SESSION_OFFLINE = 16;  // 网关通知后端：客户端下线。
constexpr uint8_t CMD_SERVER_ONLINE = 17;  // 网关通知后端：服务实例上线。
constexpr uint8_t CMD_SERVER_OFFLINE = 18;  // 网关通知后端：服务实例下线。
constexpr uint8_t CMD_FORWARD_TO_SERVER = 19;  // 后端让网关转发一帧到指定服务实例。

constexpr uint8_t SERVICE_ID_GAME   = 0;
constexpr uint8_t SERVICE_ID_BATTLE = 1;

typedef std::function<void(bool, const std::string_view&)> GatewayConnectCallback;
typedef std::function<void()> GatewayDisconnectCallback;
typedef std::function<void(uint8_t, uint16_t, int32_t, uint32_t, const std::string_view&)>
    GatewayMsgCallback;

class GatewayClient
{
public:
    struct Config
    {
        std::string host;
        int port;
        uint8_t serviceId;
        uint32_t instanceId;
        float reconnectInterval;
    };

    GatewayClient();
    ~GatewayClient();

    void init(const Config& config);
    void start();
    void stop();
    void update(float dt);

    bool isConnected() const { return m_state == State::Registered; }

    int sendToClient(uint32_t sessionId, uint16_t msgId, int32_t serial,
                     const char* data, size_t length);
    int kickSession(uint32_t sessionId);

    void setMsgCallback(const GatewayMsgCallback& callback) { m_onMsgCallback = callback; }
    void setDisconnectCallback(const GatewayDisconnectCallback& callback) { m_onDisconnect = callback; }

private:
    enum class State
    {
        Disconnected,
        Connecting,
        Connected,
        Registered,
    };

    void openConnection();
    void handleNetworkEvent(yasio::io_event* event);
    void tryReconnect(float dt);
    void onRecvFrame(const std::string_view& data);
    void sendRegisterReq();
    int sendFrame(uint8_t cmd, uint16_t msgId, int32_t serial, uint32_t sessionId,
                  const char* data, size_t length);

private:
    Config m_config;
    yasio::io_service m_service;
    yasio::transport_handle_t m_transport;
    State m_state;
    bool m_running;
    float m_reconnectTimer;
    GatewayMsgCallback m_onMsgCallback;
    GatewayDisconnectCallback m_onDisconnect;
};
