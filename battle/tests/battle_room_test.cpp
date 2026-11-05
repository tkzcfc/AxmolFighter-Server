// 无网络单测：驱动 BattleRoom 状态机，验证就绪、帧广播、投降 / 断线判负与结果上报。
// 通过 FakeSender 注入，不依赖 yasio / gateway。

#include "battle/BattleContent.h"
#include "battle/BattleRoom.h"
#include "battle/BattleRoomManager.h"
#include "test_support.h"
#include "battle/BattleTypes.h"
#include "battle/IBattleSender.h"

#include "mugen/sim/input/ControlManagerCore.h"
#include "mugen/sim/net/NetCodec.h"

#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

namespace
{
using namespace battle_test;

// 构造单帧输入：槽位 0 带一条 CTR_WALK + UpRight 命令。
PB::Battle::BattleInputPush makeInput(std::uint32_t battleId, std::uint32_t frame, bool withCommand)
{
    mugen::sim::PlayerInput input;
    input.slot = 0;
    if (withCommand)
    {
        mugen::sim::PlayerCommand command;
        command.controlEvent = mugen::ControlEvent::CTR_WALK;
        command.quadrant     = mugen::sim::ControlQuadrant::UpRight;
        input.commands.push_back(command);
    }

    PB::Battle::BattleInputPush push;
    push.set_battle_id(battleId);
    auto* f = push.add_inputs();
    f->set_frame(frame);
    f->set_input(mugen::sim::net::encodePlayerInput(input));
    return push;
}

void testReadyFlowAndFrames()
{
    FakeSender sender;
    auto room = battle::BattleRoom::create(sender, makeDuelConfig(1));
    expect(room != nullptr, "room created");

    // 就绪前不应广播。
    room->tick(0.016f);
    expect(sender.starts.empty(), "no start before ready");

    expect(room->markReady(1001), "session 1001 ready");
    room->tick(0.016f);
    expect(sender.starts.empty(), "no start while waiting for second player");

    expect(room->markReady(1002), "session 1002 ready");
    room->tick(0.016f);
    expect(sender.starts.size() == 2, "two start pushes after all ready");
    if (sender.starts.size() == 2)
    {
        expect(sender.starts[0].local_slot() == 0, "challenger local_slot=0");
        expect(sender.starts[1].local_slot() == 1, "target local_slot=1");
        expect(sender.starts[0].slot_count() == 2, "slot_count=2");
        expect(!sender.starts[0].resume() && !sender.starts[1].resume(), "initial start is not a resume");
        expect(sender.starts[0].frame_ms() == battle::kBattleFrameMs, "frame_ms=33");
        expect(sender.starts[0].input_delay_frames() == battle::kInputDelayFrames,
               "input_delay_frames=4");
    }

    // 提交槽位 0 第 1 帧输入后推进 ~100ms（约 3 帧）。
    room->submitInput(1001, makeInput(1, 1, true));
    room->tick(0.1f);
    expect(!sender.frames.empty(), "frames broadcast after ticking");

    bool sawFrame1 = false;
    bool sawWalkCommand = false;
    for (const auto& push : sender.frames)
    {
        for (const auto& confirmed : push.frames())
        {
            mugen::sim::FrameInput frameInput;
            if (!mugen::sim::net::decodeFrameInput(confirmed.input(), frameInput))
                continue;
            if (frameInput.frame == 1)
            {
                sawFrame1 = true;
                for (const auto& player : frameInput.players)
                    if (player.slot == 0 && !player.commands.empty() &&
                        player.commands[0].controlEvent == mugen::ControlEvent::CTR_WALK)
                        sawWalkCommand = true;
            }
        }
    }
    expect(sawFrame1, "confirmed frame 1 broadcast");
    expect(sawWalkCommand, "submitted walk command round-trips into frame 1");
}

void testSurrenderForfeit()
{
    FakeSender sender;
    auto room = battle::BattleRoom::create(sender, makeDuelConfig(2));
    room->markReady(1001);
    room->markReady(1002);
    room->tick(0.016f);  // start

    room->handleLeave(1001);  // challenger (camp 2) surrenders

    expect(sender.ends.size() == 2, "end push broadcast to both sessions");
    expect(sender.results.size() == 1, "one result push to game");
    if (!sender.results.empty())
    {
        expect(sender.results[0].winner_camp() == 8, "target camp 8 wins on surrender");
        expect(sender.results[0].reason() == static_cast<int32_t>(battle::BattleEndReason::Surrender),
               "reason=Surrender");
    }
    if (!sender.ends.empty())
        expect(sender.ends[0].winner_camp() == 8, "end push winner camp 8");
}

void testNoReadyAbort()
{
    FakeSender sender;
    auto room = battle::BattleRoom::create(sender, makeDuelConfig(3));

    room->tick(battle::kReadyTimeoutSeconds + 1.0f);

    expect(sender.starts.empty(), "no start without any ready");
    expect(sender.results.size() == 1, "abort produces a result push");
    if (!sender.results.empty())
        expect(sender.results[0].reason() == static_cast<int32_t>(battle::BattleEndReason::Abort),
               "reason=Abort");
}

void testDisconnectForfeit()
{
    FakeSender sender;
    auto room = battle::BattleRoom::create(sender, makeDuelConfig(4));
    room->markReady(1001);
    room->markReady(1002);
    room->tick(0.016f);  // start

    room->setSessionOnline(1002, false);
    room->tick(battle::kDefaultReconnectGraceSeconds + 1.0f);

    expect(sender.results.size() == 1, "disconnect produces a result push");
    if (!sender.results.empty())
    {
        expect(sender.results[0].winner_camp() == 2, "challenger camp 2 wins on disconnect");
        expect(sender.results[0].reason() == static_cast<int32_t>(battle::BattleEndReason::Disconnect),
               "reason=Disconnect");
    }
}

void testManagerRouting()
{
    FakeSender sender;
    battle::BattleRoomManager manager(10, 20);

    auto* room = manager.createRoom(sender, makeDuelConfig(5));
    expect(room != nullptr, "manager created room");
    expect(manager.findRoom(5) == room, "find by battle id");
    expect(manager.findRoomBySession(1001) == room, "find challenger by session");
    expect(manager.findRoomBySession(1002) == room, "find target by session");

    // 重复创建返回同一房间。
    expect(manager.createRoom(sender, makeDuelConfig(5)) == room, "duplicate create reuses room");

    // 结束并超过宽限后回收。
    room->handleLeave(1001);
    manager.tickAll(0.0f);
    manager.collectGarbage();
    expect(manager.findRoom(5) == room, "room retained within destroy grace");

    manager.tickAll(battle::kEndDestroySeconds + 0.1f);
    manager.collectGarbage();
    expect(manager.findRoom(5) == nullptr, "room destroyed after grace");
    expect(manager.findRoomBySession(1001) == nullptr, "session routing cleared after destroy");
}

// 帮助：按槽位构造一帧输入（经 ControlManagerCore 产生跑动命令）。
PB::Battle::BattleInputPush makeRunInput(const battle::BattleRoom& room, std::uint32_t battleId, std::uint32_t slot,
                                         std::uint32_t frame)
{
    // 朝出生朝向（对手方向）跑，避免撞到地图边界。
    const auto& world = room.authority().world();
    const auto& slots = world.ecs().singleton<mugen::sim::PlayerSlots>().entities;
    const int toward  = world.ecs().get<mugen::sim::Transform>(slots.at(slot)).towardX;
    mugen::sim::ControlManagerCore control;
    control.setKey(toward > 0 ? mugen::sim::ControlKey::Right : mugen::sim::ControlKey::Left, true);
    mugen::sim::PlayerInput input;
    input.slot = slot;
    control.advance(input.commands);

    PB::Battle::BattleInputPush push;
    push.set_battle_id(battleId);
    auto* f = push.add_inputs();
    f->set_frame(frame);
    f->set_input(mugen::sim::net::encodePlayerInput(input));
    return push;
}

float heroX(const battle::BattleRoom& room, std::uint32_t slot)
{
    const auto& world = room.authority().world();
    const auto& slots = world.ecs().singleton<mugen::sim::PlayerSlots>().entities;
    return world.ecs().get<mugen::sim::Transform>(slots.at(slot)).position.x.toFloatForRender();
}

// 以 1/30 秒步长推进房间。
void tickFor(battle::BattleRoom& room, float seconds)
{
    const float step = 1.0f / 30.0f;
    for (float t = 0.0f; t < seconds; t += step)
        room.tick(step);
}

std::size_t countTo(const std::vector<std::uint32_t>& sessions, std::uint32_t session)
{
    std::size_t n = 0;
    for (const auto s : sessions)
        if (s == session)
            ++n;
    return n;
}

void testOfflineKeepsBattleRunningAndStandsStill()
{
    FakeSender sender;
    auto room = battle::BattleRoom::create(sender, makeDuelConfig(10));
    room->markReady(1001);
    room->markReady(1002);
    room->tick(0.016f);  // start

    // 槽位 1 在第 1 帧开始向右跑，跑动命令是边沿触发的：断线后若不松开按键会一直跑下去。
    tickFor(*room, 1.0f);  // 跳过开场阶段
    const float xStart = heroX(*room, 1);
    room->submitInput(1002, makeRunInput(*room, 10, 1, static_cast<std::uint32_t>(room->authority().currentFrame() + 1)));
    tickFor(*room, 0.5f);
    const float xRunning = heroX(*room, 1);
    expect(room->authority().currentFrame() > 10, "battle advances before offline");

    room->setSessionOnline(1002, false);
    const std::uint64_t frameAtOffline = room->authority().currentFrame();
    const std::size_t framesToOfflineBefore = countTo(sender.framesTo, 1002);
    tickFor(*room, 1.0f);  // 先让待机命令生效、角色停稳
    const float xSettled = heroX(*room, 1);
    tickFor(*room, 3.0f);
    const float xLater = heroX(*room, 1);

    expect(room->authority().currentFrame() > frameAtOffline + 60, "battle keeps running while slot is offline");
    expect(sender.results.empty(), "no forfeit within reconnect grace");
    expect(!room->finished(), "room not finished while slot is offline");
    expect(countTo(sender.framesTo, 1002) == framesToOfflineBefore, "no frames sent to the offline session");
    expect(countTo(sender.framesTo, 1001) > framesToOfflineBefore, "online peer keeps receiving frames");
    std::printf("slot1 x: start %.2f running %.2f settled %.2f later %.2f\n", xStart, xRunning, xSettled, xLater);
    expect(xRunning != xStart, "slot actually ran before going offline");
    expect(xLater == xSettled, "offline slot stands still (no stuck key)");
}

void testRejoinWithinGraceRestoresInput()
{
    FakeSender sender;
    auto room = battle::BattleRoom::create(sender, makeDuelConfig(11));
    room->markReady(1001);
    room->markReady(1002);
    room->tick(0.016f);  // start
    tickFor(*room, 0.3f);

    room->setSessionOnline(1002, false);
    tickFor(*room, 20.0f);
    expect(sender.results.empty(), "no forfeit after 20s offline");

    std::uint32_t oldSession = 0;
    const auto result = room->rejoin(22, 2002, oldSession);
    expect(result == battle::RejoinResult::Ok, "rejoin accepted within grace");
    expect(oldSession == 1002, "rejoin reports the replaced session");
    expect(room->hasSession(2002) && !room->hasSession(1002), "slot rebound to the new session");

    // 新会话在发 BattleReadyReq 前不接收帧。
    const std::size_t startsBefore = sender.starts.size();
    tickFor(*room, 0.5f);
    expect(countTo(sender.framesTo, 2002) == 0, "no frames before the rejoined client is ready");
    expect(sender.starts.size() == startsBefore, "no start push before ready");

    expect(room->markReady(2002), "rejoined session ready");
    expect(countTo(sender.startTo, 2002) == 1, "start push re-sent to the new session");
    expect(countTo(sender.snapshotTo, 2002) == 1, "snapshot sent right after the start push");
    if (!sender.snapshots.empty())
    {
        const auto& snap = sender.snapshots.back();
        expect(snap.frame() == room->authority().currentFrame(), "snapshot frame is the current frame");
        expect(!snap.world().empty() && snap.server_time_ms() != 0, "snapshot carries world and server time");
    }
    if (!sender.starts.empty())
    {
        const auto& start = sender.starts.back();
        expect(start.resume(), "re-sent start is flagged resume (snapshot follows)");
        expect(start.local_slot() == 1 && start.players_size() == 2, "re-sent start keeps slot and players");
        expect(start.players(0).name() == "Alice" && start.players(1).name() == "Bob",
               "re-sent start keeps player list");
    }

    // 旧会话的输入被拒绝，新会话的输入生效。
    const std::size_t framesToNewBefore = countTo(sender.framesTo, 2002);
    room->submitInput(1002, makeRunInput(*room, 11, 1, static_cast<std::uint32_t>(room->authority().currentFrame() + 2)));
    room->submitInput(2002, makeRunInput(*room, 11, 1, static_cast<std::uint32_t>(room->authority().currentFrame() + 2)));
    const float xBefore = heroX(*room, 1);
    tickFor(*room, 0.5f);
    expect(countTo(sender.framesTo, 2002) > framesToNewBefore, "frames flow to the new session after sync");
    expect(heroX(*room, 1) != xBefore, "input from the rejoined session drives the slot");

    // 断线计时已清除：超过原宽限也不会判负。
    tickFor(*room, battle::kDefaultReconnectGraceSeconds + 1.0f);
    expect(sender.results.empty(), "no forfeit after rejoin");
    expect(!room->finished(), "room still running after rejoin");
}

void testForfeitAfterConfiguredGrace()
{
    FakeSender sender;
    auto config = makeDuelConfig(12);
    config.reconnectGraceSeconds = 5.0f;
    auto room = battle::BattleRoom::create(sender, config);
    room->markReady(1001);
    room->markReady(1002);
    room->tick(0.016f);  // start

    room->setSessionOnline(1001, false);
    tickFor(*room, 4.5f);
    expect(sender.results.empty(), "no forfeit before configured grace elapses");
    tickFor(*room, 1.0f);
    expect(sender.results.size() == 1, "forfeit once configured grace elapses");
    if (!sender.results.empty())
    {
        expect(sender.results[0].reason() == static_cast<int32_t>(battle::BattleEndReason::Disconnect),
               "reason=Disconnect");
        expect(sender.results[0].winner_camp() == 8, "online opponent (camp 8) wins");
    }
}

void testRejoinRejectedAfterEnd()
{
    FakeSender sender;
    battle::BattleRoomManager manager(10, 20);
    auto* room = manager.createRoom(sender, makeDuelConfig(13));
    room->markReady(1001);
    room->markReady(1002);
    room->tick(0.016f);

    expect(manager.rejoin(999, 22, 3002) == battle::RejoinResult::NoBattle, "rejoin unknown battle");
    expect(manager.rejoin(13, 777, 3002) == battle::RejoinResult::NoPlayer, "rejoin unknown player");
    expect(manager.rejoin(13, 22, 3002) == battle::RejoinResult::Ok, "manager rejoin ok");
    expect(manager.findRoomBySession(3002) == room, "new session routes to the room");
    expect(manager.findRoomBySession(1002) == nullptr, "old session route removed");

    room->handleLeave(1001);
    expect(manager.rejoin(13, 22, 3003) == battle::RejoinResult::Ended, "rejoin after end rejected");
}

void testLateReadySlotGetsStartAndSnapshot()
{
    FakeSender sender;
    auto room = battle::BattleRoom::create(sender, makeDuelConfig(14));
    room->markReady(1001);
    room->tick(battle::kReadyTimeoutSeconds + 1.0f);  // 仅一方就绪：超时后照常开始
    tickFor(*room, 1.0f);
    expect(countTo(sender.startTo, 1001) == 1, "ready slot got the start push");
    expect(countTo(sender.startTo, 1002) == 0, "absent slot is not pushed before it is ready");
    expect(countTo(sender.framesTo, 1002) == 0, "no frames to a slot that has not synced");

    room->markReady(1002);
    expect(countTo(sender.startTo, 1002) == 1 && countTo(sender.snapshotTo, 1002) == 1,
           "late slot gets start + snapshot when it becomes ready");
    tickFor(*room, 0.3f);
    expect(countTo(sender.framesTo, 1002) > 0, "late slot receives frames after syncing");
}

void testStartCarriesAppearance()
{
    FakeSender sender;
    auto config = makeDuelConfig(15);
    config.players[0].appearance.set_class_id(3);
    config.players[0].appearance.set_armament_id(55);
    config.players[0].appearance.add_fashions()->set_config_id(900);
    config.players[1].appearance.set_pet_id(17102);
    auto room = battle::BattleRoom::create(sender, config);
    room->markReady(1001);
    room->markReady(1002);
    room->tick(0.016f);

    expect(!sender.starts.empty(), "start pushed");
    if (sender.starts.empty())
        return;
    const auto& start = sender.starts[0];
    expect(start.players_size() == 2, "players carried");
    if (start.players_size() == 2)
    {
        expect(start.players(0).appearance().armament_id() == 55 &&
                   start.players(0).appearance().fashions_size() == 1 &&
                   start.players(0).appearance().fashions(0).config_id() == 900,
               "player 0 appearance forwarded to start push");
        expect(start.players(1).appearance().pet_id() == 17102, "player 1 appearance forwarded to start push");
    }
    // 外观只是表现层数据：不进入 WorldSetup。
    mugen::sim::WorldSetup setup;
    expect(mugen::sim::net::decodeWorldSetup(start.setup(), setup), "setup decodes");
}

}  // namespace

void runBattleRoomTests()
{
    testReadyFlowAndFrames();
    testSurrenderForfeit();
    testNoReadyAbort();
    testDisconnectForfeit();
    testManagerRouting();
    testOfflineKeepsBattleRunningAndStandsStill();
    testRejoinWithinGraceRestoresInput();
    testForfeitAfterConfiguredGrace();
    testRejoinRejectedAfterEnd();
    testLateReadySlotGetsStartAndSnapshot();
    testStartCarriesAppearance();
}
