#include "battle/BattleRoomManager.h"

#include "framework/Logger.h"
#include "game_types.pb.h"

#include <spdlog/spdlog.h>

namespace battle
{

BattleRoomManager::BattleRoomManager(std::uint32_t maxRooms, std::uint32_t maxSessions)
    : m_maxRooms(maxRooms), m_maxSessions(maxSessions)
{
}

BattleRoom* BattleRoomManager::createRoom(const BattleRoomConfig& config)
{
    if (config.battleId == 0)
        return nullptr;

    if (auto it = m_rooms.find(config.battleId); it != m_rooms.end())
        return it->second.get();

    if (m_rooms.size() >= m_maxRooms)
    {
        spdlog::warn("Max battle count reached: {}/{}", m_rooms.size(), m_maxRooms);
        return nullptr;
    }

    auto room = std::make_unique<BattleRoom>();
    if (!room->init(config))
        return nullptr;

    const auto battleId = config.battleId;
    m_rooms.emplace(battleId, std::move(room));
    spdlog::info("BattleRoomManager: room {} created", battleId);
    return m_rooms[battleId].get();
}

BattleRoom* BattleRoomManager::findRoom(std::uint32_t battleId)
{
    auto it = m_rooms.find(battleId);
    return it != m_rooms.end() ? it->second.get() : nullptr;
}

BattleRoom* BattleRoomManager::findRoomBySession(std::uint32_t sessionId)
{
    auto it = m_sessionToRoom.find(sessionId);
    if (it == m_sessionToRoom.end())
        return nullptr;
    return findRoom(it->second);
}

bool BattleRoomManager::addPlayerToRoom(BattleRoom& room, const PB::Types::BattlePlayerSpec& spec)
{
    const std::uint32_t sessionId = spec.session_id();

    if (m_sessionToRoom.find(sessionId) != m_sessionToRoom.end())
        return true;

    if (!canAcceptSession(sessionId))
        return false;

    if (!room.addPlayer(spec))
        return false;

    m_sessionToRoom[sessionId] = room.battleId();
    return true;
}

void BattleRoomManager::removePlayer(std::uint32_t sessionId)
{
    auto it = m_sessionToRoom.find(sessionId);
    if (it == m_sessionToRoom.end())
        return;

    if (auto* room = findRoom(it->second))
    {
        room->removePlayer(sessionId);
        if (room->isEmpty())
        {
            spdlog::info("BattleRoom {} destroyed, no players remain", room->battleId());
            m_rooms.erase(room->battleId());
        }
    }

    m_sessionToRoom.erase(it);
}

void BattleRoomManager::tickAll(float fixedDt)
{
    for (auto& [_, room] : m_rooms)
        room->tick(fixedDt);
}

bool BattleRoomManager::canAcceptSession(std::uint32_t sessionId) const
{
    if (m_sessionToRoom.find(sessionId) != m_sessionToRoom.end())
        return true;
    if (m_rooms.size() >= m_maxRooms)
        return false;
    return activeSessionCount() < m_maxSessions;
}

}  // namespace battle
