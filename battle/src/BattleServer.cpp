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

namespace
{
struct ForwardedBackendFrame
{
    uint8_t cmd = 0;
    uint16_t msgId = 0;
    int32_t serial = 0;
    uint32_t sessionId = 0;
    std::string_view payload;
};

uint16_t readUint16(const char* data)
{
    return (static_cast<uint16_t>(static_cast<unsigned char>(data[0])) << 8) |
        static_cast<uint16_t>(static_cast<unsigned char>(data[1]));
}

uint32_t readUint32(const char* data)
{
    return (static_cast<uint32_t>(static_cast<unsigned char>(data[0])) << 24) |
        (static_cast<uint32_t>(static_cast<unsigned char>(data[1])) << 16) |
        (static_cast<uint32_t>(static_cast<unsigned char>(data[2])) << 8) |
        static_cast<uint32_t>(static_cast<unsigned char>(data[3]));
}

bool parseForwardedBackendFrame(const std::string& data, ForwardedBackendFrame& frame)
{
    if (data.size() < BACKEND_FRAME_HEADER_SIZE)
        return false;

    const uint32_t frameLen = readUint32(data.data());
    if (frameLen < BACKEND_FRAME_HEADER_SIZE || frameLen > data.size())
        return false;

    frame.cmd = static_cast<uint8_t>(data[4]);
    frame.msgId = readUint16(data.data() + 5);
    frame.serial = static_cast<int32_t>(readUint32(data.data() + 7));
    frame.sessionId = readUint32(data.data() + 11);
    frame.payload = std::string_view(data.data() + BACKEND_FRAME_HEADER_SIZE,
                                     frameLen - BACKEND_FRAME_HEADER_SIZE);
    return true;
}
}

BattleServer::BattleServer()
    : m_running(false)
    , m_randomSeed(0xBA771E)
    , m_loadReportTimer(0.0f)
{
}

BattleServer::~BattleServer()
{
    shutdown();
}

bool BattleServer::init(const BattleServerConfig& config)
{
    m_config = config;

    // battle 作为后端服务连接网关，注册成 battle 实例。
    GatewayClient::Config gwConfig;
    gwConfig.host = config.gatewayHost;
    gwConfig.port = config.gatewayPort;
    gwConfig.serviceId = SERVICE_ID_BATTLE;
    gwConfig.instanceId = config.instanceId;
    gwConfig.reconnectInterval = config.reconnectInterval;
    gwConfig.initialLoadScore = 0;
    gwConfig.initialAcceptingBindings = config.maxBattles > 0 && config.maxSessions > 0;
    gwConfig.initialLoadMessage = gwConfig.initialAcceptingBindings ? "" : "battle server capacity is full";

    m_gateway.init(gwConfig);
    m_gateway.setMsgCallback([this](uint8_t cmd, uint16_t msgId, int32_t serial, uint32_t sessionId,
                                    const std::string_view& payload) {
        this->onGatewayMsg(cmd, msgId, serial, sessionId, payload);
    });
    m_gateway.setDisconnectCallback([this]() {
        // 网关断开后本地绑定都不可靠，直接清掉等待重连。
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
        else if (msgId == PB::Gateway::ForwardToServerReq::Id)
        {
            PB::Gateway::ForwardToServerReq req;
            ForwardedBackendFrame inner;
            if (!req.ParseFromArray(payload.data(), static_cast<int>(payload.size())) ||
                !parseForwardedBackendFrame(req.payload(), inner))
            {
                break;
            }

            if (inner.cmd == CMD_BUSINESS && inner.msgId == PB::Game::BattleCreateReq::Id)
            {
                onBattleCreate(inner.sessionId, inner.serial, inner.payload,
                               req.source_service_id(), req.source_instance_id());
            }
        }
        break;

    case CMD_BUSINESS:
        if (msgId == PB::Game::BattleCreateReq::Id)
            onBattleCreate(sessionId, serial, payload, 0, 0);
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

void BattleServer::onBattleCreate(uint32_t sessionId, int32_t serial, const std::string_view& payload,
                                  uint32_t sourceServiceId, uint32_t sourceInstanceId)
{
    PB::Game::BattleCreateReq req;
    if (!req.ParseFromArray(payload.data(), static_cast<int>(payload.size())))
    {
        sendBattleCreateResp(sourceServiceId, sourceInstanceId, serial, 1,
                             "invalid BattleCreateReq", nullptr);
        return;
    }

    if (auto it = m_sessionToBattle.find(sessionId); it != m_sessionToBattle.end())
    {
        // 重复进入时返回现有战斗，避免重复创建角色。
        auto battleIt = m_battles.find(it->second);
        sendBattleCreateResp(sourceServiceId, sourceInstanceId, serial,
                             battleIt != m_battles.end() ? 0 : 2,
                             battleIt != m_battles.end() ? "" : "battle not found",
                             battleIt != m_battles.end() ? battleIt->second.get() : nullptr);
        return;
    }

    BattleInstance* battle = createBattle(req.battle_id(), req.map_id());
    if (!battle)
    {
        sendBattleCreateResp(sourceServiceId, sourceInstanceId, serial, 3,
                             "create battle failed", nullptr);
        return;
    }

    bool joined = false;
    for (const auto& player : req.players())
    {
        if (player.session_id() == sessionId)
            joined = addPlayerToBattle(*battle, player.session_id());
        else
            addPlayerToBattle(*battle, player.session_id());
    }

    if (!joined)
    {
        sendBattleCreateResp(sourceServiceId, sourceInstanceId, serial, 4, "battle is full", nullptr);
        return;
    }

    sendBattleCreateResp(sourceServiceId, sourceInstanceId, serial, 0, "", battle);
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

BattleInstance* BattleServer::createBattle(uint32_t battleId, int32_t mapId)
{
    if (battleId == 0)
        return nullptr;

    if (auto it = m_battles.find(battleId); it != m_battles.end())
        return it->second.get();

    if (m_battles.size() >= m_config.maxBattles)
    {
        printf("[BattleServer] max battle count reached: %zu/%u\n", m_battles.size(), m_config.maxBattles);
        return nullptr;
    }

    auto battle = std::make_unique<BattleInstance>();
    battle->battleId = battleId;
    battle->mapId = mapId <= 0 ? 1 : mapId;
    battle->world = std::make_unique<mugen::GameWord>();

    if (!battle->world->init(++m_randomSeed) || !battle->world->loadMap(battle->mapId))
    {
        printf("[BattleServer] failed to initialize battle %u map=%d\n", battle->battleId, battle->mapId);
        return nullptr;
    }

    const uint32_t createdBattleId = battle->battleId;
    m_battles.emplace(createdBattleId, std::move(battle));
    printf("[BattleServer] battle %u created map=%d\n", createdBattleId, mapId);
    return m_battles[createdBattleId].get();
}

bool BattleServer::addPlayerToBattle(BattleInstance& battle, uint32_t sessionId)
{
    // session 总量也要限住，避免只限制 battle 数不够用。
    if (activeSessionCount() >= m_config.maxSessions && m_sessionToBattle.find(sessionId) == m_sessionToBattle.end())
        return false;

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

void BattleServer::sendBattleCreateResp(uint32_t targetServiceId, uint32_t targetInstanceId,
                                        int32_t serial, int32_t code, const std::string& message,
                                        const BattleInstance* battle)
{
    PB::Game::BattleCreateResp resp;
    resp.set_code(code);
    resp.set_message(message);
    if (battle)
    {
        resp.set_battle_id(battle->battleId);
        resp.set_battle_instance_id(m_config.instanceId);
        resp.set_server_frame(battle->serverFrame);
        auto dump = serializeWorld(*battle);
        resp.set_world_dump(dump);
    }

    if (targetServiceId == 0 && targetInstanceId == 0)
        return;

    m_gateway.forwardMessageToServer(static_cast<uint8_t>(targetServiceId),
                                     static_cast<int32_t>(targetInstanceId),
                                     serial < 0 ? -serial : serial, 0, resp);
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

uint32_t BattleServer::activeSessionCount() const
{
    return static_cast<uint32_t>(m_sessionToBattle.size());
}

bool BattleServer::canAcceptBinding(uint32_t sessionId) const
{
    // 已经在本服的 session 可以继续通过，便于重试或恢复。
    if (m_sessionToBattle.find(sessionId) != m_sessionToBattle.end())
        return true;
    if (m_battles.size() >= m_config.maxBattles)
        return false;
    if (activeSessionCount() >= m_config.maxSessions)
        return false;
    return true;
}

void BattleServer::sendLoadReport()
{
    // 第一版用 active_battles 作为通用负载值。
    PB::Gateway::ServiceLoadReportPush push;
    const uint32_t loadScore = m_config.maxBattles == 0
        ? 100
        : static_cast<uint32_t>((m_battles.size() * 100) / m_config.maxBattles);
    push.set_load_score(loadScore > 100 ? 100 : loadScore);
    push.set_accepting_bindings(canAcceptBinding(0));
    push.set_message(push.accepting_bindings() ? "" : "battle server capacity is full");
    m_gateway.sendControl(0, 0, push);
}

void BattleServer::tick(float dt)
{
    m_loadReportTimer += dt;
    if (m_loadReportTimer >= m_config.loadReportInterval)
    {
        m_loadReportTimer = 0.0f;
        if (m_gateway.isConnected())
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
