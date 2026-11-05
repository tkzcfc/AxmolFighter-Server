#include "BattleServer.h"

#include "battle/BattleContent.h"
#include "framework/Logger.h"
#include "game_battle.pb.h"

#include <chrono>
#include <spdlog/spdlog.h>

BattleServer::BattleServer() = default;

BattleServer::~BattleServer()
{
    shutdown();
}

bool BattleServer::init(const BattleServerConfig& config)
{
    m_config = config;

    if (!m_config.contentRoot.empty() && !battle::mountBattleContent(m_config.contentRoot))
    {
        spdlog::error("BattleServer failed to mount content_root={}", m_config.contentRoot);
        return false;
    }

    m_roomManager =
        std::make_unique<battle::BattleRoomManager>(m_config.maxBattles, m_config.maxSessions);

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
    if (frame.msgId == PB::BattleInternal::BattleRejoinReq::Id)
        return onBattleRejoin(frame);

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
    if (m_roomManager)
        if (auto* room = m_roomManager->findRoomBySession(sessionId))
            room->setSessionOnline(sessionId, true);
}

void BattleServer::onSessionOffline(battle::BackendClient& client, std::uint32_t sessionId)
{
    (void)client;
    spdlog::info("Session offline: {}", sessionId);
    if (m_roomManager)
        if (auto* room = m_roomManager->findRoomBySession(sessionId))
            room->setSessionOnline(sessionId, false);
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

    auto* room = m_roomManager ? m_roomManager->findRoomBySession(sessionId) : nullptr;

    switch (frame.msgId)
    {
    case PB::Battle::BattleReadyReq::Id:
    {
        PB::Battle::BattleReadyReq req;
        if (!battle::parsePayload(req, frame.payload))
            return;
        if (room)
            room->markReady(sessionId);
        break;
    }
    case PB::Battle::BattleInputPush::Id:
    {
        PB::Battle::BattleInputPush input;
        if (!battle::parsePayload(input, frame.payload))
            return;
        if (room)
            room->submitInput(sessionId, input);
        break;
    }
    case PB::Battle::BattleResyncReq::Id:
    {
        PB::Battle::BattleResyncReq req;
        if (!battle::parsePayload(req, frame.payload))
            return;
        if (room)
            room->handleResync(sessionId, req);
        break;
    }
    case PB::Battle::BattleLeaveReq::Id:
    {
        PB::Battle::BattleLeaveReq req;
        if (!battle::parsePayload(req, frame.payload))
            return;
        if (room)
            room->handleLeave(sessionId);
        break;
    }
    default:
        spdlog::debug("Unhandled client push session={} msg_id={}", sessionId, frame.msgId);
        break;
    }
}

battle::SerializedMessagePtr BattleServer::onBattleCreate(const battle::BackendFrame& frame)
{
    PB::BattleInternal::BattleCreateReq req;
    if (!battle::parsePayload(req, frame.payload))
        return makeBattleCreateResp(1, "invalid BattleCreateReq", 0);

    // 重复创建：直接返回已有房间。
    if (auto* existing = m_roomManager->findRoom(req.battle_id()))
        return makeBattleCreateResp(0, "", existing->battleId());

    battle::BattleRoomConfig config;
    config.battleId    = req.battle_id();
    config.roomId      = req.room_id();
    config.randomSeed  = req.random_seed();
    config.pvp         = req.pvp();
    config.reconnectGraceSeconds = m_config.reconnectGraceSeconds;
    for (const auto& player : req.players())
    {
        battle::PlayerSlot slot;
        slot.sessionId = player.session_id();
        slot.playerId  = player.player_id();
        slot.name      = player.name();
        slot.classId   = player.class_id();
        slot.slot      = player.slot();
        slot.camp      = player.camp();
        slot.level     = player.level();
        slot.appearance = player.appearance();
        config.players.push_back(slot);
    }

    if (config.players.empty())
        return makeBattleCreateResp(2, "no players", 0);

    if (!m_roomManager->createRoom(*this, config))
        return makeBattleCreateResp(3, "create battle failed", 0);

    return makeBattleCreateResp(0, "", req.battle_id());
}

battle::SerializedMessagePtr BattleServer::onBattleRejoin(const battle::BackendFrame& frame)
{
    PB::BattleInternal::BattleRejoinResp resp;
    PB::BattleInternal::BattleRejoinReq req;
    if (!battle::parsePayload(req, frame.payload))
    {
        resp.set_code(static_cast<std::int32_t>(battle::RejoinResult::NoBattle));
        resp.set_message("invalid BattleRejoinReq");
        return battle::makeSerializedMessage(resp);
    }

    const auto result = m_roomManager->rejoin(req.battle_id(), req.player_id(), req.session_id());
    resp.set_code(static_cast<std::int32_t>(result));
    switch (result)
    {
    case battle::RejoinResult::Ok:
        break;
    case battle::RejoinResult::NoBattle:
        resp.set_message("battle not found");
        break;
    case battle::RejoinResult::Ended:
        resp.set_message("battle already ended");
        break;
    case battle::RejoinResult::NoPlayer:
        resp.set_message("player not in battle");
        break;
    }
    spdlog::info("BattleRejoin battle={} player={} session={} -> code={}",
                 req.battle_id(), req.player_id(), req.session_id(), resp.code());
    return battle::makeSerializedMessage(resp);
}

battle::SerializedMessagePtr BattleServer::makeBattleCreateResp(std::int32_t code,
                                                                const std::string& message,
                                                                std::uint32_t battleId) const
{
    PB::BattleInternal::BattleCreateResp resp;
    resp.set_code(code);
    resp.set_message(message);
    resp.set_battle_id(battleId);
    resp.set_battle_instance_id(m_config.instanceId);
    return battle::makeSerializedMessage(resp);
}

void BattleServer::sendStart(std::uint32_t sessionId, const PB::Battle::BattleStartPush& push)
{
    m_backend.sendPush(sessionId, push);
}

void BattleServer::sendFrames(std::uint32_t sessionId, const PB::Battle::BattleFramesPush& push)
{
    m_backend.sendPush(sessionId, push);
}

void BattleServer::sendSnapshot(std::uint32_t sessionId, const PB::Battle::BattleSnapshotPush& push)
{
    m_backend.sendPush(sessionId, push);
}

void BattleServer::sendEnd(std::uint32_t sessionId, const PB::Battle::BattleEndPush& push)
{
    m_backend.sendPush(sessionId, push);
}

void BattleServer::sendResult(const PB::BattleInternal::BattleResultReq& req)
{
    m_backend.sendServerPush(battle::ServerSource::anyInstance(battle::kServiceIdGame), req);
}

void BattleServer::sendLoadReport()
{
    const std::uint32_t loadScore = m_config.maxBattles == 0
        ? 100
        : static_cast<std::uint32_t>((m_roomManager->roomCount() * 100) / m_config.maxBattles);
    const bool accepting = m_roomManager->canAcceptSession();
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
    m_roomManager->collectGarbage();
}
