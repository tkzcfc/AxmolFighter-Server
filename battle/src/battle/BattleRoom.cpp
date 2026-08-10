#include "battle/BattleRoom.h"

#include "framework/Logger.h"
#include "game_types.pb.h"
#include "mugen/ActorSpawner.h"
#include "mugen/Components.h"
#include "mugen/GameWord.h"
#include "mugen/conf/Config.h"
#include "mugen/conf/GameDef.h"
#include "mugen/core/serialize/ByteBuffer.h"

#include <spdlog/spdlog.h>

using namespace mugen;

namespace battle
{

BattleRoom::BattleRoom() = default;

BattleRoom::~BattleRoom()
{
    shutdown();
}

bool BattleRoom::init(const BattleRoomConfig& config)
{
    m_config = config;
    m_world  = std::make_unique<mugen::GameWord>();

    if (!m_world->init(config.randomSeed) || !m_world->loadMap(config.mapId))
    {
        spdlog::error("BattleRoom {}: failed to init world map={}", config.battleId, config.mapId);
        m_world = nullptr;
        return false;
    }

    spdlog::info("BattleRoom {} created map={} mode={} max_players={}",
                 config.battleId, config.mapId, config.mode, config.maxPlayers);
    return true;
}

void BattleRoom::shutdown()
{
    m_players.clear();
    m_world.reset();
}

bool BattleRoom::addPlayer(const PB::Types::BattlePlayerSpec& spec)
{
    if (!m_world || isFull())
        return false;

    const std::uint32_t sessionId = spec.session_id();
    if (hasPlayer(sessionId))
        return true;

    const int32_t roleId = actor_spawner::resolvePlayableRoleId(spec.class_id());
    if (!Config::getInstance()->getRoleConfigById(roleId))
    {
        spdlog::error("BattleRoom {}: role config not found for class_id={} roleId={}", m_config.battleId,
                      spec.class_id(), roleId);
        return false;
    }

    const std::size_t slotIndex = m_players.size();
    const auto [spawnX, spawnY] = resolveSpawnPoint(slotIndex);

    actor_spawner::PlayerSpawnParams params;
    params.playerId = spec.player_id();
    params.name     = spec.name();

    auto* actor = actor_spawner::spawnRolePlayerActor(&m_world->ecsManager, roleId, spawnX, spawnY, params);
    if (!actor)
    {
        spdlog::error("BattleRoom {}: failed to spawn player session={}", m_config.battleId, sessionId);
        return false;
    }
    actor->notifyEntityReady();

    PlayerSlot slot;
    slot.sessionId = sessionId;
    slot.playerId  = spec.player_id();
    slot.name      = spec.name();
    slot.classId   = spec.class_id();
    slot.actorId   = actor->getId();
    m_players.emplace(sessionId, slot);

    spdlog::info("BattleRoom {}: session={} actor={} class_id={} pos=({},{})",
                 m_config.battleId, sessionId, slot.actorId, spec.class_id(), spawnX, spawnY);
    return true;
}

void BattleRoom::removePlayer(std::uint32_t sessionId)
{
    m_players.erase(sessionId);
}

bool BattleRoom::hasPlayer(std::uint32_t sessionId) const
{
    return m_players.find(sessionId) != m_players.end();
}

bool BattleRoom::isFull() const
{
    return m_players.size() >= m_config.maxPlayers;
}

bool BattleRoom::isEmpty() const
{
    return m_players.empty();
}

void BattleRoom::applyInput(std::uint32_t sessionId, std::uint32_t clientFrame, std::uint32_t inputMask)
{
    auto it = m_players.find(sessionId);
    if (it == m_players.end() || !m_world)
        return;

    auto actor = m_world->ecsManager.getEntity(it->second.actorId);
    if (!actor)
        return;

    auto inputComp = MG_GET_COMPONENT(actor, InputComponent);
    if (!inputComp)
        return;

    inputComp->keyDown        = inputMask;
    it->second.lastClientFrame = clientFrame;
}

void BattleRoom::tick(float fixedDt)
{
    if (!m_world || isEmpty())
        return;

    m_elapsed += fixedDt;
    ++m_serverFrame;
    m_world->update(fixedDt);
}

std::string BattleRoom::serializeWorld() const
{
    if (!m_world)
        return {};

    mugen::ByteBuffer buffer(1024 * 1024 * 2);
    m_world->serialize(buffer);
    buffer.writeFinish();
    return std::string(reinterpret_cast<const char*>(buffer.data()), buffer.len());
}

std::uint32_t BattleRoom::lastClientFrame(std::uint32_t sessionId) const
{
    auto it = m_players.find(sessionId);
    return it != m_players.end() ? it->second.lastClientFrame : 0;
}

mugen::EntityId BattleRoom::actorIdForSession(std::uint32_t sessionId) const
{
    auto it = m_players.find(sessionId);
    return it != m_players.end() ? it->second.actorId : mugen::INVALID_ENTITY_ID;
}

std::pair<std::int32_t, std::int32_t> BattleRoom::resolveSpawnPoint(std::size_t slotIndex) const
{
    if (!m_world)
        return {0, 0};

    auto* director = m_world->getDirector();
    auto* directorComp = director ? MG_GET_COMPONENT(director, DirectorComponent) : nullptr;
    auto* mapEntity =
        directorComp ? m_world->ecsManager.getEntity(directorComp->mapEntityId) : nullptr;
    auto* mapComp = mapEntity ? MG_GET_COMPONENT(mapEntity, GameMapComponent) : nullptr;
    if (!mapComp)
    {
        spdlog::warn("BattleRoom {}: GameMapComponent missing, spawn at origin", m_config.battleId);
        return {0, 0};
    }

    if (!mapComp->spawnPoints.empty())
    {
        const auto& first = mapComp->spawnPoints[0];
        return {first.x + static_cast<std::int32_t>(slotIndex) * 200, first.y};
    }

    spdlog::warn("BattleRoom {}: map {} has no spawnPoints, fallback to scope", m_config.battleId, m_config.mapId);
    const auto& scope = mapComp->scope;
    if (slotIndex == 0)
        return {scope.x + scope.width / 4, scope.y + scope.height / 2};
    return {scope.x + (scope.width * 3) / 4 + static_cast<std::int32_t>(slotIndex - 1) * 200,
            scope.y + scope.height / 2};
}

}  // namespace battle
