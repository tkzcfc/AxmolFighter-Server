#pragma once

#include "framework/BackendClient.h"
#include "game.pb.h"
#include "mugen/core/ecs/Types.h"

#include <cstdint>
#include <memory>
#include <string>
#include <unordered_map>
#include <unordered_set>

namespace mugen
{
class GameWord;
}

struct BattleServerConfig
{
    std::uint32_t instanceId = 1;
    std::string gatewayHost = "127.0.0.1";
    int gatewayPort = 7100;
    float reconnectInterval = 3.0f;
    int tickRate = 30;
    std::uint32_t maxBattles = 100;
    std::uint32_t maxSessions = 200;
    float loadReportInterval = 5.0f;
    bool restartOnGatewayDisconnect = true;
};

struct BattleInstance
{
    std::uint32_t battleId = 0;
    std::int32_t mapId = 1;
    std::uint32_t serverFrame = 0;
    float elapsed = 0.0f;
    std::unordered_set<std::uint32_t> players;
    std::unique_ptr<mugen::GameWord> world;
};

class BattleServer final : public battle::BackendDelegate
{
public:
    BattleServer();
    ~BattleServer() override;

    bool init(const BattleServerConfig& config);
    void run();
    void shutdown();

    void onConnected(battle::BackendClient& client) override;
    void onDisconnected(battle::BackendClient& client) override;
    void onSessionOnline(battle::BackendClient& client, std::uint32_t sessionId) override;
    void onSessionOffline(battle::BackendClient& client, std::uint32_t sessionId) override;
    battle::SerializedMessagePtr onClientRequest(battle::BackendClient& client,
                                                 std::uint32_t sessionId,
                                                 const battle::BackendFrame& frame) override;
    void onClientPush(battle::BackendClient& client,
                      std::uint32_t sessionId,
                      const battle::BackendFrame& frame) override;
    battle::SerializedMessagePtr onServerRequest(battle::BackendClient& client,
                                                 battle::ServerSource source,
                                                 const battle::BackendFrame& frame) override;
    void onServerPush(battle::BackendClient& client,
                      battle::ServerSource source,
                      const battle::BackendFrame& frame) override;
    void onShutdown(battle::BackendClient& client) override;

private:
    battle::SerializedMessagePtr onBattleCreate(const battle::BackendFrame& frame);
    void onBattleInput(std::uint32_t sessionId, const battle::BackendFrame& frame);

    BattleInstance* createBattle(std::uint32_t battleId, std::int32_t mapId);
    bool addPlayerToBattle(BattleInstance& battle, std::uint32_t sessionId);
    void removePlayer(std::uint32_t sessionId);

    std::string serializeWorld(const BattleInstance& battle) const;
    battle::SerializedMessagePtr makeBattleCreateResp(std::int32_t code,
                                                      const std::string& message,
                                                      const BattleInstance* battle) const;
    void sendSnapshot(const BattleInstance& battle);
    void sendLoadReport();
    std::uint32_t activeSessionCount() const;
    bool canAcceptBinding(std::uint32_t sessionId) const;
    void tick(float dt);

private:
    BattleServerConfig m_config;
    battle::BackendClient m_backend;
    bool m_running = false;
    std::uint64_t m_randomSeed = 0xBA771E;
    float m_loadReportTimer = 0.0f;

    std::unordered_map<std::uint32_t, std::unique_ptr<BattleInstance>> m_battles;
    std::unordered_map<std::uint32_t, std::uint32_t> m_sessionToBattle;
    std::unordered_map<std::uint32_t, mugen::EntityId> m_sessionToActor;
};
