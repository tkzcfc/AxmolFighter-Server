#include "GatewayClient.h"
#include "yasio/ibstream.hpp"
#include "yasio/obstream.hpp"
#include <cstring>
#include <cstdio>

#ifndef AXLOGI
#    define AXLOGI(fmt, ...) printf("[I] " fmt "\n", ##__VA_ARGS__)
#endif
#ifndef AXLOGW
#    define AXLOGW(fmt, ...) fprintf(stderr, "[W] " fmt "\n", ##__VA_ARGS__)
#endif
#ifndef AXLOGE
#    define AXLOGE(fmt, ...) fprintf(stderr, "[E] " fmt "\n", ##__VA_ARGS__)
#endif

using namespace std::string_view_literals;

GatewayClient::GatewayClient()
    : m_service(1)
    , m_transport(nullptr)
    , m_state(State::Disconnected)
    , m_running(false)
    , m_reconnectTimer(0.0f)
{
    m_service.set_option(yasio::YOPT_S_CONNECT_TIMEOUT, 5);
    m_service.set_option(yasio::YOPT_S_DNS_QUERIES_TIMEOUT, 3);
    m_service.set_option(yasio::YOPT_S_DNS_QUERIES_TRIES, 1);
    m_service.start([this](yasio::event_ptr&& e) {
        this->handleNetworkEvent(e.get());
    });
}

GatewayClient::~GatewayClient()
{
    stop();
}

void GatewayClient::init(const Config& config)
{
    m_config = config;
}

void GatewayClient::start()
{
    m_running        = true;
    m_reconnectTimer = 0.0f;
    openConnection();
}

void GatewayClient::stop()
{
    m_running = false;
    if (m_transport)
    {
        m_service.close(m_transport);
        m_transport = nullptr;
    }
    m_state = State::Disconnected;
}

void GatewayClient::update(float dt)
{
    if (!m_running)
        return;

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
        openConnection();
    }
}

void GatewayClient::openConnection()
{
    m_state     = State::Connecting;
    m_transport = nullptr;

    m_service.set_option(yasio::YOPT_C_UNPACK_PARAMS, 0, 1024 * 1024 * 10, 0, 4, 0);
    m_service.set_option(yasio::YOPT_C_UNPACK_STRIP, 0, 4);
    m_service.set_option(yasio::YOPT_C_UNPACK_NO_BSWAP, 0, 0);
    m_service.set_option(yasio::YOPT_C_REMOTE_ENDPOINT, 0, m_config.host.data(), m_config.port);

    AXLOGI("GatewayClient: connecting to %s:%d ...", m_config.host.c_str(), m_config.port);
    m_service.open(0, yasio::YCK_TCP_CLIENT);
}

void GatewayClient::handleNetworkEvent(yasio::io_event* event)
{
    switch (event->kind())
    {
    case yasio::YEK_ON_OPEN:
        if (event->status() == 0)
        {
            m_transport = event->transport();
            m_state     = State::Connected;
            AXLOGI("GatewayClient: connected, sending registration");
            sendRegisterReq();
        }
        else
        {
            AXLOGW("GatewayClient: connect failed, internal error code: %d", event->status());
            m_transport      = nullptr;
            m_state          = State::Disconnected;
            m_reconnectTimer = 0.0f;
        }
        break;

    case yasio::YEK_ON_CLOSE:
        AXLOGW("GatewayClient: disconnected, internal error code: %d", event->status());
        m_transport      = nullptr;
        m_state          = State::Disconnected;
        m_reconnectTimer = 0.0f;
        if (m_running && m_onDisconnect)
            m_onDisconnect();
        break;

    case yasio::YEK_ON_PACKET:
    {
        auto& packet = event->packet();
        onRecvFrame(std::string_view(packet.data(), packet.size()));
        break;
    }

    default:
        break;
    }
}

void GatewayClient::onRecvFrame(const std::string_view& data)
{
    // yasio strips len(4), leaving [u8 cmd][u16 msg_id][i32 serial][u32 session_id][payload...].
    if (data.size() < BACKEND_FRAME_BODY_HEADER_SIZE)
    {
        AXLOGE("GatewayClient: invalid frame size: %zu", data.size());
        return;
    }

    yasio::ibstream_view ibs(data.data(), static_cast<int>(data.size()));
    uint8_t cmd        = ibs.read<uint8_t>();
    uint16_t msgId     = ibs.read<uint16_t>();
    int32_t serial     = ibs.read<int32_t>();
    uint32_t sessionId = ibs.read<uint32_t>();
    const char* payload = data.data() + BACKEND_FRAME_BODY_HEADER_SIZE;
    size_t payloadLen   = data.size() - BACKEND_FRAME_BODY_HEADER_SIZE;

    // 处理注册响应
    if (cmd == CMD_SERVER_REG_RESP)
    {
        if (payloadLen >= 1)
        {
            uint8_t code = static_cast<uint8_t>(payload[0]);
            if (code == 0)
            {
                m_state = State::Registered;
                AXLOGI("GatewayClient: registered successfully (service=%d, instance=%u)",
                       m_config.serviceId, m_config.instanceId);
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
        m_onMsgCallback(cmd, msgId, serial, sessionId, std::string_view(payload, payloadLen));
    }
}

void GatewayClient::sendRegisterReq()
{
    // ServerRegReq: service_id(1) + instance_id(4) big-endian
    yasio::obstream payload;
    payload.write<uint8_t>(m_config.serviceId);
    payload.write<uint32_t>(m_config.instanceId);
    sendFrame(CMD_SERVER_REG_REQ, 0, 0, 0, payload.data(), payload.length());
}

int GatewayClient::sendToClient(uint32_t sessionId, uint16_t msgId, int32_t serial,
                                const char* data, size_t length)
{
    return sendFrame(CMD_BUSINESS, msgId, serial, sessionId, data, length);
}

int GatewayClient::kickSession(uint32_t sessionId)
{
    // KickSession: session_id(4) 大端序
    yasio::obstream payload;
    payload.write<uint32_t>(sessionId);
    return sendFrame(CMD_KICK_SESSION, 0, 0, 0, payload.data(), payload.length());
}

int GatewayClient::sendFrame(uint8_t cmd, uint16_t msgId, int32_t serial, uint32_t sessionId,
                             const char* data, size_t length)
{
    if (!m_transport)
        return -1;

    uint32_t frameLen = static_cast<uint32_t>(BACKEND_FRAME_HEADER_SIZE + length);

    yasio::obstream obs;
    obs.write<uint32_t>(frameLen);
    obs.write<uint8_t>(cmd);
    obs.write<uint16_t>(msgId);
    obs.write<int32_t>(serial);
    obs.write<uint32_t>(sessionId);
    if (length > 0)
        obs.write_bytes(data, static_cast<int>(length));

    return m_service.write(m_transport, std::move(obs.buffer()));
}
