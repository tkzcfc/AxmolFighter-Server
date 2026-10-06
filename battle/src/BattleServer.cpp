#include "BattleServer.h"

#include "framework/Logger.h"
#include "game_battle.pb.h"
#include "mugen/conf/Config.h"
#include "mugen/avatar/data/AvatarAssetCache.h"
#include "mugen/core/io/FileUtils.h"

#include <chrono>
#include <spdlog/spdlog.h>

using namespace mugen;

BattleServer::BattleServer() = default;

BattleServer::~BattleServer()
{
    shutdown();
}

bool BattleServer::init(const BattleServerConfig& config)
{
    m_config = config;

    if (!m_config.contentRoot.empty())
    {
        mugen::io::clearSearchPaths();
        const std::string zhcn = m_config.contentRoot + "/res_zhcn";
        if (mugen::io::isDirectoryExist(zhcn))
            mugen::io::addSearchPath(zhcn, true);
        mugen::io::addSearchPath(m_config.contentRoot, true);
        if (!mugen::Config::getInstance()->loadConfig("mugen/config/config.bin"))
        {
            spdlog::error("Failed to load config.bin from content_root={}", m_config.contentRoot);
            return false;
        }
        auto* avatarCache = mugen::AvatarAssetCache::getInstance();
        avatarCache->addSearchPath("res_zhcn");
        if (!avatarCache->load("mugen/config/avatar.bin"))
        {
            spdlog::error("Failed to load avatar.bin from content_root={}", m_config.contentRoot);
            return false;
        }
        spdlog::info("BattleServer content_root={}", m_config.contentRoot);
    }

    m_roomManager = std::make_unique<battle::BattleRoomManager>(m_config.maxBattles, m_config.maxSessions);
    m_sync        = std::make_unique<battle::BattleSync>(m_backend);

    battle::BackendConfig backendConfig;
    backendConfig.serviceId = battle::kServiceIdBattle;
    backendConfig.instanceId = m_config.instanceId;
    backendConfig.gatewayHost = m_config.gatewayHost;
    backendConfig.gatewayPort = m_config.gatewayPort;
    backendConfig.initialLoadScore = 0;
    backendConfig.initialAcceptingBindings = m_config.maxBattles > 0 && m_config.maxSessions > 0;
    backendConfig.initialLoadMessage = backendConfig.initialAcceptingBindings
        ? ""
        : "battle server capacity is full";

    return m_backend.init(backendConfig, this);
}

void BattleServer::run()
{
    m_running = true;
    spdlog::info("BattleServer running tick_rate={} instance_id={}",
                 m_config.tickRate,
                 m_config.instanceId);

    const auto tickInterval = std::chrono::microseconds(1000000 / m_config.tickRate);
    m_backend.schedule(tickInterval, [this](yasio::io_service&) {
        if (!m_running)
            return true;

        tick(1.0f / static_cast<float>(m_config.tickRate));
        return false;
    });

    m_backend.start();
    spdlog::info("BattleServer stopped");
}

void BattleServer::shutdown()
{
    m_running = false;
    m_backend.stop();
}

void BattleServer::onConnected(battle::BackendClient& client)
{
    (void)client;
    spdlog::info("BattleServer connected to gateway");
}

void BattleServer::onDisconnected(battle::BackendClient& client)
{
    (void)client;
    spdlog::warn("BattleServer gateway disconnected, shutting down");
    m_roomManager.reset();
    shutdown();
}

battle::SerializedMessagePtr BattleServer::onServerRequest(battle::BackendClient& client,
                                                           battle::ServerSource source,
                                                           const battle::BackendFrame& frame)
{
    (void)client;
    spdlog::debug("Server request source={}/{} msg_id={} serial={}",
                  source.serviceId,
                  source.instanceId,
                  frame.msgId,
                  frame.serial);

    if (frame.msgId == PB::BattleInternal::BattleCreateReq::Id)
        return onBattleCreate(frame);

    return nullptr;
}

void BattleServer::onServerPush(battle::BackendClient& client,
                                battle::ServerSource source,
                                const battle::BackendFrame& frame)
{
    (void)client;
    spdlog::debug("Unhandled server push source={}/{} msg_id={}",
                  source.serviceId,
                  source.instanceId,
                  frame.msgId);
}

void BattleServer::onServerOnline(battle::BackendClient& client,
                                  std::uint32_t serviceId,
                                  std::uint32_t instanceId)
{
    (void)client;
    spdlog::info("Backend service online service_id={} instance_id={}", serviceId, instanceId);
}

void BattleServer::onServerOffline(battle::BackendClient& client,
                                   std::uint32_t serviceId,
                                   std::uint32_t instanceId)
{
    (void)client;
    spdlog::warn("Backend service offline service_id={} instance_id={}", serviceId, instanceId);
    if (serviceId == battle::kServiceIdGame)
    {
        spdlog::warn("Game server offline, shutting down BattleServer");
        shutdown();
    }
}

void BattleServer::onShutdown(battle::BackendClient& client)
{
    (void)client;
}

void BattleServer::onSessionOnline(battle::BackendClient& client, std::uint32_t sessionId)
{
    (void)client;
    spdlog::info("Session online: {}", sessionId);
}

void BattleServer::onSessionOffline(battle::BackendClient& client, std::uint32_t sessionId)
{
    (void)client;
    spdlog::info("Session offline: {}", sessionId);
    if (m_roomManager)
    {
        m_roomManager->removePlayer(sessionId);
        m_backend.unbindService(sessionId, battle::kServiceIdBattle);
    }
}

battle::SerializedMessagePtr BattleServer::onClientRequest(battle::BackendClient& client,
                                                           std::uint32_t sessionId,
                                                           const battle::BackendFrame& frame)
{
    (void)client;
    spdlog::debug("Unhandled client request session={} msg_id={}", sessionId, frame.msgId);
    return nullptr;
}

void BattleServer::onClientPush(battle::BackendClient& client,
                                std::uint32_t sessionId,
                                const battle::BackendFrame& frame)
{
    (void)client;
    if (frame.msgId == PB::Battle::BattleInputPush::Id)
    {
        onBattleInput(sessionId, frame);
        return;
    }

    spdlog::debug("Unhandled client push session={} msg_id={}", sessionId, frame.msgId);
}

battle::SerializedMessagePtr BattleServer::onBattleCreate(const battle::BackendFrame& frame)
{
    PB::BattleInternal::BattleCreateReq req;
    if (!battle::parsePayload(req, frame.payload))
        return makeBattleCreateResp(1, "invalid BattleCreateReq", nullptr);

    // 重复创建：直接返回已有房间状态
    if (auto* existing = m_roomManager->findRoom(req.battle_id()))
    {
        const auto actorId = existing->actorIdForSession(frame.sessionId);
        return makeBattleCreateResp(0, "", existing, actorId);
    }

    battle::BattleRoomConfig roomConfig;
    roomConfig.battleId   = req.battle_id();
    roomConfig.mapId      = req.map_id() <= 0 ? 1 : req.map_id();
    roomConfig.randomSeed = req.random_seed() != 0 ? req.random_seed() : ++m_randomSeed;
    roomConfig.mode       = "duel";
    roomConfig.maxPlayers = 2;

    auto* room = m_roomManager->createRoom(roomConfig);
    if (!room)
        return makeBattleCreateResp(3, "create battle failed", nullptr);

    bool joined = false;
    std::uint32_t requesterActorId = 0;
    for (const auto& player : req.players())
    {
        const bool ok = m_roomManager->addPlayerToRoom(*room, player);
        if (!ok)
            continue;

        if (player.session_id() == frame.sessionId || frame.sessionId == 0)
        {
            joined = true;
            requesterActorId = room->actorIdForSession(player.session_id());
        }
    }

    if (!joined && req.players_size() > 0)
        return makeBattleCreateResp(4, "battle is full or spawn failed", nullptr);

    m_sync->sendSnapshot(*room);
    return makeBattleCreateResp(0, "", room, requesterActorId);
}

void BattleServer::onBattleInput(std::uint32_t sessionId, const battle::BackendFrame& frame)
{
    PB::Battle::BattleInputPush input;
    if (!battle::parsePayload(input, frame.payload))
        return;

    auto* room = m_roomManager->findRoomBySession(sessionId);
    if (!room || room->battleId() != input.battle_id())
        return;

    room->applyInput(sessionId, input.client_frame(), input.input_mask());
}

battle::SerializedMessagePtr BattleServer::makeBattleCreateResp(std::int32_t code,
                                                                const std::string& message,
                                                                const battle::BattleRoom* room,
                                                                std::uint32_t requesterActorId) const
{
    PB::BattleInternal::BattleCreateResp resp;
    resp.set_code(code);
    resp.set_message(message);
    if (room)
    {
        resp.set_battle_id(room->battleId());
        resp.set_battle_instance_id(m_config.instanceId);
        resp.set_server_frame(room->serverFrame());
        resp.set_world_dump(room->serializeWorld());
        resp.set_actor_entity_id(requesterActorId);

        for (const auto& [sessionId, slot] : room->players())
        {
            auto* spawn = resp.add_players();
            spawn->set_session_id(sessionId);
            spawn->set_actor_entity_id(slot.actorId);
        }
    }
    return battle::makeSerializedMessage(resp);
}

void BattleServer::sendLoadReport()
{
    const std::uint32_t loadScore = m_config.maxBattles == 0
        ? 100
        : static_cast<std::uint32_t>((m_roomManager->roomCount() * 100) / m_config.maxBattles);
    const bool accepting = m_roomManager->canAcceptSession(0);
    m_backend.reportLoad(loadScore > 100 ? 100 : loadScore,
                         accepting,
                         accepting ? "" : "battle server capacity is full");
}

void BattleServer::tick(float dt)
{
    if (!m_roomManager)
        return;

    m_loadReportTimer += dt;
    if (m_loadReportTimer >= m_config.loadReportInterval)
    {
        m_loadReportTimer = 0.0f;
        sendLoadReport();
    }

    m_roomManager->tickAll(dt);

    const int snapshotInterval =
        m_config.snapshotIntervalFrames > 0 ? m_config.snapshotIntervalFrames : 1;
    m_roomManager->forEachRoom([this, snapshotInterval](battle::BattleRoom& room) {
        if (room.isEmpty())
            return;
        if (room.serverFrame() % static_cast<std::uint32_t>(snapshotInterval) != 0)
            return;
        m_sync->sendSnapshot(room);
    });
}
