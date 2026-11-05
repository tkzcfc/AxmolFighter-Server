#pragma once

#include "client_battle.pb.h"
#include "game_battle.pb.h"

#include <cstdint>

namespace battle
{

// 战斗房间对外发送接口：把网络层与房间逻辑解耦，便于无网络单测。
class IBattleSender
{
public:
    virtual ~IBattleSender() = default;

    // 下发战斗开始参数。
    virtual void sendStart(std::uint32_t sessionId, const PB::Battle::BattleStartPush& push) = 0;
    // 广播已确认帧。
    virtual void sendFrames(std::uint32_t sessionId, const PB::Battle::BattleFramesPush& push) = 0;
    // 下发重同步快照。
    virtual void sendSnapshot(std::uint32_t sessionId, const PB::Battle::BattleSnapshotPush& push) = 0;
    // 下发战斗结束。
    virtual void sendEnd(std::uint32_t sessionId, const PB::Battle::BattleEndPush& push) = 0;
    // 上报结果给 game 服。
    virtual void sendResult(const PB::BattleInternal::BattleResultReq& req) = 0;
};

}  // namespace battle
