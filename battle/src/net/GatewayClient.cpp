#include "GatewayClient.h"
#include "yasio/ibstream.hpp"
#include <cstring>

using namespace std::string_view_literals;

GatewayClient::GatewayClient()
    : m_impl(nullptr)
    , m_connectionId(-1)
    , m_state(State::Disconnected)
    , m_running(false)
    , m_reconnectTimer(0.0f)
{
}

GatewayClient::~GatewayClient()
{
    stop();
    delete m_impl;
}

void GatewayClient::init(const Config& config)
{
    m_config = config;
    m_impl   = new YasioClient(1);
    m_impl->setEventCallback([this](int eventType, int id, const std::string_view& data) {
        this->onEvent(eventType, id, data);
    });
}

void GatewayClient::start()
{
    m_running        = true;
    m_reconnectTimer = 0.0f;
    m_connectionId   = m_impl->connect(m_config.host, m_config.port, yasio::YCK_TCP_CLIENT);
    m_state          = State::Connecting;
    AXLOGI("GatewayClient: connecting to %s:%d ...", m_config.host.c_str(), m_config.port);
}

void GatewayClient::stop()
{
    m_running = false;
    if (m_connectionId >= 0)
    {
        m_impl->disconnect(m_connectionId);
        m_connectionId = -1;
    }
    m_state = State::Disconnected;
}

void GatewayClient::update(float dt)
{
    if (!m_running)
        return;

    m_impl->poll();

    // 断线重连
    if (m_state == State::Disconnected)
    {
        tryReconnect(dt);
    }
}

void GatewayClient::tryReconnect(float dt)
{
    m_reconnectTimer += dt;
    if (m_reconnectTimer >= m_config.reconnectInterval)
    {
        m_reconnectTimer = 0.0f;
        m_connectionId   = m_impl->connect(m_config.host, m_config.port, yasio::YCK_TCP_CLIENT);
        m_state          = State::Connecting;
        AXLOGI("GatewayClient: reconnecting to %s:%d ...", m_config.host.c_str(), m_config.port);
    }
}

void GatewayClient::onEvent(int eventType, int id, const std::string_view& data)
{
    switch (eventType)
    {
    case YasioClient::OnConnectSuccess:
        m_connectionId = id;
        m_state        = State::Connected;
        AXLOGI("GatewayClient: connected, sending registration");
        sendRegisterReq();
        break;

    case YasioClient::OnConnectFailed:
        AXLOGW("GatewayClient: connect failed: %.*s", (int)data.size(), data.data());
        m_state          = State::Disconnected;
        m_connectionId   = -1;
        m_reconnectTimer = 0.0f;
        break;

    case YasioClient::OnDisconnect:
        AXLOGW("GatewayClient: disconnected: %.*s", (int)data.size(), data.data());
        m_state          = State::Disconnected;
        m_connectionId   = -1;
        m_reconnectTimer = 0.0f;
        if (m_onDisconnect)
            m_onDisconnect();
        break;

    case YasioClient::OnRecvData:
        onRecvFrame(data);
        break;
    }
}

void GatewayClient::onRecvFrame(const std::string_view& data)
{
    // yasio 已剥离 4 字节 len，剩余：[u16 msg_id][i32 serial][u32 session_id][payload...]
    if (data.size() < BACKEND_FRAME_BODY_HEADER_SIZE)
    {
        AXLOGE("GatewayClient: invalid frame size: %zu", data.size());
        return;
    }

    yasio::ibstream_view ibs(data.data(), static_cast<int>(data.size()));
    uint16_t msgId     = ibs.read<uint16_t>();
    int32_t serial     = ibs.read<int32_t>();
    uint32_t sessionId = ibs.read<uint32_t>();
    const char* payload = data.data() + BACKEND_FRAME_BODY_HEADER_SIZE;
    size_t payloadLen   = data.size() - BACKEND_FRAME_BODY_HEADER_SIZE;

    // 处理注册响应
    if (msgId == MSG_SERVER_REG_RESP)
    {
        if (payloadLen >= 1)
        {
            uint8_t code = static_cast<uint8_t>(payload[0]);
            if (code == 0)
            {
                m_state = State::Registered;
                AXLOGI("GatewayClient: registered successfully (service=%d, instance=%u)",
                       m_config.serviceType, m_config.instanceId);
            }
            else
            {
                AXLOGE("GatewayClient: registration failed, code=%d", code);
                stop();
            }
        }
        return;
    }

    // 其他消息转发给回调
    if (m_onMsgCallback)
    {
        m_onMsgCallback(msgId, serial, sessionId, std::string_view(payload, payloadLen));
    }
}

void GatewayClient::sendRegisterReq()
{
    // ServerRegReq: service_type(1) + instance_id(4) 大端序
    yasio::obstream payload;
    payload.write<uint8_t>(m_config.serviceType);
    payload.write<uint32_t>(m_config.instanceId);
    sendFrame(MSG_SERVER_REG_REQ, 0, 0, payload.data(), payload.length());
}

int GatewayClient::sendToClient(uint32_t sessionId, uint16_t msgId, int32_t serial,
                                const char* data, size_t length)
{
    return sendFrame(msgId, serial, sessionId, data, length);
}

int GatewayClient::kickSession(uint32_t sessionId)
{
    // KickSession: session_id(4) 大端序
    yasio::obstream payload;
    payload.write<uint32_t>(sessionId);
    return sendFrame(MSG_KICK_SESSION, 0, 0, payload.data(), payload.length());
}

int GatewayClient::sendFrame(uint16_t msgId, int32_t serial, uint32_t sessionId,
                             const char* data, size_t length)
{
    if (m_connectionId < 0)
        return -1;

    uint32_t frameLen = static_cast<uint32_t>(BACKEND_FRAME_HEADER_SIZE + length);

    yasio::obstream obs;
    obs.write<uint32_t>(frameLen);
    obs.write<uint16_t>(msgId);
    obs.write<int32_t>(serial);
    obs.write<uint32_t>(sessionId);
    if (length > 0)
        obs.write_bytes(data, static_cast<int>(length));

    return m_impl->send(m_connectionId, obs);
}
