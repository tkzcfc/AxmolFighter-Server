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

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <map>
#include <set>
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

// 单向链路的时延模型：基础时延 + 确定性抖动，且保证同一链路内按发送顺序到达（模拟 TCP 的有序交付）。
struct Link
{
    Micros m_latencyUs = 0;
    Micros m_jitterUs  = 0;
    std::uint32_t m_rng = 12345;
    Micros m_lastDeliver = 0;

    Micros deliverAt(Micros now)
    {
        m_rng = m_rng * 1664525u + 1013904223u;
        const Micros jitter = m_jitterUs > 0 ? static_cast<Micros>((m_rng >> 8) % static_cast<std::uint32_t>(m_jitterUs + 1)) : 0;
        const Micros at     = std::max(m_lastDeliver, now + m_latencyUs + jitter);
        m_lastDeliver       = at;
        return at;
    }
};

// 把服务器推送序列化后放上线路，模拟真实编码。每个目标会话一条独立链路。
class WireSender final : public battle::IBattleSender
{
public:
    WireSender(Wire& toClients, const Micros& now, Micros latency, Micros jitter = 0)
        : m_wire(toClients), m_now(now), m_latency(latency), m_jitter(jitter)
    {
    }
    void sendStart(std::uint32_t session, const PB::Battle::BattleStartPush& p) override
    {
        m_wire.push(link(session).deliverAt(m_now), {Message::Kind::Start, session, p.SerializeAsString()});
    }
    void sendFrames(std::uint32_t session, const PB::Battle::BattleFramesPush& p) override
    {
        m_wire.push(link(session).deliverAt(m_now), {Message::Kind::Frames, session, p.SerializeAsString()});
    }
    void sendSnapshot(std::uint32_t session, const PB::Battle::BattleSnapshotPush& p) override
    {
        ++snapshots;
        m_wire.push(link(session).deliverAt(m_now), {Message::Kind::Snapshot, session, p.SerializeAsString()});
    }
    void sendEnd(std::uint32_t, const PB::Battle::BattleEndPush&) override {}
    void sendResult(const PB::BattleInternal::BattleResultReq&) override {}

    // 运行中改变所有下行链路的基础时延。
    void setLatency(Micros latency)
    {
        m_latency = latency;
        for (auto& [session, l] : m_links)
            l.m_latencyUs = latency;
    }

    int snapshots = 0;

private:
    Link& link(std::uint32_t session)
    {
        auto it = m_links.find(session);
        if (it == m_links.end())
        {
            Link l;
            l.m_latencyUs = m_latency;
            l.m_jitterUs  = m_jitter;
            l.m_rng       = 777u + session;
            it = m_links.emplace(session, l).first;
        }
        return it->second;
    }

    Wire& m_wire;
    const Micros& m_now;
    Micros m_latency;
    Micros m_jitter;
    std::map<std::uint32_t, Link> m_links;
};

// 单客户端的性能与回滚度量：区分网络回调（确认输入触发回滚）与逐帧推进两处耗时。
struct ClientPerf
{
    // 网络回调（onFrames）调用次数与耗时（毫秒）。
    std::uint64_t framesCalls = 0;
    double framesMsTotal      = 0.0;
    double framesMsMax        = 0.0;
    // 逐帧推进（update）中实际推进了帧的调用次数与耗时（毫秒）。
    std::uint64_t updateCalls = 0;
    double updateMsTotal      = 0.0;
    double updateMsMax        = 0.0;
    // 单次网络回调内重推帧数的最大值。
    std::uint64_t maxResimInCall = 0;
    // 单次 update 内追帧（推进帧数）的最大值。
    std::uint64_t maxCatchUp = 0;
    // 单个渲染帧内 网络回调 + 推进 的最大总耗时（毫秒）。
    double frameMsMax = 0.0;
};

// 模拟表现层的实体跟踪：统计每个渲染帧可见实体的创建 / 销毁次数与动作切换次数，
// 用来量化回滚造成的表现层返工（节点重建、setMotion 重放）。
struct PresenterProbe
{
    std::map<std::uint32_t, std::string> live;
    std::uint64_t created = 0;
    std::uint64_t removed = 0;
    std::uint64_t motionChanges = 0;

    void observe(mugen::sim::World& world)
    {
        std::map<std::uint32_t, std::string> now;
        world.ecs().each<mugen::sim::EntityGroup, mugen::sim::Transform, mugen::sim::MotionState>(
            [&](mugen::Entity entity, const mugen::sim::EntityGroup&, const mugen::sim::Transform&,
                const mugen::sim::MotionState& motion) { now[entity.value] = motion.motionName + "|" + motion.entryId; });
        for (const auto& [id, motion] : now)
        {
            const auto it = live.find(id);
            if (it == live.end())
                ++created;
            else if (it->second != motion)
                ++motionChanges;
        }
        for (const auto& [id, motion] : live)
            if (!now.contains(id))
                ++removed;
        live = std::move(now);
    }
};

double elapsedMs(std::chrono::steady_clock::time_point begin)
{
    return std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - begin).count();
}

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
        const auto begin           = std::chrono::steady_clock::now();
        const auto resimBefore     = m_sync->session().stats().resimFrames;
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
        mugen::sim::net::ServerInputFeedback feedback;
        feedback.m_echoClientTimeMs = push.echo_client_time_ms();
        feedback.m_slackMin         = push.input_slack_min();
        feedback.m_slackFrame       = push.input_slack_frame();
        feedback.m_slackSamples     = push.input_slack_samples();
        feedback.m_lateInputs       = push.late_inputs();
        m_sync->onServerFeedback(feedback, localNowMs(now));
        leadMax = std::max(leadMax, m_sync->inputLeadFrames());
        m_sync->flushRollback();
        for (const auto& hash : push.hashes())
        {
            if (m_sync->session().hashAt(hash.frame()) != 0)
                ++hashesCompared;
            if (m_sync->checkServerHash(hash.frame(), hash.hash()))
                ++hashMismatches;
        }
        const double ms = elapsedMs(begin);
        ++perf.framesCalls;
        perf.framesMsTotal += ms;
        perf.framesMsMax = std::max(perf.framesMsMax, ms);
        perf.maxResimInCall = std::max(perf.maxResimInCall, m_sync->session().stats().resimFrames - resimBefore);
        m_frameMsSoFar += ms;
    }

    // 对应 NetBattleMode::onUpdate 的推进部分；返回需要上送的 BattleInputPush。
    bool update(Micros now, std::string& outPush)
    {
        if (!m_started || m_awaitSnapshot)
            return false;
        const auto begin           = std::chrono::steady_clock::now();
        const std::uint64_t target = m_sync->targetFrame(localNowMs(now));
        std::uint32_t catchUp      = 0;
        bool advanced              = false;
        while (m_sync->session().currentFrame() < target && catchUp < kMaxCatchUpFrames &&
               m_sync->session().canAdvance())
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
        const double ms = elapsedMs(begin);
        probe.observe(m_sync->session().world());
        m_frameMsSoFar += ms;
        perf.frameMsMax = std::max(perf.frameMsMax, m_frameMsSoFar);
        m_frameMsSoFar  = 0.0;
        if (!advanced)
            return false;
        ++perf.updateCalls;
        perf.updateMsTotal += ms;
        perf.updateMsMax = std::max(perf.updateMsMax, ms);
        perf.maxCatchUp  = std::max<std::uint64_t>(perf.maxCatchUp, catchUp);

        PB::Battle::BattleInputPush push;
        push.set_battle_id(m_battleId);
        push.set_ack_frame(static_cast<std::uint32_t>(m_sync->session().confirmedFrame()));
        push.set_client_time_ms(localNowMs(now));
        std::vector<std::pair<std::uint64_t, mugen::sim::PlayerInput>> inputs;
        m_sync->collectPendingInputs(inputs);
        for (auto& [frame, input] : inputs)
        {
            auto* entry = push.add_inputs();
            entry->set_frame(static_cast<std::uint32_t>(frame));
            std::string encoded = mugen::sim::net::encodePlayerInput(input);
            // 每个帧号只能有一份内容：冗余重发必须与首次发送逐字节一致。
            const auto [it, inserted] = sentInputs.emplace(frame, encoded);
            if (!inserted && it->second != encoded)
                ++resendConflicts;
            entry->set_input(std::move(encoded));
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

    ClientPerf perf;
    PresenterProbe probe;
    int hashMismatches = 0;
    // 与服务器哈希实际对账成功的次数（排除已被覆盖、无法比较的帧）。
    int hashesCompared = 0;
    // 本客户端累计推进的逻辑帧数，以及收到快照时已推进的帧数。
    std::uint64_t ticks               = 0;
    std::uint64_t ticksBeforeSnapshot = 0;
    std::uint64_t frameAfterSnapshot  = 0;
    // 观察到的最大自适应输入领先量。
    std::uint32_t leadMax = 0;
    // 已发送输入帧的内容，以及同一帧号被发出不同内容的次数（必须为 0）。
    std::map<std::uint64_t, std::string> sentInputs;
    int resendConflicts = 0;

private:
    static constexpr std::uint32_t kMaxCatchUpFrames = 4;
    // 任意的本地时钟基准，避免偏移样本落在 0 附近。
    static constexpr std::uint64_t kClockBaseMs = 5000000;

    // 自上次 update 以来累计的网络回调耗时，update 时并入同一渲染帧的总耗时。
    double m_frameMsSoFar = 0.0;
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


// 打印单个客户端的回滚与耗时度量；seconds 为战斗持续的模拟秒数。
void printClientPerf(const char* name, TestClient& client, double seconds)
{
    const auto& stats = client.sync().session().stats();
    const auto& perf  = client.perf;
    const double perRollback = stats.rollbacks > 0 ? static_cast<double>(stats.resimFrames) / stats.rollbacks : 0.0;
    std::printf(
        "  %s: rollbacks=%llu (%.1f/s) mispredict=%llu (local=%llu) resim/rollback=%.1f maxDepth=%llu maxResimInCall=%llu "
        "stalls=%llu maxCatchUp=%llu\n",
        name,
        static_cast<unsigned long long>(stats.rollbacks),
        static_cast<double>(stats.rollbacks) / seconds,
        static_cast<unsigned long long>(stats.mispredictions),
        static_cast<unsigned long long>(stats.localMispredictions),
        perRollback,
        static_cast<unsigned long long>(stats.maxDepth),
        static_cast<unsigned long long>(perf.maxResimInCall),
        static_cast<unsigned long long>(stats.stalls),
        static_cast<unsigned long long>(perf.maxCatchUp));
    std::printf(
        "  %s: update avg=%.3fms max=%.3fms (n=%llu) | frames-push avg=%.3fms max=%.3fms (n=%llu) | worst render "
        "frame=%.3fms\n",
        name,
        perf.updateCalls > 0 ? perf.updateMsTotal / perf.updateCalls : 0.0,
        perf.updateMsMax,
        static_cast<unsigned long long>(perf.updateCalls),
        perf.framesCalls > 0 ? perf.framesMsTotal / perf.framesCalls : 0.0,
        perf.framesMsMax,
        static_cast<unsigned long long>(perf.framesCalls),
        perf.frameMsMax);
}

// 普通攻击连点场景：槽位 0 先朝对手移动进入攻击距离，随后以固定节奏连点 CTR_ATKA；槽位 1 保持空闲。
// 因此客户端 0 的错误预测只可能来自本地攻击被服务器改写，客户端 1 的错误预测来自对端攻击。
void runAttackSpamScenario(const char* label, Micros latencyUs, std::int64_t clockDomainOffsetMs,
                           bool expectLocalAccepted = false, std::int32_t attackerClass = 1, std::int32_t defenderClass = 2)
{
    std::printf("--- scenario: %s (latency %lld us)\n", label, static_cast<long long>(latencyUs));

    Micros now = 0;
    Wire toClients;
    Wire toServer;
    WireSender sender(toClients, now, latencyUs);

    auto config = makeDuelConfig(300);
    config.players[0].classId = attackerClass;
    config.players[1].classId = defenderClass;
    auto room = battle::BattleRoom::create(sender, config);
    expect(room != nullptr, "room created for attack spam scenario");
    if (!room)
        return;

    std::vector<std::unique_ptr<TestClient>> clients;
    clients.push_back(std::make_unique<TestClient>(1001, clockDomainOffsetMs));
    clients.push_back(std::make_unique<TestClient>(1002, clockDomainOffsetMs));
    room->markReady(1001);
    room->markReady(1002);

    constexpr Micros kServerTickUs = 33333;
    constexpr Micros kClientTickUs = 16667;
    constexpr Micros kApproachAt   = 1000000;
    constexpr Micros kApproachEnd  = 6000000;
    constexpr Micros kSpamBegin    = 6300000;
    constexpr Micros kSpamPeriod   = 300000;  // 约每 0.3s 一次轻点。
    constexpr Micros kSpamHold     = 50000;
    constexpr Micros kEndAt        = 14000000;

    Micros nextServerTick = 0;
    Micros nextClientTick = 0;
    bool approaching      = false;
    bool approachDone     = false;
    std::set<std::string> seenMotions;
    PresenterProbe serverProbe;
    bool keyDown          = false;
    Micros lastPress      = 0;
    mugen::sim::ControlKey approachKey = mugen::sim::ControlKey::Right;

    for (now = 0; now <= kEndAt; now += 1000)
    {
        for (auto& message : toClients.popDue(now))
        {
            auto& client = *clients[message.session == 1001 ? 0 : 1];
            if (message.kind == Message::Kind::Start)
            {
                PB::Battle::BattleStartPush push;
                push.ParseFromString(message.bytes);
                client.onStart(push, now);
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
        if (now >= nextClientTick)
        {
            for (auto& client : clients)
            {
                std::string bytes;
                if (client->update(now, bytes))
                    toServer.push(now + latencyUs, {Message::Kind::Input, client->session(), std::move(bytes)});
            }
            serverProbe.observe(const_cast<mugen::sim::World&>(room->authority().world()));
            nextClientTick += kClientTickUs;
        }

        if (!clients[0]->started())
            continue;
        if (now % 1000 == 0)
        {
            const auto& w     = room->authority().world();
            const auto& slots = w.ecs().singleton<mugen::sim::PlayerSlots>().entities;
            seenMotions.insert(w.ecs().get<mugen::sim::MotionState>(slots.at(0)).motionName + "/" +
                               w.ecs().get<mugen::sim::MotionState>(slots.at(1)).motionName);
        }
        if (!approaching && now >= kApproachAt)
        {
            approaching = true;
            const auto self   = heroTransform(clients[0]->sync().session().world(), 0);
            const auto target = heroTransform(clients[0]->sync().session().world(), 1);
            approachKey = target.position.x > self.position.x ? mugen::sim::ControlKey::Right : mugen::sim::ControlKey::Left;
            clients[0]->setKey(approachKey, true);
        }
        // 进入攻击距离（约 120 单位）后松开方向键开始连点，保证攻击能够命中。
        if (approaching && !approachDone)
        {
            const auto self   = heroTransform(clients[0]->sync().session().world(), 0);
            const auto target = heroTransform(clients[0]->sync().session().world(), 1);
            const auto gap    = self.position.x > target.position.x ? self.position.x - target.position.x
                                                                    : target.position.x - self.position.x;
            if (gap < mugen::Fixed::fromInt(120) || now >= kApproachEnd)
            {
                std::printf("  approach done at t=%lldms gap=%.1f hero0.x=%.1f\n", static_cast<long long>(now / 1000),
                            gap.toFloatForRender(), self.position.x.toFloatForRender());
                approachDone = true;
                clients[0]->setKey(approachKey, false);
            }
        }
        if (now >= kSpamBegin)
        {
            if (!keyDown && now - lastPress >= kSpamPeriod)
            {
                keyDown   = true;
                lastPress = now;
                clients[0]->setKey(mugen::sim::ControlKey::AttackA, true);
            }
            else if (keyDown && now - lastPress >= kSpamHold)
            {
                keyDown = false;
                clients[0]->setKey(mugen::sim::ControlKey::AttackA, false);
            }
        }
    }

    std::printf("  distinct (attacker/defender) motion pairs seen on server: %zu\n", seenMotions.size());
    const double seconds = static_cast<double>(kEndAt - kSpamBegin) / 1.0e6;
    std::printf("  spam window %.1fs, server frame=%llu\n", seconds,
                static_cast<unsigned long long>(room->authority().currentFrame()));
    {
        // 确认攻击确实命中：统计客户端 1 看到的受击数字事件与世界内实体数。
        std::size_t hurt = 0;
        const auto& events = clients[1]->sync().session().eventsToPresent();
        for (const auto& event : events)
            if (std::holds_alternative<mugen::sim::HurtNumber>(event.payload))
                ++hurt;
        std::size_t entities = 0;
        clients[1]->sync().session().world().ecs().each<mugen::sim::Transform>(
            [&](mugen::Entity, const mugen::sim::Transform&) { ++entities; });
        const auto a = heroTransform(clients[1]->sync().session().world(), 0);
        const auto b = heroTransform(clients[1]->sync().session().world(), 1);
        {
            const auto& slots = clients[1]->sync().session().world().ecs().singleton<mugen::sim::PlayerSlots>().entities;
            const auto& world = clients[1]->sync().session().world();
            std::printf("  hero1 hp=%.1f/%.1f hero0 motion=%s\n",
                        world.ecs().get<mugen::sim::Vitals>(slots.at(1)).hp.toFloatForRender(),
                        world.ecs().get<mugen::sim::Vitals>(slots.at(1)).maxHp.toFloatForRender(),
                        world.ecs().get<mugen::sim::MotionState>(slots.at(0)).motionName.c_str());
        }
        std::printf("  hero0 z=%.1f toward=%d | hero1 z=%.1f toward=%d\n", a.position.z.toFloatForRender(), a.towardX,
                    b.position.z.toFloatForRender(), b.towardX);
        std::printf("  client1 world: events=%zu hurtNumbers=%zu entities=%zu hero0.x=%.1f hero1.x=%.1f\n",
                    events.size(), hurt, entities, a.position.x.toFloatForRender(), b.position.x.toFloatForRender());
    }
    {
        // 单次操作成本探针：在战斗进行中的世界上测量克隆 / 哈希 / 序列化 / 单帧推进耗时。
        const mugen::sim::World& live = clients[1]->sync().session().world();
        constexpr int kIters          = 500;
        auto begin                    = std::chrono::steady_clock::now();
        for (int i = 0; i < kIters; ++i)
        {
            mugen::sim::World copy = live;
            if (copy.ecs().singleton<mugen::sim::WorldClock>().frame == ~0ull)
                std::printf("unreachable\n");
        }
        const double cloneUs = elapsedMs(begin) * 1000.0 / kIters;
        begin                = std::chrono::steady_clock::now();
        std::uint64_t sink   = 0;
        for (int i = 0; i < kIters; ++i)
            sink ^= live.hash();
        const double hashUs = elapsedMs(begin) * 1000.0 / kIters;
        begin               = std::chrono::steady_clock::now();
        for (int i = 0; i < kIters; ++i)
        {
            mugen::ByteBuffer buffer;
            live.serialize(buffer);
            sink ^= buffer.len();
        }
        const double serializeUs = elapsedMs(begin) * 1000.0 / kIters;
        mugen::sim::World scratch = live;
        begin                     = std::chrono::steady_clock::now();
        for (int i = 0; i < kIters; ++i)
        {
            mugen::sim::FrameInput input;
            input.frame = scratch.ecs().singleton<mugen::sim::WorldClock>().frame + 1;
            input.players.resize(2);
            input.players[0].slot = 0;
            input.players[1].slot = 1;
            scratch.tick(input);
        }
        const double tickUs = elapsedMs(begin) * 1000.0 / kIters;
        std::printf("  op cost (us): clone=%.1f hash=%.1f serialize=%.1f tick=%.1f (sink %llu)\n", cloneUs, hashUs,
                    serializeUs, tickUs, static_cast<unsigned long long>(sink & 1));
    }
    std::printf("  presenter churn (created/removed/motionChanges): server-ref=%llu/%llu/%llu client0=%llu/%llu/%llu "
                "client1=%llu/%llu/%llu\n",
                static_cast<unsigned long long>(serverProbe.created), static_cast<unsigned long long>(serverProbe.removed),
                static_cast<unsigned long long>(serverProbe.motionChanges),
                static_cast<unsigned long long>(clients[0]->probe.created),
                static_cast<unsigned long long>(clients[0]->probe.removed),
                static_cast<unsigned long long>(clients[0]->probe.motionChanges),
                static_cast<unsigned long long>(clients[1]->probe.created),
                static_cast<unsigned long long>(clients[1]->probe.removed),
                static_cast<unsigned long long>(clients[1]->probe.motionChanges));
    printClientPerf("client0(local spammer)", *clients[0], static_cast<double>(kEndAt) / 1.0e6);
    printClientPerf("client1(remote observer)", *clients[1], static_cast<double>(kEndAt) / 1.0e6);

    expect(clients[0]->hashMismatches == 0 && clients[1]->hashMismatches == 0, "attack spam: no hash mismatch");
    expect(sender.snapshots == 0, "attack spam: no resync snapshot requested");
    if (expectLocalAccepted)
    {
        const auto& local  = clients[0]->sync().session().stats();
        const auto& remote = clients[1]->sync().session().stats();
        // 单向时延在输入预留范围内时，本地攻击不会被服务器改写，因此不产生本地错误预测与回滚。
        expect(local.localMispredictions == 0 && local.rollbacks == 0, "attack spam: local attacks never trigger rollbacks");
        // 对端攻击是边缘输入，空输入预测必然错误：每次点击一次回滚，深度约为输入延迟加单向时延。
        expect(remote.remoteMispredictions > 10, "attack spam: remote attacks mispredict as expected");
        expect(remote.resimFrames <= remote.rollbacks * 8, "attack spam: each rollback resimulates at most 8 frames");
        // 回滚不应让表现层重建实体：可见实体的创建 / 销毁次数与权威世界基本一致。
        expect(clients[1]->probe.created <= serverProbe.created + 3 && clients[1]->probe.removed <= serverProbe.removed + 3,
               "attack spam: rollbacks do not churn presenter entities");
    }
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

// 自适应输入领先量场景：两名玩家持续做方向切换与攻击连点（命令几乎每隔几十毫秒就出现一次），
// 链路带抖动；kLatencySwitchAt 时双向基础时延切换为 latencyAfterUs（<0 表示不切换）。
// 收敛（或切换后稳定）之后，服务器不得再记录任何迟到输入，客户端本地输入也不得被服务器改写。
void runAdaptiveLeadScenario(const char* label, Micros latencyUs, Micros jitterUs, Micros latencyAfterUs = -1)
{
    std::printf("--- scenario: %s (one-way %lld us, jitter %lld us, after-switch %lld us)\n",
                label,
                static_cast<long long>(latencyUs),
                static_cast<long long>(jitterUs),
                static_cast<long long>(latencyAfterUs));

    Micros now = 0;
    Wire toClients;
    Wire toServer;
    WireSender sender(toClients, now, latencyUs, jitterUs);
    std::map<std::uint32_t, Link> upLinks;
    for (std::uint32_t session : {1001u, 1002u})
    {
        Link l;
        l.m_latencyUs = latencyUs;
        l.m_jitterUs  = jitterUs;
        l.m_rng       = 9001u + session;
        upLinks.emplace(session, l);
    }

    auto room = battle::BattleRoom::create(sender, makeDuelConfig(400));
    expect(room != nullptr, "room created for adaptive lead scenario");
    if (!room)
        return;

    std::vector<std::unique_ptr<TestClient>> clients;
    clients.push_back(std::make_unique<TestClient>(1001, 0));
    clients.push_back(std::make_unique<TestClient>(1002, 55555));
    room->markReady(1001);
    room->markReady(1002);

    constexpr Micros kServerTickUs   = 33333;
    constexpr Micros kClientTickUs   = 16667;
    constexpr Micros kSwitchAt       = 8000000;
    constexpr Micros kSettleAfter    = 4500000;  // 收敛 / 切换后留给领先量调整的时间。
    constexpr Micros kEndAt          = 16000000;
    const Micros kFirstCheckAt       = kSettleAfter;
    const Micros kSecondCheckAt      = latencyAfterUs >= 0 ? kSwitchAt + kSettleAfter : kEndAt;

    Micros nextServerTick = 0;
    Micros nextClientTick = 0;
    bool switched         = false;
    // 稳定期起点的累计计数快照。
    struct Mark
    {
        bool taken = false;
        std::uint32_t late[2]  = {0, 0};
        std::uint32_t carried[2] = {0, 0};
        std::uint64_t localMispredict[2] = {0, 0};
        std::uint32_t lead[2] = {0, 0};
    } mark;
    std::uint32_t leadAtEnd[2] = {0, 0};

    auto takeMark = [&]() {
        mark.taken = true;
        for (std::size_t i = 0; i < 2; ++i)
        {
            const auto& st      = room->authority().inputStats(static_cast<std::uint32_t>(i));
            mark.late[i]        = st.m_lateInputs;
            mark.carried[i]     = st.m_carriedInputs;
            mark.localMispredict[i] = clients[i]->sync().session().stats().localMispredictions;
            mark.lead[i]        = clients[i]->sync().inputLeadFrames();
        }
    };

    for (now = 0; now <= kEndAt; now += 1000)
    {
        for (auto& message : toClients.popDue(now))
        {
            auto& client = *clients[message.session == 1001 ? 0 : 1];
            if (message.kind == Message::Kind::Start)
            {
                PB::Battle::BattleStartPush push;
                push.ParseFromString(message.bytes);
                client.onStart(push, now);
            }
            else if (message.kind == Message::Kind::Frames)
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
        if (now >= nextClientTick)
        {
            for (auto& client : clients)
            {
                std::string bytes;
                if (client->update(now, bytes))
                    toServer.push(upLinks[client->session()].deliverAt(now),
                                  {Message::Kind::Input, client->session(), std::move(bytes)});
            }
            nextClientTick += kClientTickUs;
        }

        if (!clients[0]->started() || !clients[1]->started())
            continue;

        // 方向每 600ms 在左右之间切换，攻击键每 450ms 轻点一次（按住 60ms），两名玩家相位错开。
        for (std::size_t i = 0; i < 2; ++i)
        {
            const Micros t     = now + static_cast<Micros>(i) * 130000;
            const bool right   = (t / 600000) % 2 == 0;
            clients[i]->setKey(mugen::sim::ControlKey::Right, right);
            clients[i]->setKey(mugen::sim::ControlKey::Left, !right);
            clients[i]->setKey(mugen::sim::ControlKey::AttackA, (t % 450000) < 60000);
        }

        if (latencyAfterUs >= 0 && !switched && now >= kSwitchAt)
        {
            switched = true;
            sender.setLatency(latencyAfterUs);
            for (auto& [session, l] : upLinks)
                l.m_latencyUs = latencyAfterUs;
        }
        // 第一个检查点：快照累计计数；到终点或切换后稳定点再比较。
        if (!mark.taken && now >= (latencyAfterUs >= 0 ? kSwitchAt + kSettleAfter : kFirstCheckAt))
            takeMark();
        if (now == kEndAt - 1000)
            for (std::size_t i = 0; i < 2; ++i)
                leadAtEnd[i] = clients[i]->sync().inputLeadFrames();
    }
    (void)kSecondCheckAt;

    const std::int64_t oneWayMs     = (switched ? latencyAfterUs : latencyUs) / 1000;
    const std::int64_t peakOneWayMs = std::max(latencyUs, latencyAfterUs) / 1000;
    for (std::size_t i = 0; i < 2; ++i)
    {
        const auto& st    = room->authority().inputStats(static_cast<std::uint32_t>(i));
        const auto& sync  = clients[i]->sync();
        const std::uint64_t local = sync.session().stats().localMispredictions;
        std::printf("  slot%zu: lead=%u (max %u, at-mark %u) rtt=%lldms jitter=%lldms | server late=%u (carried %u dropped %u) "
                    "| late after mark=%u carried after mark=%u | local mispredict total=%llu after mark=%llu | stalls=%llu\n",
                    i,
                    sync.inputLeadFrames(),
                    clients[i]->leadMax,
                    mark.lead[i],
                    static_cast<long long>(sync.rttMs()),
                    static_cast<long long>(sync.rttJitterMs()),
                    st.m_lateInputs,
                    st.m_carriedInputs,
                    st.m_droppedInputs,
                    st.m_lateInputs - mark.late[i],
                    st.m_carriedInputs - mark.carried[i],
                    static_cast<unsigned long long>(local),
                    static_cast<unsigned long long>(local - mark.localMispredict[i]),
                    static_cast<unsigned long long>(sync.session().stats().stalls));

        expect(st.m_lateInputs == mark.late[i], "adaptive lead: no late input after convergence");
        expect(local == mark.localMispredict[i], "adaptive lead: server never rewrites a local input after convergence");
        expect(clients[i]->resendConflicts == 0, "adaptive lead: every input frame is sent with exactly one content");
        // 领先量足以覆盖 RTT，但不过度：RTT（含抖动）帧数 + 少量余量以内。
        const std::int64_t rttFrames = (2 * peakOneWayMs + 2 * (jitterUs / 1000) + 32) / 33;
        expect(static_cast<std::int64_t>(sync.inputLeadFrames()) >= (2 * oneWayMs + 32) / 33,
               "adaptive lead: lead covers the round trip");
        expect(static_cast<std::int64_t>(clients[i]->leadMax) <= rttFrames + 10, "adaptive lead: lead does not run away");
        expect(clients[i]->hashMismatches == 0, "adaptive lead: no hash mismatch");
        // 链路变好后领先量要降下来（缩小是缓慢的，但稳定期结束时应已明显低于峰值）。
        if (latencyAfterUs >= 0 && latencyAfterUs < latencyUs)
            expect(sync.inputLeadFrames() + 6 <= clients[i]->leadMax, "adaptive lead: lead shrinks after the link recovers");
    }
    expect(sender.snapshots == 0, "adaptive lead: no resync snapshot requested");
    expect(clients[0]->hashesCompared >= 10 && clients[1]->hashesCompared >= 10, "adaptive lead: hashes compared");
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
    runAttackSpamScenario("attack-spam-20ms", 20000, 0, true);
    runAttackSpamScenario("attack-spam-60ms", 60000, 123456, true);
    runAttackSpamScenario("attack-spam-100ms", 100000, -4000000);
    runAttackSpamScenario("attack-spam-80ms", 80000, 0);
    runAttackSpamScenario("attack-spam-120ms", 120000, 0);
    runAdaptiveLeadScenario("adaptive-lead-20ms", 20000, 6000);
    runAdaptiveLeadScenario("adaptive-lead-60ms", 60000, 15000);
    runAdaptiveLeadScenario("adaptive-lead-120ms", 120000, 30000);
    runAdaptiveLeadScenario("adaptive-lead-200ms", 200000, 40000);
    runAdaptiveLeadScenario("adaptive-lead-spike-20ms-to-150ms", 20000, 6000, 150000);
    runAdaptiveLeadScenario("adaptive-lead-relax-200ms-to-20ms", 200000, 20000, 20000);
    runReconnectScenario("reconnect-localhost", 1000, 0);
    runReconnectScenario("reconnect-wan-60ms-offset-domain", 60000, 123456);
}
