#pragma once

#include "battle/BattleRoom.h"
#include "battle/IBattleSender.h"

#include <cstdint>
#include <memory>
#include <unordered_map>

namespace battle
{

// 管理本服所有战斗房间，负责 battle_id / session 路由与容量控制。
class BattleRoomManager
{
public:
    explicit BattleRoomManager(std::uint32_t maxRooms = 100, std::uint32_t maxSessions = 200);

    // 创建房间；battle_id 已存在或超容量返回 nullptr。
    BattleRoom* createRoom(IBattleSender& sender, const BattleRoomConfig& config);
    BattleRoom* findRoom(std::uint32_t battleId);
    BattleRoom* findRoomBySession(std::uint32_t sessionId);
    // 玩家用新会话重新接入进行中的战斗：改绑槽位并更新 session 路由。
    RejoinResult rejoin(std::uint32_t battleId, std::int64_t playerId, std::uint32_t newSessionId);

    void tickAll(float dt);
    // 清理已结束且超过销毁宽限的房间。
    void collectGarbage();

    std::size_t roomCount() const { return m_rooms.size(); }
    std::uint32_t activeSessionCount() const
    {
        return static_cast<std::uint32_t>(m_sessionToRoom.size());
    }
    bool canAcceptSession() const;

private:
    std::uint32_t m_maxRooms;
    std::uint32_t m_maxSessions;
    std::unordered_map<std::uint32_t, std::unique_ptr<BattleRoom>> m_rooms;
    std::unordered_map<std::uint32_t, std::uint32_t> m_sessionToRoom;
};

}  // namespace battle
