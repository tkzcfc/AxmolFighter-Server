#include "GatewayClient.h"
#include "gateway.pb.h"
#include "yasio/ibstream.hpp"
#include "yasio/obstream.hpp"
#include <exception>
#include <cstdio>
#include <cstring>

#ifndef AXLOGI
#    define AXLOGI(fmt, ...) printf("[I] " fmt "\n", ##__VA_ARGS__)
#endif
#ifndef AXLOGW
#    define AXLOGW(fmt, ...) fprintf(stderr, "[W] " fmt "\n", ##__VA_ARGS__)
#endif
#ifndef AXLOGE
#    define AXLOGE(fmt, ...) fprintf(stderr, "[E] " fmt "\n", ##__VA_ARGS__)
#endif

namespace
{
void defaultRequestCallback(bool, const std::string_view&) {}

void writeUint16(std::string& data, size_t offset, uint16_t value)
{
    data[offset] = static_cast<char>((value >> 8) & 0xFF);
    data[offset + 1] = static_cast<char>(value & 0xFF);
}

void writeUint32(std::string& data, size_t offset, uint32_t value)
{
    data[offset] = static_cast<char>((value >> 24) & 0xFF);
    data[offset + 1] = static_cast<char>((value >> 16) & 0xFF);
    data[offset + 2] = static_cast<char>((value >> 8) & 0xFF);
    data[offset + 3] = static_cast<char>(value & 0xFF);
}
}

GatewayClient::GatewayClient()
    : m_service(1)
    , m_transport(nullptr)
    , m_state(State::Disconnected)
    , m_running(false)
    , m_reconnectTimer(0.0f)
    , m_serialCounter(0)
{
    m_service.set_option(yasio::YOPT_S_CONNECT_TIMEOUT, 5);
    m_service.set_option(yasio::YOPT_S_DNS_QUERIES_TIMEOUT, 3);
    m_service.set_option(yasio::YOPT_S_DNS_QUERIES_TRIES, 1);
    m_service.start([this](yasio::event_ptr&& e) { this->handleNetworkEvent(e.get()); });
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
    m_running = true;
    m_reconnectTimer = 0.0f;
    openConnection();
}

void GatewayClient::stop()
{
    m_running = false;
    failAllPendingRequests("disconnected");
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

    timeoutCheck(dt);

    if (m_state == State::Disconnected)
        tryReconnect(dt);
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
    m_state = State::Connecting;
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
            m_state = State::Connected;
            m_serialCounter = 0;
            AXLOGI("GatewayClient: connected, sending registration");
            sendRegisterReq();
        }
        else
        {
            AXLOGW("GatewayClient: connect failed, internal error code: %d", event->status());
            m_transport = nullptr;
            m_state = State::Disconnected;
            m_reconnectTimer = 0.0f;
        }
        break;

    case yasio::YEK_ON_CLOSE:
        AXLOGW("GatewayClient: disconnected, internal error code: %d", event->status());
        m_transport = nullptr;
        m_state = State::Disconnected;
        m_reconnectTimer = 0.0f;
        failAllPendingRequests("connection lost");
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
    if (data.size() < BACKEND_FRAME_BODY_HEADER_SIZE)
    {
        AXLOGE("GatewayClient: invalid frame size: %zu", data.size());
        return;
    }

    yasio::ibstream_view ibs(data.data(), static_cast<int>(data.size()));
    uint8_t cmd = ibs.read<uint8_t>();
    uint16_t msgId = ibs.read<uint16_t>();
    int32_t serial = ibs.read<int32_t>();
    uint32_t sessionId = ibs.read<uint32_t>();
    const char* payload = data.data() + BACKEND_FRAME_BODY_HEADER_SIZE;
    size_t payloadLen = data.size() - BACKEND_FRAME_BODY_HEADER_SIZE;

    if (cmd == CMD_GATEWAY_CONTROL && msgId == PB::Gateway::ServerRegResp::Id)
    {
        PB::Gateway::ServerRegResp resp;
        if (resp.ParseFromArray(payload, static_cast<int>(payloadLen)))
        {
            if (resp.code() == 0)
            {
                m_state = State::Registered;
                AXLOGI("GatewayClient: registered successfully (service=%d, instance=%u)",
                       m_config.serviceId, m_config.instanceId);
                if (m_onConnect)
                    m_onConnect(true, "");
            }
            else
            {
                AXLOGE("GatewayClient: registration failed, code=%u", resp.code());
                stop();
            }
        }
        return;
    }

    if (cmd == CMD_GATEWAY_CONTROL && msgId == PB::Gateway::ServerPingReq::Id)
    {
        PB::Gateway::ServerPingReq ping;
        if (ping.ParseFromArray(payload, static_cast<int>(payloadLen)))
        {
            PB::Gateway::ServerPongResp pong;
            pong.set_nonce(ping.nonce());
            sendControl(0, 0, pong);
        }
        return;
    }

    if (serial > 0)
    {
        auto it = m_pendingRequests.find(serial);
        if (it != m_pendingRequests.end())
        {
            auto callback = std::move(it->second.callback);
            m_pendingRequests.erase(it);
            callback(cmd == CMD_BUSINESS, std::string_view(payload, payloadLen));
            return;
        }
    }

    if (m_onMsgCallback)
        m_onMsgCallback(cmd, msgId, serial, sessionId, std::string_view(payload, payloadLen));
}

void GatewayClient::sendRegisterReq()
{
    PB::Gateway::ServerRegReq req;
    req.set_service_id(m_config.serviceId);
    req.set_instance_id(m_config.instanceId);
    req.set_load_score(m_config.initialLoadScore > 100 ? 100 : m_config.initialLoadScore);
    req.set_accepting_bindings(m_config.initialAcceptingBindings);
    req.set_load_message(m_config.initialLoadMessage);
    std::string payload;
    req.SerializeToString(&payload);
    sendFrame(CMD_GATEWAY_CONTROL, PB::Gateway::ServerRegReq::Id, 0, 0, payload.data(), payload.size());
}

int32_t GatewayClient::sendRequest(uint32_t sessionId, uint16_t msgId, const char* data,
                                   size_t length, const GatewayRequestCallback& callback,
                                   float timeout)
{
    --m_serialCounter;
    int32_t serial = m_serialCounter;
    int32_t requestId = -serial;

    PendingRequest req;
    req.callback = callback ? callback : defaultRequestCallback;
    req.timeout = timeout;
    m_pendingRequests.emplace(requestId, std::move(req));

    if (sendFrame(CMD_BUSINESS, msgId, serial, sessionId, data, length) < 0)
    {
        auto it = m_pendingRequests.find(requestId);
        if (it != m_pendingRequests.end())
        {
            auto cb = std::move(it->second.callback);
            m_pendingRequests.erase(it);
            cb(false, "send failed");
        }
    }

    return requestId;
}

void GatewayClient::cancelRequest(int32_t requestId)
{
    m_pendingRequests.erase(requestId);
}

void GatewayClient::sendPush(uint32_t sessionId, uint16_t msgId, const char* data, size_t length)
{
    sendFrame(CMD_BUSINESS, msgId, 0, sessionId, data, length);
}

void GatewayClient::sendResponse(uint32_t sessionId, uint16_t msgId, int32_t serial,
                                 const char* data, size_t length)
{
    sendFrame(CMD_BUSINESS, msgId, serial, sessionId, data, length);
}

int GatewayClient::sendToClient(uint32_t sessionId, uint16_t msgId, int32_t serial,
                                const char* data, size_t length)
{
    return sendFrame(CMD_BUSINESS, msgId, serial, sessionId, data, length);
}

int GatewayClient::bindService(uint32_t sessionId, uint8_t serviceId, int32_t targetInstanceId)
{
    PB::Gateway::BindServiceReq req;
    req.set_session_id(sessionId);
    req.set_service_id(serviceId);
    req.set_target_instance_id(targetInstanceId);
    std::string payload;
    if (!req.SerializeToString(&payload))
        return -1;
    return sendFrame(CMD_GATEWAY_CONTROL, PB::Gateway::BindServiceReq::Id, 0, 0,
                     payload.data(), payload.size());
}

int GatewayClient::unbindService(uint32_t sessionId, uint8_t serviceId)
{
    PB::Gateway::UnbindServiceReq req;
    req.set_session_id(sessionId);
    req.set_service_id(serviceId);
    std::string payload;
    if (!req.SerializeToString(&payload))
        return -1;
    return sendFrame(CMD_GATEWAY_CONTROL, PB::Gateway::UnbindServiceReq::Id, 0, 0,
                     payload.data(), payload.size());
}

int GatewayClient::kickSession(uint32_t sessionId)
{
    PB::Gateway::KickSessionReq req;
    req.set_session_id(sessionId);
    std::string payload;
    if (!req.SerializeToString(&payload))
        return -1;
    return sendFrame(CMD_GATEWAY_CONTROL, PB::Gateway::KickSessionReq::Id, 0, 0,
                     payload.data(), payload.size());
}

int GatewayClient::forwardToServer(uint8_t targetServiceId, int32_t targetInstanceId,
                                   const char* data, size_t length)
{
    PB::Gateway::ForwardToServerReq req;
    req.set_target_service_id(targetServiceId);
    req.set_target_instance_id(targetInstanceId);
    if (length > 0)
        req.set_payload(data, length);
    std::string payload;
    if (!req.SerializeToString(&payload))
        return -1;
    return sendFrame(CMD_GATEWAY_CONTROL, PB::Gateway::ForwardToServerReq::Id, 0, 0,
                     payload.data(), payload.size());
}

int GatewayClient::forwardMessageToServer(uint8_t targetServiceId, int32_t targetInstanceId,
                                          uint8_t cmd, uint16_t msgId, int32_t serial,
                                          uint32_t sessionId, const char* data, size_t length)
{
    const uint32_t frameLen = static_cast<uint32_t>(BACKEND_FRAME_HEADER_SIZE + length);
    std::string frame(frameLen, '\0');
    writeUint32(frame, 0, frameLen);
    frame[4] = static_cast<char>(cmd);
    writeUint16(frame, 5, msgId);
    writeUint32(frame, 7, static_cast<uint32_t>(serial));
    writeUint32(frame, 11, sessionId);
    if (length > 0)
        std::memcpy(frame.data() + BACKEND_FRAME_HEADER_SIZE, data, length);

    return forwardToServer(targetServiceId, targetInstanceId, frame.data(), frame.size());
}

int GatewayClient::sendControl(uint16_t msgId, int32_t serial, uint32_t sessionId,
                               const char* data, size_t length)
{
    return sendFrame(CMD_GATEWAY_CONTROL, msgId, serial, sessionId, data, length);
}

GatewayTask GatewayClient::requestAsync(uint32_t sessionId, uint16_t msgId,
                                        const char* data, size_t length, float timeout)
{
    auto promise = std::make_shared<TaskPromise<GatewayResponse>>();
    auto result = promise->get_result();

    sendRequest(
        sessionId, msgId, data, length,
        [promise, sessionId, msgId](bool ok, const std::string_view& payload) mutable {
            GatewayResponse response;
            response.ok = ok;
            response.cmd = ok ? CMD_BUSINESS : CMD_GATEWAY_ERROR;
            response.msgId = msgId;
            response.sessionId = sessionId;
            if (ok)
                response.payload.assign(payload.data(), payload.size());
            else
                response.error.assign(payload.data(), payload.size());

            try
            {
                promise->set_result(std::move(response));
            }
            catch (...)
            {
            }
        },
        timeout);

    return result;
}

void GatewayClient::failAllPendingRequests(const std::string& error)
{
    if (m_pendingRequests.empty())
        return;

    std::vector<GatewayRequestCallback> callbacks;
    callbacks.reserve(m_pendingRequests.size());
    for (auto& it : m_pendingRequests)
        callbacks.emplace_back(std::move(it.second.callback));
    m_pendingRequests.clear();

    for (auto& callback : callbacks)
        callback(false, error);
}

void GatewayClient::timeoutCheck(float dt)
{
    if (m_pendingRequests.empty())
        return;

    std::vector<GatewayRequestCallback> callbacks;
    for (auto it = m_pendingRequests.begin(); it != m_pendingRequests.end();)
    {
        it->second.timeout -= dt;
        if (it->second.timeout <= 0.0f)
        {
            callbacks.emplace_back(std::move(it->second.callback));
            it = m_pendingRequests.erase(it);
        }
        else
        {
            ++it;
        }
    }

    for (auto& callback : callbacks)
        callback(false, "timeout");
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
