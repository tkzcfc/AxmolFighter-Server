#pragma once

#include "mugen/core/ecs/Types.h"

#include <cstdint>
#include <string>

namespace battle
{

// 战斗房间创建参数。
struct BattleRoomConfig
{
    std::uint32_t battleId = 0;
    std::int32_t mapId = 1;
    std::uint64_t randomSeed = 0;
    // 战斗模式：duel/team 等，当前仅作预留。
    std::string mode = "duel";
    // 最大玩家数，当前决斗为 2。
    std::uint32_t maxPlayers = 2;
};

// 单个已入座玩家的状态。
struct PlayerSlot
{
    std::uint32_t sessionId = 0;
    std::int64_t playerId = 0;
    std::string name;
    std::int32_t classId = 0;
    mugen::EntityId actorId = mugen::INVALID_ENTITY_ID;
    // 服务器最近一次应用的该玩家客户端帧号（OW 输入 ack）。
    std::uint32_t lastClientFrame = 0;
};

}  // namespace battle
