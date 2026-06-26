#include "framework/BackendClient.h"

#include "framework/Logger.h"
#include <spdlog/spdlog.h>
#include <utility>

namespace battle
{

BackendClient::BackendClient()
    : m_service(1)
{
    m_service.set_option(yasio::YOPT_S_CONNECT_TIMEOUT, 5);
    m_service.set_option(yasio::YOPT_S_DNS_QUERIES_TIMEOUT, 3);
    m_service.set_option(yasio::YOPT_S_DNS_QUERIES_TRIES, 1);
    m_service.set_option(yasio::YOPT_S_NO_NEW_THREAD, 1);
}

BackendClient::~BackendClient()
{
    stop();
}

bool BackendClient::init(const BackendConfig& config, BackendDelegate* delegate)
{
    m_config = config;
    m_delegate = delegate;
    return true;
}

void BackendClient::start()
{
    if (m_running)
        return;

    m_running = true;
    startRpcTimer();
    openConnection();
    m_service.start([this](yasio::event_ptr&& event) {
        handleNetworkEvent(event.get());
    });
}

void BackendClient::stop()
{
    if (!m_running)
        return;

    m_running = false;
    stopAllSessions();
    if (m_delegate)
        m_delegate->onShutdown(*this);
    m_rpc.failAll("backend session stopped");
    closeTransport();
    m_service.stop();
    m_state = State::Disconnected;
}

bool BackendClient::isConnected() const
{
    return m_state == State::Registered;
}

yasio::highp_timer_ptr BackendClient::schedule(const std::chrono::microseconds& duration,
                                               yasio::timer_cb_t callback)
{
    return m_service.schedule(duration, std::move(callback));
}

bool BackendClient::sendFrame(std::uint8_t cmd,
                              std::uint16_t msgId,
                              std::int32_t serial,
                              std::uint32_t sessionId,
                              const char* payload,
                              std::size_t payloadLen)
{
    if (!m_transport)
        return false;

    auto frame = encodeBackendFrame(cmd, msgId, serial, sessionId, payload, payloadLen);
    return m_service.write(m_transport, yasio::sbyte_buffer(frame.data(), frame.data() + frame.size())) >= 0;
}

bool BackendClient::bindService(std::uint32_t sessionId,
                                std::uint32_t serviceId,
                                std::int32_t targetInstanceId)
{
    PB::Gateway::BindServiceReq req;
    req.set_session_id(sessionId);
    req.set_service_id(serviceId);
    req.set_target_instance_id(targetInstanceId);
    return sendMessage(kCmdGatewayControl, 0, 0, req);
}

bool BackendClient::unbindService(std::uint32_t sessionId, std::uint32_t serviceId)
{
    PB::Gateway::UnbindServiceReq req;
    req.set_session_id(sessionId);
    req.set_service_id(serviceId);
    return sendMessage(kCmdGatewayControl, 0, 0, req);
}

bool BackendClient::kickSession(std::uint32_t sessionId)
{
    PB::Gateway::KickSessionReq req;
    req.set_session_id(sessionId);
    return sendMessage(kCmdGatewayControl, 0, 0, req);
}

bool BackendClient::reportLoad(std::uint32_t loadScore,
                               bool acceptingBindings,
                               const std::string& message)
{
    PB::Gateway::ServiceLoadReportPush push;
    push.set_load_score(loadScore > 100 ? 100 : loadScore);
    push.set_accepting_bindings(acceptingBindings);
    push.set_message(message);
    return sendMessage(kCmdGatewayControl, 0, 0, push);
}

std::int32_t BackendClient::requestServerPayload(ServerSource target,
                                                 std::uint16_t msgId,
                                                 const char* payload,
                                                 std::size_t payloadLen,
                                                 RawRequestCallback callback,
                                                 float timeoutSeconds)
{
    const auto requestId = m_rpc.nextRequestId();
    m_rpc.addPending(requestId, std::move(callback), timeoutSeconds);
    if (!sendServerPayload(target, msgId, -requestId, 0, payload, payloadLen))
    {
        m_rpc.remove(requestId);
        return -1;
    }
    return requestId;
}

bool BackendClient::sendServerPayload(ServerSource target,
                                      std::uint16_t msgId,
                                      std::int32_t serial,
                                      std::uint32_t sessionId,
                                      const char* payload,
                                      std::size_t payloadLen)
{
    auto inner = encodeBackendFrame(kCmdBusiness, msgId, serial, sessionId, payload, payloadLen);

    PB::Gateway::ForwardToServerReq req;
    req.set_target_service_id(target.serviceId);
    req.set_target_instance_id(target.instanceId);
    req.set_payload(inner.data(), inner.size());
    req.set_source_service_id(m_config.serviceId);
    req.set_source_instance_id(m_config.instanceId);

    std::string forwardPayload;
    if (!req.SerializeToString(&forwardPayload))
        return false;

    return sendFrame(kCmdGatewayControl,
                     static_cast<std::uint16_t>(PB::Gateway::ForwardToServerReq::Id),
                     serial,
                     0,
                     forwardPayload.data(),
                     forwardPayload.size());
}

void BackendClient::openConnection()
{
    if (!m_running || m_state == State::Connecting || m_state == State::Connected ||
        m_state == State::Registered)
    {
        return;
    }

    m_state = State::Connecting;
    m_transport = nullptr;

    m_service.set_option(yasio::YOPT_C_UNPACK_PARAMS, 0, kMaxBackendPacketSize, 0, 4, 0);
    m_service.set_option(yasio::YOPT_C_UNPACK_STRIP, 0, 4);
    m_service.set_option(yasio::YOPT_C_UNPACK_NO_BSWAP, 0, 0);
    m_service.set_option(yasio::YOPT_C_REMOTE_ENDPOINT, 0, m_config.gatewayHost.data(), m_config.gatewayPort);

    spdlog::info("BackendClient connecting to {}:{} ...", m_config.gatewayHost, m_config.gatewayPort);
    m_service.open(0, yasio::YCK_TCP_CLIENT);
}

void BackendClient::handleNetworkEvent(yasio::io_event* event)
{
    switch (event->kind())
    {
    case yasio::YEK_ON_OPEN:
        if (event->status() == 0)
        {
            m_transport = event->transport();
            m_state = State::Connected;
            spdlog::info("BackendClient connected, sending registration");
            sendRegisterReq();
        }
        else
        {
            spdlog::warn("BackendClient connect failed, status={}", event->status());
            m_transport = nullptr;
            m_state = State::Disconnected;
            handleConnectionFailure();
        }
        break;

    case yasio::YEK_ON_CLOSE:
        spdlog::warn("BackendClient disconnected, status={}", event->status());
        m_transport = nullptr;
        m_state = State::Disconnected;
        onGatewayDisconnected();
        handleConnectionFailure();
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

void BackendClient::onRecvFrame(std::string_view body)
{
    BackendFrame frame;
    std::string error;
    if (!decodeBackendFrameBody(body, frame, &error))
    {
        spdlog::warn("BackendClient dropped invalid frame: {}", error);
        return;
    }
    processFrame(frame);
}

void BackendClient::processFrame(const BackendFrame& frame)
{
    if (m_state != State::Registered)
    {
        if (frame.cmd != kCmdGatewayControl || frame.msgId != PB::Gateway::ServerRegResp::Id)
        {
            spdlog::warn("BackendClient ignored pre-registration frame cmd={} msg_id={}",
                         frame.cmd,
                         frame.msgId);
            return;
        }

        PB::Gateway::ServerRegResp resp;
        if (!parsePayload(resp, frame.payload))
        {
            spdlog::error("BackendClient failed to decode ServerRegResp");
            closeTransport();
            m_state = State::Disconnected;
            handleConnectionFailure();
            return;
        }

        if (resp.code() != 0)
        {
            spdlog::error("BackendClient registration failed, code={}", resp.code());
            closeTransport();
            m_state = State::Disconnected;
            handleConnectionFailure();
            return;
        }

        m_state = State::Registered;
        spdlog::info("BackendClient registered service_id={} instance_id={}",
                     m_config.serviceId,
                     m_config.instanceId);
        onGatewayConnected();
        return;
    }

    onGatewayFrame(frame);
}

void BackendClient::sendRegisterReq()
{
    PB::Gateway::ServerRegReq req;
    req.set_service_id(m_config.serviceId);
    req.set_instance_id(m_config.instanceId);
    req.set_load_score(m_config.initialLoadScore > 100 ? 100 : m_config.initialLoadScore);
    req.set_accepting_bindings(m_config.initialAcceptingBindings);
    req.set_load_message(m_config.initialLoadMessage);

    std::string payload;
    if (!req.SerializeToString(&payload))
    {
        spdlog::error("BackendClient failed to serialize ServerRegReq");
        return;
    }

    sendFrame(kCmdGatewayControl,
              static_cast<std::uint16_t>(PB::Gateway::ServerRegReq::Id),
              kRegisterSerial,
              0,
              payload.data(),
              payload.size());
}

void BackendClient::closeTransport()
{
    if (m_transport)
    {
        m_service.close(m_transport);
        m_transport = nullptr;
    }
}

void BackendClient::handleConnectionFailure()
{
    if (!m_running)
        return;

    spdlog::warn("BackendClient stopping after gateway disconnect/failure");
    m_running = false;
    m_rpc.failAll("gateway disconnected");
    m_service.stop();
}

void BackendClient::onGatewayConnected()
{
    spdlog::info("BackendClient connected to gateway");
    if (m_delegate)
        m_delegate->onConnected(*this);
}

void BackendClient::onGatewayDisconnected()
{
    spdlog::warn("BackendClient disconnected from gateway");
    stopAllSessions();
    m_rpc.failAll("gateway disconnected");
    if (m_delegate)
        m_delegate->onDisconnected(*this);
}

void BackendClient::onGatewayFrame(const BackendFrame& frame)
{
    switch (frame.cmd)
    {
    case kCmdGatewayControl:
        handleControlFrame(frame);
        break;
    case kCmdBusiness:
        handleBusinessFrame(frame);
        break;
    default:
        spdlog::debug("Dropped frame with unknown cmd={}", frame.cmd);
        break;
    }
}

void BackendClient::handleControlFrame(const BackendFrame& frame)
{
    if (frame.serial > 0 && m_rpc.resolve(frame))
        return;

    if (frame.msgId == PB::Gateway::SessionOnlinePush::Id)
    {
        PB::Gateway::SessionOnlinePush push;
        if (parsePayload(push, frame.payload))
            spawnSession(push.session_id());
        return;
    }

    if (frame.msgId == PB::Gateway::SessionOfflinePush::Id)
    {
        PB::Gateway::SessionOfflinePush push;
        if (parsePayload(push, frame.payload))
            stopSession(push.session_id());
        return;
    }

    if (frame.msgId == PB::Gateway::ForwardToServerReq::Id)
    {
        PB::Gateway::ForwardToServerReq req;
        if (parsePayload(req, frame.payload))
            routeForwardToServer(req);
        else
            spdlog::warn("Failed to decode ForwardToServerReq");
        return;
    }

    if (frame.msgId == PB::Gateway::ServerPingReq::Id)
    {
        PB::Gateway::ServerPingReq ping;
        if (!parsePayload(ping, frame.payload))
        {
            spdlog::warn("Failed to decode ServerPingReq");
            return;
        }

        PB::Gateway::ServerPongResp pong;
        pong.set_nonce(ping.nonce());
        if (!sendMessage(kCmdGatewayControl, frame.sessionId, 0, pong))
            spdlog::warn("Gateway heartbeat pong failed nonce={}", ping.nonce());
        return;
    }

    if (frame.msgId == PB::Gateway::ServerOnlinePush::Id)
    {
        PB::Gateway::ServerOnlinePush push;
        if (parsePayload(push, frame.payload))
            spdlog::info("Backend service online service_id={} instance_id={}",
                         push.service_id(),
                         push.instance_id());
        return;
    }

    if (frame.msgId == PB::Gateway::ServerOfflinePush::Id)
    {
        PB::Gateway::ServerOfflinePush push;
        if (parsePayload(push, frame.payload))
            spdlog::info("Backend service offline service_id={} instance_id={}",
                         push.service_id(),
                         push.instance_id());
        return;
    }

    spdlog::debug("Unhandled gateway control msg_id={} serial={}", frame.msgId, frame.serial);
}

void BackendClient::handleBusinessFrame(const BackendFrame& frame)
{
    if (frame.serial > 0 && m_rpc.resolve(frame))
        return;

    if (m_onlineSessions.find(frame.sessionId) == m_onlineSessions.end())
    {
        spdlog::warn("Dropped client frame for unknown session {}", frame.sessionId);
        if (frame.serial != 0)
        {
            auto response = commonError("unknown session");
            if (response)
            {
                sendFrame(kCmdBusiness,
                          response->msgId,
                          -frame.serial,
                          frame.sessionId,
                          response->payload.data(),
                          response->payload.size());
            }
        }
        return;
    }

    processClientFrame(frame);
}

void BackendClient::routeForwardToServer(const PB::Gateway::ForwardToServerReq& req)
{
    BackendFrame inner;
    std::string error;
    if (!decodeBackendFrame(req.payload(), inner, &error))
    {
        spdlog::warn("Dropped invalid forwarded frame: {}", error);
        return;
    }

    if (inner.serial > 0)
    {
        if (!m_rpc.resolve(inner))
            spdlog::debug("Ignored unmatched server response serial={}", inner.serial);
        return;
    }

    ServerSource source;
    source.serviceId = req.source_service_id();
    source.instanceId = static_cast<std::int32_t>(req.source_instance_id());
    processServerFrame(source, inner);
}

void BackendClient::spawnSession(std::uint32_t sessionId)
{
    stopSession(sessionId);

    m_onlineSessions.insert(sessionId);
    if (m_delegate)
        m_delegate->onSessionOnline(*this, sessionId);
    spdlog::info("Session {} online", sessionId);
}

void BackendClient::stopSession(std::uint32_t sessionId)
{
    auto it = m_onlineSessions.find(sessionId);
    if (it == m_onlineSessions.end())
        return;

    m_onlineSessions.erase(it);
    if (m_delegate)
        m_delegate->onSessionOffline(*this, sessionId);
    spdlog::info("Session {} offline", sessionId);
}

void BackendClient::stopAllSessions()
{
    auto sessions = std::move(m_onlineSessions);
    m_onlineSessions.clear();
    if (m_delegate)
    {
        for (std::uint32_t sessionId : sessions)
            m_delegate->onSessionOffline(*this, sessionId);
    }
}

void BackendClient::processClientFrame(const BackendFrame& frame)
{
    if (!m_delegate)
    {
        if (frame.serial != 0)
        {
            auto response = commonError("no backend delegate");
            if (response)
            {
                sendFrame(kCmdBusiness,
                          response->msgId,
                          -frame.serial,
                          frame.sessionId,
                          response->payload.data(),
                          response->payload.size());
            }
        }
        return;
    }

    if (frame.serial == 0)
    {
        m_delegate->onClientPush(*this, frame.sessionId, frame);
        return;
    }

    auto response = m_delegate->onClientRequest(*this, frame.sessionId, frame);
    if (!response)
        response = commonError("request failed");

    if (!response)
    {
        spdlog::error("Failed to create client response");
        return;
    }

    sendFrame(kCmdBusiness,
              response->msgId,
              -frame.serial,
              frame.sessionId,
              response->payload.data(),
              response->payload.size());
}

void BackendClient::processServerFrame(ServerSource source, const BackendFrame& frame)
{
    if (!m_delegate)
    {
        spdlog::error("No backend delegate to handle server frame from service_id={} instance_id={}",
                      source.serviceId,
                      source.instanceId);
        return;
    }

    if (frame.serial == 0)
    {
        m_delegate->onServerPush(*this, source, frame);
        return;
    }

    auto response = m_delegate->onServerRequest(*this, source, frame);
    if (!response)
        response = commonError("request failed");

    if (!response)
    {
        spdlog::error("Failed to create server response");
        return;
    }

    sendServerPayload(source,
                      response->msgId,
                      -frame.serial,
                      0,
                      response->payload.data(),
                      response->payload.size());
}

SerializedMessagePtr BackendClient::commonError(std::string message) const
{
    PB::Game::CommonErrorResp resp;
    resp.set_code(-1);
    resp.set_message("internal error: " + message);
    return makeSerializedMessage(resp);
}

void BackendClient::startRpcTimer()
{
    m_lastRpcUpdate = std::chrono::steady_clock::now();
    m_service.schedule(std::chrono::milliseconds(100), [this](yasio::io_service&) {
        if (!m_running)
            return true;

        const auto now = std::chrono::steady_clock::now();
        const float dt = std::chrono::duration<float>(now - m_lastRpcUpdate).count();
        m_lastRpcUpdate = now;
        m_rpc.update(dt);
        return false;
    });
}

}
