#pragma once

#include "net/GatewayClient.h"
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <memory>

/// 战斗服配置
struct BattleServerConfig
{
    uint32_t instanceId         = 1;
    std::string gatewayHost     = "127.0.0.1";
    int gatewayPort             = 7100;
    float reconnectInterval     = 3.0f;
    int tickRate                = 30;  // 帧率（每秒 tick 次数）
};

/// 战斗实例（一场战斗）
struct BattleInstance
{
    uint32_t battleId;
    std::unordered_set<uint32_t> players;  // session_id 集合
    float elapsed;
};

/// 战斗服主类
class BattleServer
{
public:
    BattleServer();
    ~BattleServer();

    bool init(const BattleServerConfig& config);

    /// 主循环（阻塞）
    void run();

    /// 停止
    void shutdown();

private:
    void onGatewayMsg(uint8_t cmd, uint16_t msgId, int32_t serial, uint32_t sessionId, const std::string_view& payload);
    void onSessionOnline(uint32_t sessionId);
    void onSessionOffline(uint32_t sessionId);
    void onPlayerState(uint32_t sessionId, int32_t serial, const std::string_view& payload);

    void tick(float dt);

private:
    BattleServerConfig m_config;
    GatewayClient m_gateway;
    bool m_running;

    // 战斗实例管理
    uint32_t m_battleIdSeed;
    std::unordered_map<uint32_t, std::unique_ptr<BattleInstance>> m_battles;
    // session_id → battle_id 映射
    std::unordered_map<uint32_t, uint32_t> m_sessionToBattle;
};
