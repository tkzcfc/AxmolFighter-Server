#include "BattleServer.h"

#include "framework/Logger.h"
#include "mugen/Components.h"
#include "mugen/GameWord.h"
#include "mugen/core/serialize/ByteBuffer.h"

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

    battle::BackendConfig backendConfig;
    backendConfig.serviceId = battle::kServiceIdBattle;
    backendConfig.instanceId = config.instanceId;
    backendConfig.gatewayHost = config.gatewayHost;
    backendConfig.gatewayPort = config.gatewayPort;
    backendConfig.initialLoadScore = 0;
    backendConfig.initialAcceptingBindings = config.maxBattles > 0 && config.maxSessions > 0;
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
    spdlog::warn("BattleServer gateway disconnected, clearing battle state");
    m_battles.clear();
    m_sessionToBattle.clear();
    m_sessionToActor.clear();
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

    if (frame.msgId == PB::Game::BattleCreateReq::Id)
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

void BattleServer::onShutdown(battle::BackendClient& client)
{
    (void)client;
    m_battles.clear();
    m_sessionToBattle.clear();
    m_sessionToActor.clear();
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
    removePlayer(sessionId);
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
    if (frame.msgId == PB::Game::BattleInputPush::Id)
    {
        onBattleInput(sessionId, frame);
        return;
    }

    spdlog::debug("Unhandled client push session={} msg_id={}", sessionId, frame.msgId);
}

battle::SerializedMessagePtr BattleServer::onBattleCreate(const battle::BackendFrame& frame)
{
    PB::Game::BattleCreateReq req;
    if (!battle::parsePayload(req, frame.payload))
        return makeBattleCreateResp(1, "invalid BattleCreateReq", nullptr);

    if (auto it = m_sessionToBattle.find(frame.sessionId); it != m_sessionToBattle.end())
    {
        auto battleIt = m_battles.find(it->second);
        return makeBattleCreateResp(battleIt != m_battles.end() ? 0 : 2,
                                    battleIt != m_battles.end() ? "" : "battle not found",
                                    battleIt != m_battles.end() ? battleIt->second.get() : nullptr);
    }

    BattleInstance* battle = createBattle(req.battle_id(), req.map_id());
    if (!battle)
        return makeBattleCreateResp(3, "create battle failed", nullptr);

    bool joined = false;
    for (const auto& player : req.players())
    {
        const bool ok = addPlayerToBattle(*battle, player.session_id());
        if (player.session_id() == frame.sessionId || frame.sessionId == 0)
            joined = joined || ok;
    }

    if (!joined && req.players_size() > 0)
        return makeBattleCreateResp(4, "battle is full", nullptr);

    sendSnapshot(*battle);
    return makeBattleCreateResp(0, "", battle);
}

void BattleServer::onBattleInput(std::uint32_t sessionId, const battle::BackendFrame& frame)
{
    PB::Game::BattleInputPush input;
    if (!battle::parsePayload(input, frame.payload))
        return;

    auto battleIt = m_sessionToBattle.find(sessionId);
    if (battleIt == m_sessionToBattle.end() || battleIt->second != input.battle_id())
        return;

    auto actorIt = m_sessionToActor.find(sessionId);
    auto battleMapIt = m_battles.find(battleIt->second);
    if (actorIt == m_sessionToActor.end() || battleMapIt == m_battles.end())
        return;

    auto actor = battleMapIt->second->world->ecsManager.getEntity(actorIt->second);
    if (!actor)
        return;

    auto inputComp = MG_GET_COMPONENT(actor, InputComponent);
    if (!inputComp)
        return;

    inputComp->keyDown = input.input_mask();
}

BattleInstance* BattleServer::createBattle(std::uint32_t battleId, std::int32_t mapId)
{
    if (battleId == 0)
        return nullptr;

    if (auto it = m_battles.find(battleId); it != m_battles.end())
        return it->second.get();

    if (m_battles.size() >= m_config.maxBattles)
    {
        spdlog::warn("Max battle count reached: {}/{}", m_battles.size(), m_config.maxBattles);
        return nullptr;
    }

    auto battle = std::make_unique<BattleInstance>();
    battle->battleId = battleId;
    battle->mapId = mapId <= 0 ? 1 : mapId;
    battle->world = std::make_unique<mugen::GameWord>();

    if (!battle->world->init(++m_randomSeed) || !battle->world->loadMap(battle->mapId))
    {
        spdlog::error("Failed to initialize battle {} map={}", battle->battleId, battle->mapId);
        return nullptr;
    }

    const auto createdBattleId = battle->battleId;
    m_battles.emplace(createdBattleId, std::move(battle));
    spdlog::info("Battle {} created map={}", createdBattleId, mapId);
    return m_battles[createdBattleId].get();
}

bool BattleServer::addPlayerToBattle(BattleInstance& battle, std::uint32_t sessionId)
{
    if (activeSessionCount() >= m_config.maxSessions &&
        m_sessionToBattle.find(sessionId) == m_sessionToBattle.end())
    {
        return false;
    }

    if (battle.players.size() >= 2)
        return false;

    auto directorComp = MG_GET_COMPONENT(battle.world->getDirector(), DirectorComponent);
    const bool primary = battle.players.empty();
    const mugen::EntityId actorId = primary ? directorComp->primaryActorEntityId
                                            : directorComp->secondaryActorEntityId;
    if (actorId == mugen::INVALID_ENTITY_ID)
        return false;

    battle.players.insert(sessionId);
    m_sessionToBattle[sessionId] = battle.battleId;
    m_sessionToActor[sessionId] = actorId;
    spdlog::info("Session {} joined battle {} actor={}", sessionId, battle.battleId, actorId);
    return true;
}

void BattleServer::removePlayer(std::uint32_t sessionId)
{
    auto it = m_sessionToBattle.find(sessionId);
    if (it == m_sessionToBattle.end())
        return;

    auto battleIt = m_battles.find(it->second);
    if (battleIt != m_battles.end())
    {
        battleIt->second->players.erase(sessionId);
        if (battleIt->second->players.empty())
        {
            spdlog::info("Battle {} destroyed, no players remain", battleIt->first);
            m_battles.erase(battleIt);
        }
    }

    m_sessionToBattle.erase(it);
    m_sessionToActor.erase(sessionId);
    m_backend.unbindService(sessionId, battle::kServiceIdBattle);
}

std::string BattleServer::serializeWorld(const BattleInstance& battle) const
{
    mugen::ByteBuffer buffer(1024 * 1024 * 2);
    battle.world->serialize(buffer);
    buffer.writeFinish();
    return std::string(reinterpret_cast<const char*>(buffer.data()), buffer.len());
}

battle::SerializedMessagePtr BattleServer::makeBattleCreateResp(std::int32_t code,
                                                                const std::string& message,
                                                                const BattleInstance* battle) const
{
    PB::Game::BattleCreateResp resp;
    resp.set_code(code);
    resp.set_message(message);
    if (battle)
    {
        resp.set_battle_id(battle->battleId);
        resp.set_battle_instance_id(m_config.instanceId);
        resp.set_server_frame(battle->serverFrame);
        resp.set_world_dump(serializeWorld(*battle));
    }
    return battle::makeSerializedMessage(resp);
}

void BattleServer::sendSnapshot(const BattleInstance& battle)
{
    PB::Game::BattleSnapshotPush push;
    push.set_battle_id(battle.battleId);
    push.set_server_frame(battle.serverFrame);
    push.set_server_time_ms(static_cast<std::uint64_t>(battle.elapsed * 1000.0f));
    push.set_world_dump(serializeWorld(battle));

    for (std::uint32_t sessionId : battle.players)
        m_backend.sendPush(sessionId, push);
}

void BattleServer::sendLoadReport()
{
    const std::uint32_t loadScore = m_config.maxBattles == 0
        ? 100
        : static_cast<std::uint32_t>((m_battles.size() * 100) / m_config.maxBattles);
    const bool accepting = canAcceptBinding(0);
    m_backend.reportLoad(loadScore > 100 ? 100 : loadScore,
                         accepting,
                         accepting ? "" : "battle server capacity is full");
}

std::uint32_t BattleServer::activeSessionCount() const
{
    return static_cast<std::uint32_t>(m_sessionToBattle.size());
}

bool BattleServer::canAcceptBinding(std::uint32_t sessionId) const
{
    if (m_sessionToBattle.find(sessionId) != m_sessionToBattle.end())
        return true;
    if (m_battles.size() >= m_config.maxBattles)
        return false;
    if (activeSessionCount() >= m_config.maxSessions)
        return false;
    return true;
}

void BattleServer::tick(float dt)
{
    m_loadReportTimer += dt;
    if (m_loadReportTimer >= m_config.loadReportInterval)
    {
        m_loadReportTimer = 0.0f;
        if (m_backend.isConnected())
            sendLoadReport();
    }

    const float fixedDt = 1.0f / static_cast<float>(m_config.tickRate);
    for (auto& [_, battle] : m_battles)
    {
        battle->elapsed += fixedDt;
        battle->serverFrame += 1;
        battle->world->update(fixedDt);
        sendSnapshot(*battle);
    }
}
