#pragma once

#include "framework/BackendCodec.h"
#include <cstdint>
#include <functional>
#include <string>
#include <unordered_map>
#include <vector>

namespace battle
{

using RawRequestCallback = std::function<void(const BackendFrame* frame, std::string error)>;

class RpcManager
{
public:
    std::int32_t nextRequestId();
    void addPending(std::int32_t requestId, RawRequestCallback callback, float timeoutSeconds);
    bool resolve(const BackendFrame& frame);
    void remove(std::int32_t requestId);
    void update(float dt);
    void failAll(const std::string& error);

private:
    struct PendingRequest
    {
        RawRequestCallback callback;
        float timeout = 0.0f;
    };

    std::int32_t m_nextRequestId = 1;
    std::unordered_map<std::int32_t, PendingRequest> m_pending;
};

}
