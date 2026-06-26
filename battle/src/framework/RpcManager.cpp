#include "framework/RpcManager.h"

namespace battle
{

std::int32_t RpcManager::nextRequestId()
{
    const auto requestId = m_nextRequestId;
    ++m_nextRequestId;
    if (m_nextRequestId <= 0)
        m_nextRequestId = 1;
    return requestId;
}

void RpcManager::addPending(std::int32_t requestId,
                            RawRequestCallback callback,
                            float timeoutSeconds)
{
    if (!callback)
        return;

    PendingRequest req;
    req.callback = std::move(callback);
    req.timeout = timeoutSeconds;
    m_pending[requestId] = std::move(req);
}

bool RpcManager::resolve(const BackendFrame& frame)
{
    if (frame.serial <= 0)
        return false;

    auto it = m_pending.find(frame.serial);
    if (it == m_pending.end())
        return false;

    auto callback = std::move(it->second.callback);
    m_pending.erase(it);

    callback(&frame, "");
    return true;
}

void RpcManager::remove(std::int32_t requestId)
{
    m_pending.erase(requestId);
}

void RpcManager::update(float dt)
{
    if (m_pending.empty())
        return;

    std::vector<RawRequestCallback> callbacks;
    for (auto it = m_pending.begin(); it != m_pending.end();)
    {
        it->second.timeout -= dt;
        if (it->second.timeout <= 0.0f)
        {
            callbacks.emplace_back(std::move(it->second.callback));
            it = m_pending.erase(it);
        }
        else
        {
            ++it;
        }
    }

    for (auto& callback : callbacks)
    {
        callback(nullptr, "request timeout");
    }
}

void RpcManager::failAll(const std::string& error)
{
    std::vector<RawRequestCallback> callbacks;
    callbacks.reserve(m_pending.size());
    for (auto& item : m_pending)
        callbacks.emplace_back(std::move(item.second.callback));
    m_pending.clear();

    for (auto& callback : callbacks)
    {
        callback(nullptr, error);
    }
}

}
