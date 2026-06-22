#include "BattleServer.h"
#include "game.pb.h"
#include "gateway.pb.h"
#include "mugen/Components.h"
#include "mugen/GameWord.h"
#include "mugen/core/serialize/ByteBuffer.h"
#include <chrono>
#include <cstdio>
#include <thread>

using namespace mugen;

BattleServer::BattleServer()
    : m_running(false)
    , m_battleIdSeed(0)
    , m_randomSeed(0xBA771E)
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
    gwConfig.host = config.gatewayHost;
    gwConfig.port = config.gatewayPort;
    gwConfig.serviceId = SERVICE_ID_BATTLE;
    gwConfig.instanceId = config.instanceId;
    gwConfig.reconnectInterval = config.reconnectInterval;

    m_gateway.init(gwConfig);
    m_gateway.setMsgCallback([this](uint8_t cmd, uint16_t msgId, int32_t serial, uint32_t sessionId,
                                    const std::string_view& payload) {
        this->onGatewayMsg(cmd, msgId, serial, sessionId, payload);
    });
    m_gateway.setDisconnectCallback([this]() {
        printf("[BattleServer] gateway disconnected, clearing all battles\n");
        m_battles.clear();
        m_sessionToBattle.clear();
        m_sessionToActor.clear();
    });

    return true;
}

void BattleServer::run()
{
    m_running = true;
    m_gateway.start();

    const auto tickInterval = std::chrono::microseconds(1000000 / m_config.tickRate);
    auto lastTime = std::chrono::steady_clock::now();

    printf("[BattleServer] running at %d tick/s, instance_id=%u\n", m_config.tickRate, m_config.instanceId);

    while (m_running)
    {
        auto now = std::chrono::steady_clock::now();
        float dt = std::chrono::duration<float>(now - lastTime).count();
        lastTime = now;

        m_gateway.update(dt);
        tick(dt);

        auto elapsed = std::chrono::steady_clock::now() - now;
        if (elapsed < tickInterval)
            std::this_thread::sleep_for(tickInterval - elapsed);
    }

    m_gateway.stop();
    printf("[BattleServer] stopped\n");
}

void BattleServer::shutdown()
{
    m_running = false;
}

void BattleServer::onGatewayMsg(uint8_t cmd, uint16_t msgId, int32_t serial, uint32_t sessionId,
                                const std::string_view& payload)
{
    switch (cmd)
    {
    case CMD_GATEWAY_CONTROL:
        if (msgId == PB::Gateway::SessionOnlinePush::Id)
        {
            PB::Gateway::SessionOnlinePush push;
            if (push.ParseFromArray(payload.data(), static_cast<int>(payload.size())))
                onSessionOnline(push.session_id());
        }
        else if (msgId == PB::Gateway::SessionOfflinePush::Id)
        {
            PB::Gateway::SessionOfflinePush push;
            if (push.ParseFromArray(payload.data(), static_cast<int>(payload.size())))
                onSessionOffline(push.session_id());
        }
        break;

    case CMD_BUSINESS:
        if (msgId == PB::Game::BattleJoinReq::Id)
            onBattleJoin(sessionId, serial, payload);
        else if (msgId == PB::Game::BattleInputPush::Id)
            onBattleInput(sessionId, payload);
        break;

    default:
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
    removePlayer(sessionId);
}

void BattleServer::onBattleJoin(uint32_t sessionId, int32_t serial, const std::string_view& payload)
{
    PB::Game::BattleJoinReq req;
    if (!req.ParseFromArray(payload.data(), static_cast<int>(payload.size())))
    {
        sendJoinResp(sessionId, serial, 1, "invalid BattleJoinReq", nullptr);
        return;
    }

    if (auto it = m_sessionToBattle.find(sessionId); it != m_sessionToBattle.end())
    {
        auto battleIt = m_battles.find(it->second);
        sendJoinResp(sessionId, serial, battleIt != m_battles.end() ? 0 : 2,
                     battleIt != m_battles.end() ? "" : "battle not found",
                     battleIt != m_battles.end() ? battleIt->second.get() : nullptr);
        return;
    }

    BattleInstance* battle = findJoinableBattle(req.map_id());
    if (!battle)
        battle = createBattle(req.map_id());

    if (!battle)
    {
        sendJoinResp(sessionId, serial, 3, "create battle failed", nullptr);
        return;
    }

    if (!addPlayerToBattle(*battle, sessionId))
    {
        sendJoinResp(sessionId, serial, 4, "battle is full", nullptr);
        return;
    }

    m_gateway.bindService(sessionId, SERVICE_ID_BATTLE, static_cast<int32_t>(m_config.instanceId));
    sendJoinResp(sessionId, serial, 0, "", battle);
    sendSnapshot(*battle);
}

void BattleServer::onBattleInput(uint32_t sessionId, const std::string_view& payload)
{
    PB::Game::BattleInputPush input;
    if (!input.ParseFromArray(payload.data(), static_cast<int>(payload.size())))
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

BattleInstance* BattleServer::findJoinableBattle(int32_t mapId)
{
    for (auto& [_, battle] : m_battles)
    {
        if (battle->mapId == mapId && battle->players.size() < 2)
            return battle.get();
    }
    return nullptr;
}

BattleInstance* BattleServer::createBattle(int32_t mapId)
{
    auto battle = std::make_unique<BattleInstance>();
    battle->battleId = ++m_battleIdSeed;
    battle->mapId = mapId <= 0 ? 1 : mapId;
    battle->world = std::make_unique<mugen::GameWord>();

    if (!battle->world->init(++m_randomSeed) || !battle->world->loadMap(battle->mapId))
    {
        printf("[BattleServer] failed to initialize battle %u map=%d\n", battle->battleId, battle->mapId);
        return nullptr;
    }

    const uint32_t battleId = battle->battleId;
    m_battles.emplace(battleId, std::move(battle));
    printf("[BattleServer] battle %u created map=%d\n", battleId, mapId);
    return m_battles[battleId].get();
}

bool BattleServer::addPlayerToBattle(BattleInstance& battle, uint32_t sessionId)
{
    if (battle.players.size() >= 2)
        return false;

    auto directorComp = MG_GET_COMPONENT(battle.world->getDirector(), DirectorComponent);
    const bool primary = battle.players.empty();
    mugen::EntityId actorId = primary ? directorComp->primaryActorEntityId : directorComp->secondaryActorEntityId;
    if (actorId == mugen::INVALID_ENTITY_ID)
        return false;

    battle.players.insert(sessionId);
    m_sessionToBattle[sessionId] = battle.battleId;
    m_sessionToActor[sessionId] = actorId;
    printf("[BattleServer] session %u joined battle %u actor=%u\n", sessionId, battle.battleId, actorId);
    return true;
}

void BattleServer::removePlayer(uint32_t sessionId)
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
            printf("[BattleServer] battle %u destroyed (no players)\n", battleIt->first);
            m_battles.erase(battleIt);
        }
    }

    m_sessionToBattle.erase(it);
    m_sessionToActor.erase(sessionId);
    m_gateway.unbindService(sessionId, SERVICE_ID_BATTLE);
}

std::string BattleServer::serializeWorld(const BattleInstance& battle) const
{
    mugen::ByteBuffer buffer(1024 * 1024 * 2);
    battle.world->serialize(buffer);
    buffer.writeFinish();
    return std::string(reinterpret_cast<const char*>(buffer.data()), buffer.len());
}

void BattleServer::sendJoinResp(uint32_t sessionId, int32_t serial, int32_t code, const std::string& message,
                                const BattleInstance* battle)
{
    PB::Game::BattleJoinResp resp;
    resp.set_code(code);
    resp.set_message(message);
    if (battle)
    {
        resp.set_battle_id(battle->battleId);
        resp.set_server_frame(battle->serverFrame);
        auto dump = serializeWorld(*battle);
        resp.set_world_dump(dump);
    }

    m_gateway.sendResponse(sessionId, serial < 0 ? -serial : serial, resp);
}

void BattleServer::sendSnapshot(const BattleInstance& battle)
{
    PB::Game::BattleSnapshotPush push;
    push.set_battle_id(battle.battleId);
    push.set_server_frame(battle.serverFrame);
    push.set_server_time_ms(static_cast<uint64_t>(battle.elapsed * 1000.0f));
    auto dump = serializeWorld(battle);
    push.set_world_dump(dump);

    for (uint32_t sessionId : battle.players)
        m_gateway.sendPush(sessionId, push);
}

void BattleServer::tick(float dt)
{
    const float fixedDt = 1.0f / static_cast<float>(m_config.tickRate);
    for (auto& [_, battle] : m_battles)
    {
        battle->elapsed += fixedDt;
        battle->serverFrame += 1;
        battle->world->update(fixedDt);
        sendSnapshot(*battle);
    }
}
