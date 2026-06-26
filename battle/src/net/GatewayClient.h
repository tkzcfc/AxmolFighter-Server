#pragma once

#include "async/Task.h"
#include "yasio/yasio.hpp"
#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <string_view>
#include <type_traits>
#include <unordered_map>
#include <vector>

constexpr size_t BACKEND_FRAME_HEADER_SIZE = 15;
constexpr size_t BACKEND_FRAME_BODY_HEADER_SIZE = 11;

constexpr uint8_t CMD_BUSINESS = 1;
constexpr uint8_t CMD_GATEWAY_CONTROL = 2;

constexpr uint8_t SERVICE_ID_GAME = 0;
constexpr uint8_t SERVICE_ID_BATTLE = 1;

typedef std::function<void(bool, const std::string_view&)> GatewayConnectCallback;
typedef std::function<void()> GatewayDisconnectCallback;
typedef std::function<void(bool, const std::string_view&)> GatewayRequestCallback;
typedef std::function<void(uint8_t, uint16_t, int32_t, uint32_t, const std::string_view&)>
    GatewayMsgCallback;

struct GatewayResponse
{
    bool ok = false;
    uint8_t cmd = CMD_BUSINESS;
    uint16_t msgId = 0;
    int32_t serial = 0;
    uint32_t sessionId = 0;
    std::string payload;
    std::string error;
};

using GatewayTask = Task<GatewayResponse>;

class GatewayClient
{
public:
    struct Config
    {
        std::string host;
        int port = 0;
        uint8_t serviceId = SERVICE_ID_BATTLE;
        uint32_t instanceId = 0;
        float reconnectInterval = 3.0f;
        uint32_t initialLoadScore = 0;
        bool initialAcceptingBindings = true;
        std::string initialLoadMessage;
    };

    GatewayClient();
    ~GatewayClient();

    void init(const Config& config);
    void start();
    void stop();
    void update(float dt);

    bool isConnected() const { return m_state == State::Registered; }

    int32_t sendRequest(uint32_t sessionId, uint16_t msgId, const char* data, size_t length,
                        const GatewayRequestCallback& callback, float timeout = 10.0f);
    template <typename PbMessage>
    int32_t sendRequest(uint32_t sessionId, const PbMessage& message,
                        const GatewayRequestCallback& callback, float timeout = 10.0f)
    {
        std::string payload;
        if (!serializePbMessage(message, payload))
            return -1;
        return sendRequest(sessionId, pbMsgId<PbMessage>(), payload.data(), payload.size(),
                           callback, timeout);
    }
    void cancelRequest(int32_t requestId);

    void sendPush(uint32_t sessionId, uint16_t msgId, const char* data, size_t length);
    template <typename PbMessage>
    void sendPush(uint32_t sessionId, const PbMessage& message)
    {
        std::string payload;
        if (!serializePbMessage(message, payload))
            return;
        sendPush(sessionId, pbMsgId<PbMessage>(), payload.data(), payload.size());
    }

    void sendResponse(uint32_t sessionId, uint16_t msgId, int32_t serial, const char* data,
                      size_t length);
    template <typename PbMessage>
    void sendResponse(uint32_t sessionId, int32_t serial, const PbMessage& message)
    {
        std::string payload;
        if (!serializePbMessage(message, payload))
            return;
        sendResponse(sessionId, pbMsgId<PbMessage>(), serial, payload.data(), payload.size());
    }

    int sendToClient(uint32_t sessionId, uint16_t msgId, int32_t serial,
                     const char* data, size_t length);
    template <typename PbMessage>
    int sendToClient(uint32_t sessionId, int32_t serial, const PbMessage& message)
    {
        std::string payload;
        if (!serializePbMessage(message, payload))
            return -1;
        return sendToClient(sessionId, pbMsgId<PbMessage>(), serial, payload.data(), payload.size());
    }

    int bindService(uint32_t sessionId, uint8_t serviceId, int32_t targetInstanceId);
    int unbindService(uint32_t sessionId, uint8_t serviceId);
    int kickSession(uint32_t sessionId);
    int forwardToServer(uint8_t targetServiceId, int32_t targetInstanceId,
                        const char* data, size_t length);
    template <typename PbMessage>
    int forwardToServer(uint8_t targetServiceId, int32_t targetInstanceId,
                        const PbMessage& message)
    {
        std::string payload;
        if (!serializePbMessage(message, payload))
            return -1;
        return forwardToServer(targetServiceId, targetInstanceId, payload.data(), payload.size());
    }

    int forwardMessageToServer(uint8_t targetServiceId, int32_t targetInstanceId, uint8_t cmd,
                               uint16_t msgId, int32_t serial, uint32_t sessionId,
                               const char* data, size_t length);
    template <typename PbMessage>
    int forwardMessageToServer(uint8_t targetServiceId, int32_t targetInstanceId, int32_t serial,
                               uint32_t sessionId, const PbMessage& message)
    {
        std::string payload;
        if (!serializePbMessage(message, payload))
            return -1;
        return forwardMessageToServer(targetServiceId, targetInstanceId, CMD_BUSINESS,
                                      pbMsgId<PbMessage>(), serial, sessionId,
                                      payload.data(), payload.size());
    }

    int sendControl(uint16_t msgId, int32_t serial, uint32_t sessionId,
                    const char* data, size_t length);
    template <typename PbMessage>
    int sendControl(int32_t serial, uint32_t sessionId, const PbMessage& message)
    {
        std::string payload;
        if (!serializePbMessage(message, payload))
            return -1;
        return sendControl(pbMsgId<PbMessage>(), serial, sessionId, payload.data(), payload.size());
    }

    GatewayTask requestAsync(uint32_t sessionId, uint16_t msgId, const char* data, size_t length,
                             float timeout = 10.0f);
    template <typename PbMessage>
    GatewayTask requestAsync(uint32_t sessionId, const PbMessage& message,
                             float timeout = 10.0f)
    {
        std::string payload;
        if (!serializePbMessage(message, payload))
        {
            TaskPromise<GatewayResponse> promise;
            auto result = promise.get_result();
            GatewayResponse response;
            response.ok = false;
            response.cmd = CMD_GATEWAY_ERROR;
            response.msgId = pbMsgId<PbMessage>();
            response.sessionId = sessionId;
            response.error = "serialize failed";
            promise.set_result(std::move(response));
            return result;
        }

        return requestAsync(sessionId, pbMsgId<PbMessage>(), payload.data(), payload.size(),
                            timeout);
    }

    void setMsgCallback(const GatewayMsgCallback& callback) { m_onMsgCallback = callback; }
    void setConnectCallback(const GatewayConnectCallback& callback) { m_onConnect = callback; }
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
    void failAllPendingRequests(const std::string& error);
    void timeoutCheck(float dt);
    int sendFrame(uint8_t cmd, uint16_t msgId, int32_t serial, uint32_t sessionId,
                  const char* data, size_t length);

    template <typename PbMessage>
    static uint16_t pbMsgId()
    {
        return static_cast<uint16_t>(std::decay_t<PbMessage>::Id);
    }

    template <typename PbMessage>
    static bool serializePbMessage(const PbMessage& message, std::string& payload)
    {
        payload.clear();
        return message.SerializeToString(&payload);
    }

private:
    Config m_config;
    yasio::io_service m_service;
    yasio::transport_handle_t m_transport;
    State m_state;
    bool m_running;
    float m_reconnectTimer;
    int32_t m_serialCounter;

    struct PendingRequest
    {
        GatewayRequestCallback callback;
        float timeout = 0.0f;
    };
    std::unordered_map<int32_t, PendingRequest> m_pendingRequests;

    GatewayMsgCallback m_onMsgCallback;
    GatewayConnectCallback m_onConnect;
    GatewayDisconnectCallback m_onDisconnect;
};
