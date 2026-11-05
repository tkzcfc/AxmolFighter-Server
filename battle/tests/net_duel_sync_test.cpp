// 联网决斗端到端无头测试：真实 BattleRoom（假发送端）+ 两个 NetBattleSync/RollbackSession 客户端，
// 以虚拟墙钟模拟网络时延、不同时钟域与真实的 protobuf 编解码，验证本地移动输入
// 同时在客户端世界与服务器权威世界生效，并验证两名决斗者朝向相对、职业各异。

#include "battle/BattleRoom.h"
#include "test_support.h"

#include "mugen/core/serialize/ByteBuffer.h"
#include "mugen/sim/BattleBootstrap.h"
#include "mugen/sim/input/ControlManagerCore.h"
#include "mugen/sim/net/NetBattleSync.h"
#include "mugen/sim/net/NetCodec.h"

#include <cmath>
#include <cstdint>
#include <cstdio>
#include <map>
#include <memory>
#include <string>
#include <utility>
#include <vector>

namespace
{
using namespace battle_test;

// 虚拟时间（微秒）。
using Micros = std::int64_t;

// 一条在途消息：目标会话（服务器 -> 客户端）或来源会话（客户端 -> 服务器）。
struct Message
{
    enum class Kind
    {
        Start,
        Frames,
        Snapshot,
        Input,
        Ready,
    };
    Kind kind = Kind::Frames;
    std::uint32_t session = 0;
    std::string bytes;
};

// 带时延的有序消息队列。
class Wire
{
public:
    void push(Micros deliverAt, Message message) { m_queue.emplace(deliverAt, std::move(message)); }
    // 取出所有到期消息。
    std::vector<Message> popDue(Micros now)
    {
        std::vector<Message> due;
        while (!m_queue.empty() && m_queue.begin()->first <= now)
        {
            due.push_back(std::move(m_queue.begin()->second));
            m_queue.erase(m_queue.begin());
        }
        return due;
    }

private:
    std::multimap<Micros, Message> m_queue;
};

// 把服务器推送序列化后放上线路，模拟真实编码。
class WireSender final : public battle::IBattleSender
{
public:
    WireSender(Wire& toClients, const Micros& now, Micros latency)
        : m_wire(toClients), m_now(now), m_latency(latency)
    {
    }
    void sendStart(std::uint32_t session, const PB::Battle::BattleStartPush& p) override
    {
        m_wire.push(m_now + m_latency, {Message::Kind::Start, session, p.SerializeAsString()});
    }
    void sendFrames(std::uint32_t session, const PB::Battle::BattleFramesPush& p) override
    {
        m_wire.push(m_now + m_latency, {Message::Kind::Frames, session, p.SerializeAsString()});
    }
    void sendSnapshot(std::uint32_t session, const PB::Battle::BattleSnapshotPush& p) override
    {
        ++snapshots;
        m_wire.push(m_now + m_latency, {Message::Kind::Snapshot, session, p.SerializeAsString()});
    }
    void sendEnd(std::uint32_t, const PB::Battle::BattleEndPush&) override {}
    void sendResult(const PB::BattleInternal::BattleResultReq&) override {}

    int snapshots = 0;

private:
    Wire& m_wire;
    const Micros& m_now;
    Micros m_latency;
};

// 复刻 NetBattleMode 的网络与预测逻辑（不含引擎表现层）。
class TestClient
{
public:
    // 断线重连的新客户端收到 resume=true 的开始参数：它只引导帧 0 的世界，
    // 必须等快照复位后才允许推进，绝不能从帧 0 重新模拟到当前帧。
    TestClient(std::uint32_t session, std::int64_t clockDomainOffsetMs)
        : m_session(session), m_domainOffsetMs(clockDomainOffsetMs)
    {
    }

    // 本地单调时钟（毫秒），与服务器时钟域相差固定偏移。
    std::uint64_t localNowMs(Micros now) const
    {
        return static_cast<std::uint64_t>(static_cast<std::int64_t>(kClockBaseMs) + m_domainOffsetMs + now / 1000);
    }

    void onStart(const PB::Battle::BattleStartPush& push, Micros now)
    {
        mugen::sim::WorldSetup setup;
        const bool decoded = mugen::sim::net::decodeWorldSetup(push.setup(), setup);
        expect(decoded, "client decodes WorldSetup");
        m_battleId  = push.battle_id();
        m_awaitSnapshot = push.resume();
        m_localSlot = push.local_slot();
        m_frameMs   = push.frame_ms();

        mugen::sim::World world;
        const auto heroes = mugen::sim::bootstrapBattle(world, setup);
        expect(!heroes.empty(), "client bootstraps battle world");

        mugen::sim::net::NetBattleSyncConfig config;
        config.m_inputDelayFrames = push.input_delay_frames();
        config.m_frameMs          = m_frameMs;
        m_sync = std::make_unique<mugen::sim::net::NetBattleSync>(m_localSlot, push.slot_count(), config);
        m_sync->start(std::move(world), push.start_server_time_ms(), localNowMs(now));
        m_started = true;
    }

    // 对应 NetBattleMode::onSnapshot：复位世界并以快照服务器时间重置时钟。
    void onSnapshot(const PB::Battle::BattleSnapshotPush& push, Micros now)
    {
        if (!m_started)
            return;
        mugen::ByteBuffer buffer;
        for (const char byte : push.world())
            buffer.writeByte(static_cast<std::uint8_t>(byte));
        buffer.writeFinish();
        const bool ok = m_sync->resync(push.frame(), buffer);
        expect(ok, "client applies snapshot");
        m_sync->resetClock(push.server_time_ms(), localNowMs(now));
        ticksBeforeSnapshot = ticks;
        frameAfterSnapshot  = m_sync->session().currentFrame();
        m_awaitSnapshot     = false;
    }

    void onFrames(const PB::Battle::BattleFramesPush& push, Micros now)
    {
        if (!m_started)
            return;
        for (const auto& frame : push.frames())
        {
            mugen::sim::FrameInput input;
            if (!mugen::sim::net::decodeFrameInput(frame.input(), input))
                continue;
            for (const auto& player : input.players)
                m_sync->confirmInput(input.frame, player.slot, player);
            for (const auto& command : input.systemCommands)
                m_sync->confirmSystem(input.frame, command);
        }
        m_sync->onServerClock(push.server_frame(), push.server_time_ms(), localNowMs(now));
        m_sync->flushRollback();
        for (const auto& hash : push.hashes())
        {
            if (m_sync->session().hashAt(hash.frame()) != 0)
                ++hashesCompared;
            if (m_sync->checkServerHash(hash.frame(), hash.hash()))
                ++hashMismatches;
        }
    }

    // 对应 NetBattleMode::onUpdate 的推进部分；返回需要上送的 BattleInputPush。
    bool update(Micros now, std::string& outPush)
    {
        if (!m_started || m_awaitSnapshot)
            return false;
        const std::uint64_t target = m_sync->targetFrame(localNowMs(now));
        std::uint32_t catchUp      = 0;
        bool advanced              = false;
        while (m_sync->session().currentFrame() < target && catchUp < kMaxCatchUpFrames)
        {
            std::vector<mugen::sim::PlayerCommand> commands;
            m_control.advance(commands);
            mugen::sim::PlayerInput input;
            input.slot     = m_localSlot;
            input.commands = std::move(commands);
            m_sync->submitLocalInput(m_sync->session().currentFrame() + 1, std::move(input));
            if (!m_sync->session().tick())
                break;
            ++ticks;
            advanced = true;
            ++catchUp;
        }
        if (!advanced)
            return false;

        PB::Battle::BattleInputPush push;
        push.set_battle_id(m_battleId);
        push.set_ack_frame(static_cast<std::uint32_t>(m_sync->session().confirmedFrame()));
        std::vector<std::pair<std::uint64_t, mugen::sim::PlayerInput>> inputs;
        m_sync->collectPendingInputs(inputs);
        for (auto& [frame, input] : inputs)
        {
            auto* entry = push.add_inputs();
            entry->set_frame(static_cast<std::uint32_t>(frame));
            entry->set_input(mugen::sim::net::encodePlayerInput(input));
        }
        std::vector<std::pair<std::uint64_t, std::uint64_t>> hashes;
        m_sync->collectClientHashes(hashes);
        for (const auto& [frame, hash] : hashes)
        {
            auto* entry = push.add_client_hashes();
            entry->set_frame(static_cast<std::uint32_t>(frame));
            entry->set_hash(hash);
        }
        outPush = push.SerializeAsString();
        return true;
    }

    void setKey(mugen::sim::ControlKey key, bool pressed) { m_control.setKey(key, pressed); }

    std::uint32_t session() const { return m_session; }
    mugen::sim::net::NetBattleSync& sync() { return *m_sync; }
    bool started() const { return m_started; }

    int hashMismatches = 0;
    // 与服务器哈希实际对账成功的次数（排除已被覆盖、无法比较的帧）。
    int hashesCompared = 0;
    // 本客户端累计推进的逻辑帧数，以及收到快照时已推进的帧数。
    std::uint64_t ticks               = 0;
    std::uint64_t ticksBeforeSnapshot = 0;
    std::uint64_t frameAfterSnapshot  = 0;

private:
    static constexpr std::uint32_t kMaxCatchUpFrames = 4;
    // 任意的本地时钟基准，避免偏移样本落在 0 附近。
    static constexpr std::uint64_t kClockBaseMs = 5000000;

    std::uint32_t m_session = 0;
    std::int64_t m_domainOffsetMs = 0;
    std::uint32_t m_battleId = 0;
    std::uint32_t m_localSlot = 0;
    std::uint32_t m_frameMs = 33;
    bool m_started = false;
    bool m_awaitSnapshot = false;
    std::unique_ptr<mugen::sim::net::NetBattleSync> m_sync;
    mugen::sim::ControlManagerCore m_control;
};

mugen::sim::Transform heroTransform(const mugen::sim::World& world, std::uint32_t slot)
{
    const auto& slots = world.ecs().singleton<mugen::sim::PlayerSlots>().entities;
    return world.ecs().get<mugen::sim::Transform>(slots.at(slot));
}

// 以给定单向时延与时钟域偏移运行一场决斗：槽位 0 在 pressAt 后持续朝对手方向移动。
void runDuelScenario(const char* label, Micros latencyUs, std::int64_t clockDomainOffsetMs, Micros startStallUs = 0)
{
    std::printf("--- scenario: %s (latency %lld us, clock offset %lld ms, start stall %lld us)\n",
                label,
                static_cast<long long>(latencyUs),
                static_cast<long long>(clockDomainOffsetMs),
                static_cast<long long>(startStallUs));

    Micros now = 0;
    Wire toClients;
    Wire toServer;
    WireSender sender(toClients, now, latencyUs);

    auto room = battle::BattleRoom::create(sender, makeDuelConfig(100));
    expect(room != nullptr, "room created for net scenario");
    if (!room)
        return;

    std::vector<std::unique_ptr<TestClient>> clients;
    clients.push_back(std::make_unique<TestClient>(1001, clockDomainOffsetMs));
    clients.push_back(std::make_unique<TestClient>(1002, clockDomainOffsetMs));
    room->markReady(1001);
    room->markReady(1002);

    constexpr Micros kServerTickUs = 33333;  // 与 battle 服 tick_rate=30 一致。
    constexpr Micros kClientTickUs = 16667;  // 客户端 60 FPS。
    // 客户端收到开始包后的主线程阻塞（地图 / 骨骼同步加载），期间不收包也不推进。
    Micros stallUntil      = 0;
    const Micros kPressAt  = 1000000 + startStallUs;
    const Micros kEndAt    = 4000000 + startStallUs;

    Micros nextServerTick = 0;
    Micros nextClientTick = 0;
    bool pressed          = false;
    mugen::sim::Transform clientBefore{};
    mugen::sim::Transform serverBefore{};
    // 朝对手方向（出生朝向）移动，保证不会撞到地图边界。
    std::int32_t moveSign = 1;

    for (now = 0; now <= kEndAt; now += 1000)
    {
        const bool clientBlocked = now < stallUntil;
        for (auto& message : clientBlocked ? std::vector<Message>{} : toClients.popDue(now))
        {
            auto& client = *clients[message.session == 1001 ? 0 : 1];
            if (message.kind == Message::Kind::Start)
            {
                PB::Battle::BattleStartPush push;
                push.ParseFromString(message.bytes);
                client.onStart(push, now);
                stallUntil = now + startStallUs;
            }
            else
            {
                PB::Battle::BattleFramesPush push;
                push.ParseFromString(message.bytes);
                client.onFrames(push, now);
            }
        }
        for (auto& message : toServer.popDue(now))
        {
            PB::Battle::BattleInputPush push;
            push.ParseFromString(message.bytes);
            room->submitInput(message.session, push);
        }

        if (now >= nextServerTick)
        {
            room->tick(1.0f / 30.0f);
            nextServerTick += kServerTickUs;
        }
        if (now >= nextClientTick && !clientBlocked)
        {
            for (auto& client : clients)
            {
                std::string bytes;
                if (client->update(now, bytes))
                    toServer.push(now + latencyUs, {Message::Kind::Input, client->session(), std::move(bytes)});
            }
            nextClientTick += kClientTickUs;
        }

        if (!pressed && now >= kPressAt && clients[0]->started())
        {
            pressed      = true;
            clientBefore = heroTransform(clients[0]->sync().session().world(), 0);
            serverBefore = heroTransform(room->authority().world(), 0);
            moveSign     = clientBefore.towardX;
            clients[0]->setKey(moveSign > 0 ? mugen::sim::ControlKey::Right : mugen::sim::ControlKey::Left, true);
        }
    }

    expect(clients[0]->started() && clients[1]->started(), "both clients received start");
    if (!(clients[0]->started() && clients[1]->started()))
        return;

    const auto& stats = clients[0]->sync().session().stats();
    std::printf("client0 frame=%llu confirmed=%llu server=%llu rollbacks=%llu mispredict=%llu stalls=%llu\n",
                static_cast<unsigned long long>(clients[0]->sync().session().currentFrame()),
                static_cast<unsigned long long>(clients[0]->sync().session().confirmedFrame()),
                static_cast<unsigned long long>(room->authority().currentFrame()),
                static_cast<unsigned long long>(stats.rollbacks),
                static_cast<unsigned long long>(stats.mispredictions),
                static_cast<unsigned long long>(stats.stalls));

    const auto clientAfter = heroTransform(clients[0]->sync().session().world(), 0);
    const auto serverAfter = heroTransform(room->authority().world(), 0);
    std::printf("hero0 x: client %.2f -> %.2f, server %.2f -> %.2f\n",
                clientBefore.position.x.toFloatForRender(),
                clientAfter.position.x.toFloatForRender(),
                serverBefore.position.x.toFloatForRender(),
                serverAfter.position.x.toFloatForRender());
    const auto moved = mugen::Fixed::fromInt(20);
    expect((clientAfter.position.x - clientBefore.position.x) * mugen::Fixed::fromInt(moveSign) > moved,
           "local move changes local hero position in client world");
    expect((serverAfter.position.x - serverBefore.position.x) * mugen::Fixed::fromInt(moveSign) > moved,
           "local move changes hero position in server world");

    // 对端客户端也能看到权威结果：槽位 0 的位置朝对手移动。
    const auto peerAfter = heroTransform(clients[1]->sync().session().world(), 0);
    expect((peerAfter.position.x - serverBefore.position.x) * mugen::Fixed::fromInt(moveSign) > moved,
           "remote client sees the moving hero");
    expect(clients[0]->hashMismatches == 0 && clients[1]->hashMismatches == 0, "no client hash mismatch");
    expect(sender.snapshots == 0, "no resync snapshot requested");
}


// 断线重连场景：槽位 1 的客户端在 kDropAt 掉线（战斗继续、角色待机），在 kRejoinAt 由全新客户端
// 用新会话接入（开始参数 + 快照），之后新客户端移动角色。全程不得出现哈希不一致，
// 重连客户端不得从帧 0 重新模拟。
void runReconnectScenario(const char* label, Micros latencyUs, std::int64_t clockDomainOffsetMs)
{
    std::printf("--- scenario: %s (latency %lld us, clock offset %lld ms)\n",
                label,
                static_cast<long long>(latencyUs),
                static_cast<long long>(clockDomainOffsetMs));

    constexpr std::uint32_t kPeerSession   = 1001;
    constexpr std::uint32_t kOldSession    = 1002;
    constexpr std::uint32_t kNewSession    = 2002;
    constexpr std::int64_t kRejoinPlayerId = 22;

    Micros now = 0;
    Wire toClients;
    Wire toServer;
    WireSender sender(toClients, now, latencyUs);

    auto room = battle::BattleRoom::create(sender, makeDuelConfig(200));
    expect(room != nullptr, "room created for reconnect scenario");
    if (!room)
        return;

    std::vector<std::unique_ptr<TestClient>> clients;
    clients.push_back(std::make_unique<TestClient>(kPeerSession, clockDomainOffsetMs));
    clients.push_back(std::make_unique<TestClient>(kOldSession, clockDomainOffsetMs));
    room->markReady(kPeerSession);
    room->markReady(kOldSession);

    constexpr Micros kServerTickUs = 33333;
    constexpr Micros kClientTickUs = 16667;
    constexpr Micros kPeerPressAt  = 1500000;
    constexpr Micros kDropAt       = 2500000;
    // 掉线期间对端继续移动，战斗已推进上百帧，重连后帧号远大于 0。
    constexpr Micros kPeerReleaseAt = 4500000;
    constexpr Micros kRejoinAt      = 7000000;
    constexpr Micros kRejoinPressAt = 9000000;
    constexpr Micros kEndAt         = 12000000;

    Micros nextServerTick = 0;
    Micros nextClientTick = 0;
    bool peerPressed      = false;
    bool peerReleased     = false;
    bool dropped          = false;
    bool rejoined         = false;
    bool rejoinPressed    = false;
    std::uint64_t serverFrameAtRejoin = 0;
    float rejoinHeroBefore            = 0.0f;
    int rejoinMoveSign                = 1;

    auto findClient = [&](std::uint32_t session) -> TestClient* {
        for (auto& c : clients)
            if (c && c->session() == session)
                return c.get();
        return nullptr;
    };

    for (now = 0; now <= kEndAt; now += 1000)
    {
        for (auto& message : toClients.popDue(now))
        {
            TestClient* client = findClient(message.session);
            if (!client)
                continue;  // 掉线客户端：在途消息丢弃。
            if (message.kind == Message::Kind::Start)
            {
                PB::Battle::BattleStartPush push;
                push.ParseFromString(message.bytes);
                client->onStart(push, now);
            }
            else if (message.kind == Message::Kind::Snapshot)
            {
                PB::Battle::BattleSnapshotPush push;
                push.ParseFromString(message.bytes);
                client->onSnapshot(push, now);
            }
            else
            {
                PB::Battle::BattleFramesPush push;
                push.ParseFromString(message.bytes);
                client->onFrames(push, now);
            }
        }
        for (auto& message : toServer.popDue(now))
        {
            if (message.kind == Message::Kind::Ready)
            {
                room->markReady(message.session);
                continue;
            }
            PB::Battle::BattleInputPush push;
            push.ParseFromString(message.bytes);
            room->submitInput(message.session, push);
        }

        if (now >= nextServerTick)
        {
            room->tick(1.0f / 30.0f);
            nextServerTick += kServerTickUs;
        }
        if (now >= nextClientTick)
        {
            for (auto& client : clients)
            {
                if (!client)
                    continue;
                std::string bytes;
                if (client->update(now, bytes))
                    toServer.push(now + latencyUs, {Message::Kind::Input, client->session(), std::move(bytes)});
            }
            nextClientTick += kClientTickUs;
        }

        if (!peerPressed && now >= kPeerPressAt && clients[0]->started())
        {
            peerPressed = true;
            const auto before = heroTransform(clients[0]->sync().session().world(), 0);
            clients[0]->setKey(before.towardX > 0 ? mugen::sim::ControlKey::Right : mugen::sim::ControlKey::Left, true);
        }
        if (peerPressed && !peerReleased && now >= kPeerReleaseAt)
        {
            peerReleased = true;
            clients[0]->setKey(mugen::sim::ControlKey::Right, false);
            clients[0]->setKey(mugen::sim::ControlKey::Left, false);
        }
        if (!dropped && now >= kDropAt)
        {
            dropped = true;
            clients[1].reset();  // 客户端进程消失
            room->setSessionOnline(kOldSession, false);
        }
        if (dropped && !rejoined && now >= kRejoinAt)
        {
            rejoined = true;
            std::uint32_t oldSession = 0;
            const auto result = room->rejoin(kRejoinPlayerId, kNewSession, oldSession);
            expect(result == battle::RejoinResult::Ok && oldSession == kOldSession, "server accepts the rejoin");
            serverFrameAtRejoin = room->authority().currentFrame();
            clients[1] = std::make_unique<TestClient>(kNewSession, clockDomainOffsetMs);
            // 新客户端进入战斗界面后发 BattleReadyReq。
            toServer.push(now + latencyUs, {Message::Kind::Ready, kNewSession, {}});
        }
        if (rejoined && !rejoinPressed && now >= kRejoinPressAt && clients[1] && clients[1]->started())
        {
            rejoinPressed   = true;
            const auto self = heroTransform(room->authority().world(), 1);
            rejoinHeroBefore = self.position.x.toFloatForRender();
            rejoinMoveSign   = self.towardX;
            clients[1]->setKey(rejoinMoveSign > 0 ? mugen::sim::ControlKey::Right : mugen::sim::ControlKey::Left, true);
        }
    }

    expect(rejoined && clients[1] != nullptr && clients[1]->started(), "rejoined client received the start push");
    if (!(rejoined && clients[1] && clients[1]->started()))
        return;
    auto& fresh = *clients[1];
    auto& peer  = *clients[0];

    std::printf("server frame at rejoin=%llu, fresh client frame after snapshot=%llu, ticks before snapshot=%llu\n",
                static_cast<unsigned long long>(serverFrameAtRejoin),
                static_cast<unsigned long long>(fresh.frameAfterSnapshot),
                static_cast<unsigned long long>(fresh.ticksBeforeSnapshot));
    expect(sender.snapshots == 1, "exactly one snapshot sent (rejoin sync)");
    expect(serverFrameAtRejoin > 150, "battle ran on while the slot was offline");
    expect(fresh.frameAfterSnapshot >= serverFrameAtRejoin, "rejoined client loads a snapshot far ahead of frame 0");
    expect(fresh.ticksBeforeSnapshot == 0, "rejoined client never simulates from frame 0");
    expect(fresh.ticks < 700, "rejoined client only simulates frames after the snapshot");

    const std::uint64_t serverFrame = room->authority().currentFrame();
    const std::uint64_t clientFrame = fresh.sync().session().currentFrame();
    const std::uint64_t lead        = clientFrame > serverFrame ? clientFrame - serverFrame : serverFrame - clientFrame;
    std::printf("end: server frame=%llu rejoined client frame=%llu confirmed=%llu\n",
                static_cast<unsigned long long>(serverFrame),
                static_cast<unsigned long long>(clientFrame),
                static_cast<unsigned long long>(fresh.sync().session().confirmedFrame()));
    expect(lead < 20, "rejoined client frame tracks the server frame");
    expect(fresh.sync().session().confirmedFrame() + 12 >= serverFrame, "rejoined client confirms server frames");

    const float heroAfter = heroTransform(room->authority().world(), 1).position.x.toFloatForRender();
    expect((heroAfter - rejoinHeroBefore) * static_cast<float>(rejoinMoveSign) > 20.0f,
           "rejoined player can move the character again");
    const float peerSees = heroTransform(peer.sync().session().world(), 1).position.x.toFloatForRender();
    expect((peerSees - rejoinHeroBefore) * static_cast<float>(rejoinMoveSign) > 20.0f,
           "peer client sees the rejoined player moving");
    const float freshSeesPeer = heroTransform(fresh.sync().session().world(), 0).position.x.toFloatForRender();
    const float serverPeer    = heroTransform(room->authority().world(), 0).position.x.toFloatForRender();
    expect(freshSeesPeer == serverPeer || std::abs(freshSeesPeer - serverPeer) < 80.0f,
           "rejoined client world matches the server for the peer hero");

    expect(peer.hashMismatches == 0 && fresh.hashMismatches == 0, "no hash mismatch across drop and rejoin");
    expect(fresh.hashesCompared >= 3, "rejoined client actually compared several server hashes");
    expect(peer.hashesCompared >= 10, "peer client compared hashes throughout");
    expect(!room->finished() && sender.snapshots == 1, "no forfeit and no extra resync");
}

// 出生检查：两名决斗者面对面，且各自使用所属职业的角色。
void testSpawnFacing()
{
    std::printf("--- scenario: spawn positions and facing\n");
    FakeSender sender;
    auto room = battle::BattleRoom::create(sender, makeDuelConfig(101));
    expect(room != nullptr, "room created for spawn test");
    if (!room)
        return;
    const auto& world = room->authority().world();
    const auto a = heroTransform(world, 0);
    const auto b = heroTransform(world, 1);
    std::printf("slot0 x=%.1f toward=%d camp, slot1 x=%.1f toward=%d\n",
                a.position.x.toFloatForRender(),
                a.towardX,
                b.position.x.toFloatForRender(),
                b.towardX);
    expect(a.towardX != b.towardX, "duelists face opposite directions");
    const bool facingEachOther = (a.position.x < b.position.x && a.towardX == 1 && b.towardX == -1) ||
                                 (b.position.x < a.position.x && b.towardX == 1 && a.towardX == -1);
    expect(facingEachOther, "duelists face each other");

    const auto& slots = world.ecs().singleton<mugen::sim::PlayerSlots>().entities;
    const auto& idA   = world.ecs().get<mugen::sim::RoleIdentity>(slots.at(0));
    const auto& idB   = world.ecs().get<mugen::sim::RoleIdentity>(slots.at(1));
    expect(idA.roleId != idB.roleId, "each slot spawns its own class role");
}

}  // namespace

void runNetDuelTests()
{
    testSpawnFacing();
    runDuelScenario("localhost", 1000, 0);
    runDuelScenario("lan-20ms", 20000, 123456);
    runDuelScenario("wan-60ms-negative-domain", 60000, -4000000);
    runDuelScenario("localhost-start-stall-1500ms", 1000, 0, 1500000);
    runDuelScenario("localhost-start-stall-400ms", 1000, 0, 400000);
    runReconnectScenario("reconnect-localhost", 1000, 0);
    runReconnectScenario("reconnect-wan-60ms-offset-domain", 60000, 123456);
}
