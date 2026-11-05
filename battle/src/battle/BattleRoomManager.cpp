#include "battle/BattleRoomManager.h"

#include <spdlog/spdlog.h>

#include <vector>

namespace battle
{

BattleRoomManager::BattleRoomManager(std::uint32_t maxRooms, std::uint32_t maxSessions)
    : m_maxRooms(maxRooms), m_maxSessions(maxSessions)
{
}

BattleRoom* BattleRoomManager::createRoom(IBattleSender& sender, const BattleRoomConfig& config)
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

    auto room = BattleRoom::create(sender, config);
    if (!room)
        return nullptr;

    for (const auto& p : config.players)
        m_sessionToRoom[p.sessionId] = config.battleId;

    const auto battleId = config.battleId;
    auto* raw           = room.get();
    m_rooms.emplace(battleId, std::move(room));
    spdlog::info("BattleRoomManager: room {} created", battleId);
    return raw;
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

RejoinResult BattleRoomManager::rejoin(std::uint32_t battleId, std::int64_t playerId, std::uint32_t newSessionId)
{
    auto* room = findRoom(battleId);
    if (!room)
        return RejoinResult::NoBattle;

    std::uint32_t oldSessionId = 0;
    const auto result = room->rejoin(playerId, newSessionId, oldSessionId);
    if (result != RejoinResult::Ok)
        return result;

    // 旧会话的路由移除：之后它迟到的离线通知不再影响该槽位。
    if (oldSessionId != newSessionId)
        m_sessionToRoom.erase(oldSessionId);
    m_sessionToRoom[newSessionId] = battleId;
    return result;
}

void BattleRoomManager::tickAll(float dt)
{
    for (auto& [_, room] : m_rooms)
        room->tick(dt);
}

void BattleRoomManager::collectGarbage()
{
    std::vector<std::uint32_t> toErase;
    for (auto& [battleId, room] : m_rooms)
        if (room->finished() && room->endedSeconds() >= kEndDestroySeconds)
            toErase.push_back(battleId);

    for (const auto battleId : toErase)
    {
        for (auto it = m_sessionToRoom.begin(); it != m_sessionToRoom.end();)
        {
            if (it->second == battleId)
                it = m_sessionToRoom.erase(it);
            else
                ++it;
        }
        m_rooms.erase(battleId);
        spdlog::info("BattleRoomManager: room {} destroyed", battleId);
    }
}

bool BattleRoomManager::canAcceptSession() const
{
    return m_rooms.size() < m_maxRooms && activeSessionCount() < m_maxSessions;
}

}  // namespace battle
