#pragma once

#include "framework/BackendCodec.h"
#include "framework/RpcManager.h"

#include "game.pb.h"
#include "gateway.pb.h"
#include "yasio/yasio.hpp"

#include <chrono>
#include <cstdint>
#include <memory>
#include <string>
#include <unordered_set>

namespace battle
{

struct ServerSource
{
    std::uint32_t serviceId = 0;
    std::int32_t instanceId = 0;

    static ServerSource anyInstance(std::uint32_t serviceId)
    {
        return ServerSource{serviceId, -1};
    }
};

struct BackendConfig
{
    std::uint32_t serviceId = kServiceIdBattle;
    std::uint32_t instanceId = 1;
    std::string gatewayHost = "127.0.0.1";
    int gatewayPort = 7100;
    float reconnectInterval = 3.0f;
    std::uint32_t initialLoadScore = 0;
    bool initialAcceptingBindings = true;
    std::string initialLoadMessage;
};

struct SerializedMessage
{
    std::uint16_t msgId = 0;
    std::string payload;
};

using SerializedMessagePtr = std::unique_ptr<SerializedMessage>;

template <typename PbResponse>
using RpcCallback = std::function<void(const PbResponse* response, const std::string& error)>;

template <typename PbMessage>
SerializedMessagePtr makeSerializedMessage(const PbMessage& message)
{
    auto result = std::make_unique<SerializedMessage>();
    result->msgId = static_cast<std::uint16_t>(PbMessage::Id);
    if (!message.SerializeToString(&result->payload))
        return nullptr;
    return result;
}

class BackendClient;

class BackendDelegate
{
public:
    virtual ~BackendDelegate() = default;
    virtual void onConnected(BackendClient& client) { (void)client; }
    virtual void onDisconnected(BackendClient& client) { (void)client; }
    virtual void onSessionOnline(BackendClient& client, std::uint32_t sessionId)
    {
        (void)client;
        (void)sessionId;
    }
    virtual void onSessionOffline(BackendClient& client, std::uint32_t sessionId)
    {
        (void)client;
        (void)sessionId;
    }
    virtual SerializedMessagePtr onClientRequest(BackendClient& client,
                                                 std::uint32_t sessionId,
                                                 const BackendFrame& frame)
    {
        (void)client;
        (void)sessionId;
        (void)frame;
        return nullptr;
    }
    virtual void onClientPush(BackendClient& client,
                              std::uint32_t sessionId,
                              const BackendFrame& frame)
    {
        (void)client;
        (void)sessionId;
        (void)frame;
    }
    virtual SerializedMessagePtr onServerRequest(BackendClient& client,
                                                 ServerSource source,
                                                 const BackendFrame& frame)
    {
        (void)client;
        (void)source;
        (void)frame;
        return nullptr;
    }
    virtual void onServerPush(BackendClient& client, ServerSource source, const BackendFrame& frame)
    {
        (void)client;
        (void)source;
        (void)frame;
    }
    virtual void onShutdown(BackendClient& client) { (void)client; }
};

class BackendClient
{
public:
    BackendClient();
    ~BackendClient();

    bool init(const BackendConfig& config, BackendDelegate* delegate);
    void start();
    void stop();
    bool isConnected() const;
    yasio::highp_timer_ptr schedule(const std::chrono::microseconds& duration,
                                    yasio::timer_cb_t callback);

    bool sendFrame(std::uint8_t cmd,
                   std::uint16_t msgId,
                   std::int32_t serial,
                   std::uint32_t sessionId,
                   const char* payload,
                   std::size_t payloadLen);

    template <typename PbMessage>
    bool sendToClient(std::uint32_t sessionId, std::int32_t serial, const PbMessage& message)
    {
        return sendMessage(kCmdBusiness, sessionId, serial, message);
    }

    template <typename PbMessage>
    bool sendPush(std::uint32_t sessionId, const PbMessage& message)
    {
        return sendToClient(sessionId, 0, message);
    }

    template <typename PbRequest, typename PbResponse>
    std::int32_t requestGateway(const PbRequest& message,
                                RpcCallback<PbResponse> callback,
                                float timeoutSeconds = kDefaultRpcTimeoutSeconds)
    {
        return requestFrame<PbRequest, PbResponse>(
            kCmdGatewayControl, 0, message, std::move(callback), timeoutSeconds);
    }

    template <typename PbRequest, typename PbResponse>
    std::int32_t requestServer(ServerSource target,
                               const PbRequest& message,
                               RpcCallback<PbResponse> callback,
                               float timeoutSeconds = kDefaultRpcTimeoutSeconds)
    {
        std::string payload;
        if (!message.SerializeToString(&payload))
        {
            if (callback)
                callback(nullptr, "serialize request failed");
            return -1;
        }
        return requestServerPayload(target,
                                    static_cast<std::uint16_t>(PbRequest::Id),
                                    payload.data(),
                                    payload.size(),
                                    makeRpcCallback<PbResponse>(std::move(callback)),
                                    timeoutSeconds);
    }

    template <typename PbMessage>
    bool sendServerPush(ServerSource target, const PbMessage& message)
    {
        std::string payload;
        if (!message.SerializeToString(&payload))
            return false;
        return sendServerPayload(target,
                                 static_cast<std::uint16_t>(PbMessage::Id),
                                 0,
                                 0,
                                 payload.data(),
                                 payload.size());
    }

    bool bindService(std::uint32_t sessionId, std::uint32_t serviceId, std::int32_t targetInstanceId);
    bool unbindService(std::uint32_t sessionId, std::uint32_t serviceId);
    bool kickSession(std::uint32_t sessionId);
    bool reportLoad(std::uint32_t loadScore, bool acceptingBindings, const std::string& message);

private:
    enum class State
    {
        Disconnected,
        Connecting,
        Connected,
        Registered,
    };

    template <typename PbMessage>
    bool sendMessage(std::uint8_t cmd,
                     std::uint32_t sessionId,
                     std::int32_t serial,
                     const PbMessage& message)
    {
        std::string payload;
        if (!message.SerializeToString(&payload))
            return false;
        return sendFrame(cmd,
                         static_cast<std::uint16_t>(PbMessage::Id),
                         serial,
                         sessionId,
                         payload.data(),
                         payload.size());
    }

    template <typename PbResponse>
    RawRequestCallback makeRpcCallback(RpcCallback<PbResponse> callback)
    {
        return [callback = std::move(callback)](const BackendFrame* frame, std::string error) {
            if (!callback)
                return;

            if (!frame)
            {
                callback(nullptr, error);
                return;
            }

            if (frame->msgId != PbResponse::Id)
            {
                std::string message = "unexpected rpc response msg_id=" + std::to_string(frame->msgId);
                if (frame->msgId == PB::Gateway::GatewayErrorResp::Id)
                {
                    PB::Gateway::GatewayErrorResp gatewayError;
                    if (parsePayload(gatewayError, frame->payload))
                    {
                        message = "gateway error " + std::to_string(gatewayError.code()) +
                            ": " + gatewayError.message();
                    }
                }
                else if (frame->msgId == PB::Game::CommonErrorResp::Id)
                {
                    PB::Game::CommonErrorResp commonError;
                    if (parsePayload(commonError, frame->payload))
                        message = commonError.message();
                }
                callback(nullptr, message);
                return;
            }

            PbResponse response;
            if (!parsePayload(response, frame->payload))
            {
                callback(nullptr, "decode rpc response failed");
                return;
            }

            callback(&response, "");
        };
    }

    template <typename PbRequest, typename PbResponse>
    std::int32_t requestFrame(std::uint8_t cmd,
                              std::uint32_t sessionId,
                              const PbRequest& message,
                              RpcCallback<PbResponse> callback,
                              float timeoutSeconds)
    {
        std::string payload;
        if (!message.SerializeToString(&payload))
        {
            if (callback)
                callback(nullptr, "serialize request failed");
            return -1;
        }

        const auto requestId = m_rpc.nextRequestId();
        m_rpc.addPending(requestId, makeRpcCallback<PbResponse>(std::move(callback)), timeoutSeconds);
        if (!sendFrame(cmd,
                       static_cast<std::uint16_t>(PbRequest::Id),
                       -requestId,
                       sessionId,
                       payload.data(),
                       payload.size()))
        {
            m_rpc.remove(requestId);
            return -1;
        }
        return requestId;
    }

    std::int32_t requestServerPayload(ServerSource target,
                                      std::uint16_t msgId,
                                      const char* payload,
                                      std::size_t payloadLen,
                                      RawRequestCallback callback,
                                      float timeoutSeconds);
    bool sendServerPayload(ServerSource target,
                           std::uint16_t msgId,
                           std::int32_t serial,
                           std::uint32_t sessionId,
                           const char* payload,
                           std::size_t payloadLen);

    void openConnection();
    void handleNetworkEvent(yasio::io_event* event);
    void onRecvFrame(std::string_view body);
    void processFrame(const BackendFrame& frame);
    void sendRegisterReq();
    void closeTransport();
    void scheduleReconnect();
    void onGatewayConnected();
    void onGatewayDisconnected();
    void onGatewayFrame(const BackendFrame& frame);
    void handleControlFrame(const BackendFrame& frame);
    void handleBusinessFrame(const BackendFrame& frame);
    void routeForwardToServer(const PB::Gateway::ForwardToServerReq& req);
    void spawnSession(std::uint32_t sessionId);
    void stopSession(std::uint32_t sessionId);
    void stopAllSessions();
    void processClientFrame(const BackendFrame& frame);
    void processServerFrame(ServerSource source, const BackendFrame& frame);
    void sendMessageToClient(std::uint32_t sessionId,
                             std::int32_t responseSerial,
                             SerializedMessagePtr message,
                             std::string error);
    void sendMessageToServer(ServerSource target,
                             std::int32_t responseSerial,
                             SerializedMessagePtr message,
                             std::string error);
    SerializedMessagePtr commonError(std::string message) const;
    void startRpcTimer();

private:
    BackendConfig m_config;
    yasio::io_service m_service;
    yasio::transport_handle_t m_transport = nullptr;
    State m_state = State::Disconnected;
    RpcManager m_rpc;
    BackendDelegate* m_delegate = nullptr;
    bool m_running = false;
    bool m_reconnectScheduled = false;
    std::chrono::steady_clock::time_point m_lastRpcUpdate;
    std::unordered_set<std::uint32_t> m_onlineSessions;
};

}
