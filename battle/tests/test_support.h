#pragma once
// 战斗服无网络测试共用的辅助：假发送端、断言与决斗房间配置。

#include "battle/BattleRoom.h"
#include "battle/BattleTypes.h"
#include "battle/IBattleSender.h"

#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

namespace battle_test
{

struct FakeSender final : public battle::IBattleSender
{
    std::vector<PB::Battle::BattleStartPush> starts;
    std::vector<PB::Battle::BattleFramesPush> frames;
    std::vector<PB::Battle::BattleSnapshotPush> snapshots;
    std::vector<PB::Battle::BattleEndPush> ends;
    std::vector<PB::BattleInternal::BattleResultReq> results;
    // 与上面各列表一一对应的目标会话。
    std::vector<std::uint32_t> startTo;
    std::vector<std::uint32_t> framesTo;
    std::vector<std::uint32_t> snapshotTo;

    void sendStart(std::uint32_t session, const PB::Battle::BattleStartPush& p) override
    {
        starts.push_back(p);
        startTo.push_back(session);
    }
    void sendFrames(std::uint32_t session, const PB::Battle::BattleFramesPush& p) override
    {
        frames.push_back(p);
        framesTo.push_back(session);
    }
    void sendSnapshot(std::uint32_t session, const PB::Battle::BattleSnapshotPush& p) override
    {
        snapshots.push_back(p);
        snapshotTo.push_back(session);
    }
    void sendEnd(std::uint32_t, const PB::Battle::BattleEndPush& p) override
    {
        ends.push_back(p);
    }
    void sendResult(const PB::BattleInternal::BattleResultReq& r) override
    {
        results.push_back(r);
    }
};

// 累计失败条数。
inline int g_failures = 0;

inline void expect(bool cond, const char* what)
{
    if (cond)
    {
        std::printf("[PASS] %s\n", what);
    }
    else
    {
        std::printf("[FAIL] %s\n", what);
        ++g_failures;
    }
}

inline battle::BattleRoomConfig makeDuelConfig(std::uint32_t battleId)
{
    battle::BattleRoomConfig config;
    config.battleId   = battleId;
    config.roomId     = 30002;
    config.randomSeed = 42;
    config.pvp        = true;

    battle::PlayerSlot challenger;
    challenger.sessionId = 1001;
    challenger.playerId  = 11;
    challenger.name      = "Alice";
    challenger.classId   = 3;  // Fighter
    challenger.slot      = 0;
    challenger.camp      = 2;  // EntityCamp::Our
    challenger.level     = 42;
    config.players.push_back(challenger);

    battle::PlayerSlot target;
    target.sessionId = 1002;
    target.playerId  = 22;
    target.name      = "Bob";
    target.classId   = 5;  // PartnerWind
    target.slot      = 1;
    target.camp      = 8;  // EntityCamp::Enemy
    target.level     = 37;
    config.players.push_back(target);

    return config;
}

}  // namespace battle_test
