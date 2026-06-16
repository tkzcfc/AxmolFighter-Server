#include "BattleServer.h"
#include <chrono>
#include <thread>
#include <cstdio>

using namespace std::string_view_literals;

BattleServer::BattleServer()
    : m_running(false)
    , m_battleIdSeed(0)
{
}

BattleServer::~BattleServer()
{
    shutdown();
}

bool BattleServer::init(const BattleServerConfig& config)
{
    m_config = config;

    GatewayClient::Config gwConfig;
    gwConfig.host              = config.gatewayHost;
    gwConfig.port              = config.gatewayPort;
    gwConfig.serviceType       = SERVICE_TYPE_BATTLE;
    gwConfig.instanceId        = config.instanceId;
    gwConfig.reconnectInterval = config.reconnectInterval;

    m_gateway.init(gwConfig);
    m_gateway.setMsgCallback([this](uint16_t msgId, int32_t serial, uint32_t sessionId,
                                    const std::string_view& payload) {
        this->onGatewayMsg(msgId, serial, sessionId, payload);
    });
    m_gateway.setDisconnectCallback([this]() {
        printf("[BattleServer] gateway disconnected, clearing all battles\n");
        m_battles.clear();
        m_sessionToBattle.clear();
    });

    return true;
}

void BattleServer::run()
{
    m_running = true;
    m_gateway.start();

    const auto tickInterval = std::chrono::microseconds(1000000 / m_config.tickRate);
    auto lastTime           = std::chrono::steady_clock::now();

    printf("[BattleServer] running at %d tick/s, instance_id=%u\n", m_config.tickRate, m_config.instanceId);

    while (m_running)
    {
        auto now = std::chrono::steady_clock::now();
        float dt = std::chrono::duration<float>(now - lastTime).count();
        lastTime = now;

        // 驱动网络
        m_gateway.update(dt);

        // 游戏逻辑 tick
        tick(dt);

        // 帧率控制
        auto elapsed = std::chrono::steady_clock::now() - now;
        if (elapsed < tickInterval)
        {
            std::this_thread::sleep_for(tickInterval - elapsed);
        }
    }

    m_gateway.stop();
    printf("[BattleServer] stopped\n");
}

void BattleServer::shutdown()
{
    m_running = false;
}

void BattleServer::onGatewayMsg(uint16_t msgId, int32_t serial, uint32_t sessionId,
                                const std::string_view& payload)
{
    switch (msgId)
    {
    case MSG_SESSION_ONLINE:
        if (payload.size() >= 4)
        {
            uint32_t sid = static_cast<uint32_t>(static_cast<uint8_t>(payload[0])) |
                           (static_cast<uint32_t>(static_cast<uint8_t>(payload[1])) << 8) |
                           (static_cast<uint32_t>(static_cast<uint8_t>(payload[2])) << 16) |
                           (static_cast<uint32_t>(static_cast<uint8_t>(payload[3])) << 24);
            onSessionOnline(sid);
        }
        break;

    case MSG_SESSION_OFFLINE:
        if (payload.size() >= 4)
        {
            uint32_t sid = static_cast<uint32_t>(static_cast<uint8_t>(payload[0])) |
                           (static_cast<uint32_t>(static_cast<uint8_t>(payload[1])) << 8) |
                           (static_cast<uint32_t>(static_cast<uint8_t>(payload[2])) << 16) |
                           (static_cast<uint32_t>(static_cast<uint8_t>(payload[3])) << 24);
            onSessionOffline(sid);
        }
        break;

    default:
        // 玩家消息（msg_id 20000-29999）
        if (msgId >= 20000 && msgId <= 29999)
        {
            onPlayerState(sessionId, serial, payload);
        }
        break;
    }
}

void BattleServer::onSessionOnline(uint32_t sessionId)
{
    printf("[BattleServer] session online: %u\n", sessionId);
}

void BattleServer::onSessionOffline(uint32_t sessionId)
{
    printf("[BattleServer] session offline: %u\n", sessionId);
    // 从战斗实例中移除
    auto it = m_sessionToBattle.find(sessionId);
    if (it != m_sessionToBattle.end())
    {
        auto battleIt = m_battles.find(it->second);
        if (battleIt != m_battles.end())
        {
            battleIt->second->players.erase(sessionId);
            // 如果战斗中没有玩家了，销毁战斗
            if (battleIt->second->players.empty())
            {
                printf("[BattleServer] battle %u destroyed (no players)\n", battleIt->first);
                m_battles.erase(battleIt);
            }
        }
        m_sessionToBattle.erase(it);
    }
}

void BattleServer::onPlayerState(uint32_t sessionId, int32_t serial, const std::string_view& payload)
{
    // TODO: 处理玩家战斗输入（PlayerState msg_id=20000）
    // 将输入分发到对应的战斗实例
    auto it = m_sessionToBattle.find(sessionId);
    if (it == m_sessionToBattle.end())
    {
        // 玩家不在任何战斗中，忽略
        return;
    }

    auto battleIt = m_battles.find(it->second);
    if (battleIt == m_battles.end())
        return;

    // TODO: 将 payload 解析为游戏输入，更新战斗状态
}

void BattleServer::tick(float dt)
{
    // 更新所有战斗实例
    for (auto& [id, battle] : m_battles)
    {
        battle->elapsed += dt;
        // TODO: 运行战斗逻辑（mugen::GameWord::update），广播状态给玩家
    }
}
