#pragma once

#include "battle/BattleRoomManager.h"
#include "battle/IBattleSender.h"
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
    int gatewayPort = 17100;
    float reconnectInterval = 3.0f;
    int tickRate = 30;
    std::uint32_t maxBattles = 100;
    std::uint32_t maxSessions = 200;
    float loadReportInterval = 5.0f;
    /** 玩家断线后允许重连的宽限秒数，超时判负 Disconnect。 */
    float reconnectGraceSeconds = battle::kDefaultReconnectGraceSeconds;
    /** Content 根目录：含 mugen/config/config.bin 与 avatar.bin */
    std::string contentRoot = "../../AxmolFighter-Client/Content";
};

// Battle 服入口：协议解析、房间路由，以及 Backend 生命周期与 IBattleSender 实现。
class BattleServer final : public battle::BackendDelegate, public battle::IBattleSender
{
public:
    BattleServer();
    ~BattleServer() override;

    bool init(const BattleServerConfig& config);
    void run();
    void shutdown();

    // ---- BackendDelegate ----
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

    // ---- IBattleSender ----
    void sendStart(std::uint32_t sessionId, const PB::Battle::BattleStartPush& push) override;
    void sendFrames(std::uint32_t sessionId, const PB::Battle::BattleFramesPush& push) override;
    void sendSnapshot(std::uint32_t sessionId, const PB::Battle::BattleSnapshotPush& push) override;
    void sendEnd(std::uint32_t sessionId, const PB::Battle::BattleEndPush& push) override;
    void sendResult(const PB::BattleInternal::BattleResultReq& req) override;

private:
    battle::SerializedMessagePtr onBattleCreate(const battle::BackendFrame& frame);
    battle::SerializedMessagePtr onBattleRejoin(const battle::BackendFrame& frame);
    battle::SerializedMessagePtr makeBattleCreateResp(std::int32_t code,
                                                      const std::string& message,
                                                      std::uint32_t battleId) const;
    void sendLoadReport();
    void tick(float dt);

private:
    BattleServerConfig m_config;
    battle::BackendClient m_backend;
    std::unique_ptr<battle::BattleRoomManager> m_roomManager;
    bool m_running = false;
    float m_loadReportTimer = 0.0f;
};
