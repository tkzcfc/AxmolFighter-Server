#pragma once

#include "battle/BattleRoom.h"

#include <cstdint>
#include <memory>
#include <unordered_map>

namespace battle
{

// 管理本服所有 BattleRoom，负责 session → room 路由与容量控制。
class BattleRoomManager
{
public:
    explicit BattleRoomManager(std::uint32_t maxRooms = 100, std::uint32_t maxSessions = 200);

    BattleRoom* createRoom(const BattleRoomConfig& config);
    BattleRoom* findRoom(std::uint32_t battleId);
    BattleRoom* findRoomBySession(std::uint32_t sessionId);

    bool addPlayerToRoom(BattleRoom& room, const PB::Types::BattlePlayerSpec& spec);
    void removePlayer(std::uint32_t sessionId);
    void tickAll(float fixedDt);

    template <typename Fn>
    void forEachRoom(Fn&& fn)
    {
        for (auto& [_, room] : m_rooms)
            fn(*room);
    }

    std::size_t roomCount() const { return m_rooms.size(); }
    std::uint32_t activeSessionCount() const { return static_cast<std::uint32_t>(m_sessionToRoom.size()); }
    bool canAcceptSession(std::uint32_t sessionId) const;

private:
    std::uint32_t m_maxRooms;
    std::uint32_t m_maxSessions;
    std::unordered_map<std::uint32_t, std::unique_ptr<BattleRoom>> m_rooms;
    std::unordered_map<std::uint32_t, std::uint32_t> m_sessionToRoom;
};

}  // namespace battle
