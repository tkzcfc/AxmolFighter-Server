#pragma once

#include "battle/BattleRoomManager.h"
#include "battle/BattleSync.h"
#include "framework/BackendClient.h"
#include "client_battle.pb.h"
#include "client_game.pb.h"
#include "game_types.pb.h"

#include <cstdint>
#include <memory>
#include <string>

struct BattleServerConfig
{
    std::uint32_t instanceId = 1;
    std::string gatewayHost = "127.0.0.1";
    int gatewayPort = 7100;
    float reconnectInterval = 3.0f;
    int tickRate = 30;
    // 全量 snapshot 间隔（逻辑帧数）；默认 10 ≈ 3Hz（tick_rate=30）
    int snapshotIntervalFrames = 10;
    std::uint32_t maxBattles = 100;
    std::uint32_t maxSessions = 200;
    float loadReportInterval = 5.0f;
    /** Content 根目录：含 mugen/config/config.bin 与 avatar.bin */
    std::string contentRoot = "../../AxmolFighter-Client/Content";
};

// Battle 服入口：只做协议解析、委托 RoomManager/Sync，以及 Backend 生命周期回调。
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
    void onServerOnline(battle::BackendClient& client,
                        std::uint32_t serviceId,
                        std::uint32_t instanceId) override;
    void onServerOffline(battle::BackendClient& client,
                         std::uint32_t serviceId,
                         std::uint32_t instanceId) override;
    void onShutdown(battle::BackendClient& client) override;

private:
    battle::SerializedMessagePtr onBattleCreate(const battle::BackendFrame& frame);
    void onBattleInput(std::uint32_t sessionId, const battle::BackendFrame& frame);

    battle::SerializedMessagePtr makeBattleCreateResp(std::int32_t code,
                                                      const std::string& message,
                                                      const battle::BattleRoom* room,
                                                      std::uint32_t requesterActorId = 0) const;
    void sendLoadReport();
    void tick(float dt);

private:
    BattleServerConfig m_config;
    battle::BackendClient m_backend;
    std::unique_ptr<battle::BattleRoomManager> m_roomManager;
    std::unique_ptr<battle::BattleSync> m_sync;
    bool m_running = false;
    std::uint64_t m_randomSeed = 0xBA771E;
    float m_loadReportTimer = 0.0f;
};
