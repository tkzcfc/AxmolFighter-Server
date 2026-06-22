#pragma once

#include "mugen/core/ecs/Types.h"
#include "net/GatewayClient.h"
#include <memory>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>

namespace mugen
{
class GameWord;
}

struct BattleServerConfig
{
    uint32_t instanceId = 1;
    std::string gatewayHost = "127.0.0.1";
    int gatewayPort = 7100;
    float reconnectInterval = 3.0f;
    int tickRate = 30;
};

struct BattleInstance
{
    uint32_t battleId = 0;
    int32_t mapId = 1;
    uint32_t serverFrame = 0;
    float elapsed = 0.0f;
    std::unordered_set<uint32_t> players;
    std::unique_ptr<mugen::GameWord> world;
};

class BattleServer
{
public:
    BattleServer();
    ~BattleServer();

    bool init(const BattleServerConfig& config);
    void run();
    void shutdown();

private:
    void onGatewayMsg(uint8_t cmd, uint16_t msgId, int32_t serial, uint32_t sessionId,
                      const std::string_view& payload);
    void onSessionOnline(uint32_t sessionId);
    void onSessionOffline(uint32_t sessionId);
    void onBattleJoin(uint32_t sessionId, int32_t serial, const std::string_view& payload);
    void onBattleInput(uint32_t sessionId, const std::string_view& payload);

    BattleInstance* findJoinableBattle(int32_t mapId);
    BattleInstance* createBattle(int32_t mapId);
    bool addPlayerToBattle(BattleInstance& battle, uint32_t sessionId);
    void removePlayer(uint32_t sessionId);

    std::string serializeWorld(const BattleInstance& battle) const;
    void sendJoinResp(uint32_t sessionId, int32_t serial, int32_t code, const std::string& message,
                      const BattleInstance* battle);
    void sendSnapshot(const BattleInstance& battle);
    void tick(float dt);

private:
    BattleServerConfig m_config;
    GatewayClient m_gateway;
    bool m_running;
    uint32_t m_battleIdSeed;
    uint64_t m_randomSeed;

    std::unordered_map<uint32_t, std::unique_ptr<BattleInstance>> m_battles;
    std::unordered_map<uint32_t, uint32_t> m_sessionToBattle;
    std::unordered_map<uint32_t, mugen::EntityId> m_sessionToActor;
};
