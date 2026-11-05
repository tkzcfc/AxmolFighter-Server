#include "battle/BattleRoom.h"

#include "mugen/conf/ReflectedConfig.h"
#include "mugen/conf/TypeConversions.h"
#include "mugen/conf/tables/MapTables.h"
#include "mugen/sim/net/NetCodec.h"

#include <spdlog/spdlog.h>

#include <chrono>
#include <utility>

namespace battle
{

namespace
{
// 按房间查询默认关卡与副本，与客户端离线入口保持同一初始化参数。
std::pair<int32_t, int32_t> resolveStageAndCopy(int32_t roomId)
{
    auto& config = mugen::conf::Config::instance();
    int32_t stageId = 0;
    int32_t copyId = 0;
    for (const auto& stage : config.table<mugen::conf::StageConfig>())
    {
        if (stage.roomId == roomId)
        {
            stageId = stage.id;
            break;
        }
    }
    for (const auto& copy : config.table<mugen::conf::CopyConfig>())
    {
        if (copy.stageId == stageId)
        {
            copyId = copy.id;
            break;
        }
    }
    return {stageId, copyId};
}
}  // namespace

std::unique_ptr<BattleRoom> BattleRoom::create(IBattleSender& sender, const BattleRoomConfig& config)
{
    auto& conf = mugen::conf::Config::instance();
    const auto* room = conf.find<mugen::conf::RoomConfig>(config.roomId);
    if (!room)
    {
        spdlog::error("BattleRoom {}: room config {} not found", config.battleId, config.roomId);
        return nullptr;
    }

    mugen::sim::WorldSetup setup;
    setup.seed    = config.randomSeed != 0 ? config.randomSeed : 1;
    setup.mode    = mugen::sim::WorldMode::Battle;
    setup.roomId  = config.roomId;
    setup.mapKey  = room->mapKey;
    setup.m_pvp   = config.pvp;
    setup.m_aiEnabled = true;
    const auto [stageId, copyId] = resolveStageAndCopy(config.roomId);
    setup.m_stageId = stageId;
    setup.m_copyId  = copyId;

    for (const auto& slot : config.players)
    {
        const auto characterClass = static_cast<mugen::CharacterClass>(slot.classId);
        mugen::sim::PlayerSetup ps;
        ps.m_slot        = static_cast<uint32_t>(slot.slot);
        ps.m_roleId      = mugen::type_conversions::toRoleConfigId(characterClass);
        ps.m_class       = characterClass;
        ps.m_level       = slot.level;
        ps.m_camp        = slot.camp;
        ps.m_aiDifficulty = 1;
        ps.m_isAi        = false;
        setup.m_players.push_back(ps);
    }

    auto instance = std::unique_ptr<BattleRoom>(new BattleRoom(sender, config));
    instance->m_setup     = setup;
    instance->m_authority = std::make_unique<mugen::sim::net::LockstepAuthority>(setup);

    spdlog::info("BattleRoom {} created room={} map={} pvp={} players={}",
                 config.battleId,
                 config.roomId,
                 room->mapKey,
                 config.pvp,
                 config.players.size());
    return instance;
}

BattleRoom::BattleRoom(IBattleSender& sender, BattleRoomConfig config)
    : m_sender(sender), m_config(std::move(config))
{
}

BattleRoom::~BattleRoom() = default;

bool BattleRoom::markReady(std::uint32_t sessionId)
{
    auto* slot = findSlot(sessionId);
    if (!slot)
        return false;
    slot->ready = true;
    // 战斗已在运行：重连会话或迟到的槽位还没收到开始参数，补发后用快照复位到当前帧。
    if (m_state == State::Running && !slot->startSent)
    {
        slot->startSent = true;
        m_sender.sendStart(sessionId, makeStartPush(*slot, true));
        sendWorldSnapshot(sessionId);
        spdlog::info("BattleRoom {}: slot {} synced from frame {}",
                     m_config.battleId, slot->slot, m_authority->currentFrame());
    }
    return true;
}

void BattleRoom::submitInput(std::uint32_t sessionId, const PB::Battle::BattleInputPush& push)
{
    if (m_state != State::Running)
        return;
    if (push.battle_id() != m_config.battleId)
        return;
    PlayerSlot* slot = findSlot(sessionId);
    if (!slot)
        return;

    if (push.client_time_ms() != 0)
        slot->echoClientTimeMs = push.client_time_ms();

    for (const auto& frame : push.inputs())
    {
        // 客户端每次都冗余重发最近若干帧输入，已收到过的帧（以及超前过多、落出回溯窗口的帧）解码前直接跳过。
        if (!m_authority->needsInput(static_cast<uint32_t>(slot->slot), frame.frame()))
            continue;
        mugen::sim::PlayerInput input;
        if (!mugen::sim::net::decodePlayerInput(frame.input(), input))
        {
            spdlog::warn("BattleRoom {} slot {}: decode input frame {} failed",
                         m_config.battleId, slot->slot, frame.frame());
            continue;
        }
        m_authority->submitInput(static_cast<uint32_t>(slot->slot), frame.frame(), std::move(input));
    }

    // 客户端哈希与权威哈希对账，不一致仅告警（客户端自行发起重同步）。
    for (const auto& clientHash : push.client_hashes())
    {
        const std::uint64_t authorityHash = m_authority->hashAt(clientHash.frame());
        if (authorityHash != 0 && authorityHash != clientHash.hash())
            spdlog::warn("BattleRoom {} slot {} frame {} hash mismatch client={:016x} server={:016x}",
                         m_config.battleId,
                         slot->slot,
                         clientHash.frame(),
                         clientHash.hash(),
                         authorityHash);
    }
}

void BattleRoom::handleResync(std::uint32_t sessionId, const PB::Battle::BattleResyncReq& req)
{
    if (m_state != State::Running)
        return;
    if (req.battle_id() != m_config.battleId)
        return;
    if (!findSlot(sessionId))
        return;

    sendWorldSnapshot(sessionId);
}

void BattleRoom::sendWorldSnapshot(std::uint32_t sessionId)
{
    std::string world;
    m_authority->serializeWorld(world);

    PB::Battle::BattleSnapshotPush push;
    push.set_battle_id(m_config.battleId);
    push.set_frame(static_cast<std::uint32_t>(m_authority->currentFrame()));
    push.set_world(world);
    push.set_server_time_ms(m_startServerTimeMs + static_cast<std::uint64_t>(m_elapsedMs));
    // frames_after 留空：快照即当前帧，之后尚无确认帧。
    m_sender.sendSnapshot(sessionId, push);
}

void BattleRoom::handleLeave(std::uint32_t sessionId)
{
    if (m_state == State::Ended)
        return;
    if (!findSlot(sessionId))
        return;
    spdlog::info("BattleRoom {}: session {} surrendered", m_config.battleId, sessionId);
    end(BattleEndReason::Surrender, opponentCamp(sessionId));
}

void BattleRoom::setSessionOnline(std::uint32_t sessionId, bool online)
{
    auto* slot = findSlot(sessionId);
    if (!slot)
        return;
    if (online)
    {
        slot->connected = true;
        slot->disconnectedSeconds = 0.0f;
        return;
    }
    if (!slot->connected)
        return;
    slot->connected = false;
    slot->disconnectedSeconds = 0.0f;
    spdlog::info("BattleRoom {}: slot {} offline, battle continues (grace {}s)",
                 m_config.battleId, slot->slot, m_config.reconnectGraceSeconds);
    // 清掉已排队的输入并松开按键，避免角色保持断线前的跑动 / 攻击状态。
    if (m_state == State::Running)
        m_authority->releaseSlot(static_cast<std::uint32_t>(slot->slot));
}

RejoinResult BattleRoom::rejoin(std::int64_t playerId, std::uint32_t newSessionId, std::uint32_t& oldSessionId)
{
    if (m_state == State::Ended)
        return RejoinResult::Ended;
    for (auto& p : m_config.players)
    {
        if (p.playerId != playerId)
            continue;
        oldSessionId        = p.sessionId;
        p.sessionId         = newSessionId;
        p.connected         = true;
        p.disconnectedSeconds = 0.0f;
        p.startSent         = false;
        p.echoClientTimeMs  = 0;
        spdlog::info("BattleRoom {}: slot {} rejoined with session {} (was {})",
                     m_config.battleId, p.slot, newSessionId, oldSessionId);
        return RejoinResult::Ok;
    }
    return RejoinResult::NoPlayer;
}

void BattleRoom::tick(float dt)
{
    if (m_state == State::Ended)
    {
        m_endedSeconds += dt;
        return;
    }

    if (m_state == State::WaitingReady)
    {
        m_waitingSeconds += dt;

        bool allReady = !m_config.players.empty();
        bool anyReady = false;
        for (const auto& p : m_config.players)
        {
            if (p.ready)
                anyReady = true;
            else
                allReady = false;
        }

        if (allReady)
        {
            start();
            return;
        }
        if (m_waitingSeconds >= kReadyTimeoutSeconds)
        {
            if (anyReady)
                start();
            else
                end(BattleEndReason::Abort, 0);
        }
        return;
    }

    // Running：断线宽限耗尽判负；宽限内战斗照常推进，该槽位输入为空。
    for (auto& p : m_config.players)
    {
        if (p.connected)
            continue;
        p.disconnectedSeconds += dt;
        if (p.disconnectedSeconds >= m_config.reconnectGraceSeconds)
        {
            spdlog::warn("BattleRoom {}: session {} did not return within {}s, forfeit",
                         m_config.battleId, p.sessionId, m_config.reconnectGraceSeconds);
            end(BattleEndReason::Disconnect, opponentCamp(p.sessionId));
            return;
        }
    }

    // 按墙钟累加器推进目标帧，落后时多步追赶并封顶。
    m_elapsedMs += static_cast<double>(dt) * 1000.0;
    const std::uint64_t targetFrame =
        static_cast<std::uint64_t>(m_elapsedMs / static_cast<double>(kBattleFrameMs));

    struct ConfirmedFrame
    {
        std::uint64_t frame;
        std::string encoded;
    };
    std::vector<ConfirmedFrame> stepped;
    std::uint32_t catchUp = 0;
    while (m_authority->currentFrame() < targetFrame && catchUp < kMaxCatchUpFrames)
    {
        const auto& confirmed = m_authority->step();
        std::string encoded   = mugen::sim::net::encodeFrameInput(confirmed);
        stepped.push_back({confirmed.frame, encoded});
        m_frameHistory.push_back(encoded);
        if (m_frameHistory.size() > kConfirmedFrameHistory)
            m_frameHistory.pop_front();
        ++catchUp;
    }

    // 收集自上次广播以来的新哈希。
    std::vector<std::pair<std::uint64_t, std::uint64_t>> newHashes;
    for (const auto& [frame, hash] : m_authority->hashes())
    {
        if (frame > m_lastSentHashFrame)
        {
            newHashes.emplace_back(frame, hash);
            m_lastSentHashFrame = frame;
        }
    }

    // 权威世界产出结果时结算。
    if (auto outcome = m_authority->outcome())
    {
        const BattleEndReason reason = outcome->m_reason == mugen::sim::PvpRoundEndReason::Ko
                                           ? BattleEndReason::Ko
                                           : BattleEndReason::Timeout;
        end(reason, outcome->m_winnerCamp);
        return;
    }

    if (stepped.empty() && newHashes.empty())
        return;

    const std::uint64_t serverTimeMs = m_startServerTimeMs + static_cast<std::uint64_t>(m_elapsedMs);
    for (auto& p : m_config.players)
    {
        // 断线或尚未收到开始参数的会话不发帧；重连后由快照复位，之后的帧才有意义。
        if (!p.connected || !p.startSent)
            continue;
        PB::Battle::BattleFramesPush push;
        push.set_battle_id(m_config.battleId);
        push.set_server_frame(static_cast<std::uint32_t>(m_authority->currentFrame()));
        push.set_server_time_ms(serverTimeMs);
        for (const auto& confirmed : stepped)
        {
            auto* frame = push.add_frames();
            frame->set_frame(static_cast<std::uint32_t>(confirmed.frame));
            frame->set_input(confirmed.encoded);
        }
        for (const auto& [frame, hash] : newHashes)
        {
            auto* h = push.add_hashes();
            h->set_frame(static_cast<std::uint32_t>(frame));
            h->set_hash(hash);
        }
        // 本槽位的输入到达反馈：客户端据此测 RTT、自适应输入领先量。
        const auto slotIndex = static_cast<std::uint32_t>(p.slot);
        const auto& inputStats = m_authority->inputStats(slotIndex);
        push.set_late_inputs(inputStats.m_lateInputs);
        if (inputStats.m_windowSamples > 0)
        {
            push.set_input_slack_min(inputStats.m_windowMinSlack);
            push.set_input_slack_frame(static_cast<std::uint32_t>(inputStats.m_windowMinFrame));
            push.set_input_slack_samples(inputStats.m_windowSamples);
            m_authority->resetSlackWindow(slotIndex);
        }
        if (p.echoClientTimeMs != 0)
        {
            push.set_echo_client_time_ms(p.echoClientTimeMs);
            p.echoClientTimeMs = 0;
        }
        m_sender.sendFrames(p.sessionId, push);
    }
}

void BattleRoom::start()
{
    m_state             = State::Running;
    m_startServerTimeMs = nowMs();
    for (auto& p : m_config.players)
    {
        // 等待就绪期间已断线的槽位：开局即待机。
        if (!p.connected)
            m_authority->releaseSlot(static_cast<std::uint32_t>(p.slot));
        // 未就绪的槽位（客户端还没进战斗界面）此刻收不到推送，等它发 BattleReadyReq 时再补发开始参数 + 快照。
        if (!p.ready)
            continue;
        p.startSent = true;
        m_sender.sendStart(p.sessionId, makeStartPush(p, false));
    }
    spdlog::info("BattleRoom {} started with {} slots", m_config.battleId, m_config.players.size());
}

void BattleRoom::end(BattleEndReason reason, int32_t winnerCamp)
{
    if (m_state == State::Ended)
        return;
    m_state        = State::Ended;
    m_endedSeconds = 0.0f;
    pushEnd(reason, winnerCamp);
    spdlog::info("BattleRoom {} ended reason={} winner_camp={}",
                 m_config.battleId,
                 static_cast<int32_t>(reason),
                 winnerCamp);
}

PB::Battle::BattleStartPush BattleRoom::makeStartPush(const PlayerSlot& slot, bool resume) const
{
    PB::Battle::BattleStartPush push;
    push.set_battle_id(m_config.battleId);
    push.set_local_slot(static_cast<std::uint32_t>(slot.slot));
    push.set_slot_count(static_cast<std::uint32_t>(m_config.players.size()));
    push.set_setup(mugen::sim::net::encodeWorldSetup(m_setup));
    push.set_start_server_time_ms(m_startServerTimeMs);
    push.set_frame_ms(kBattleFrameMs);
    push.set_input_delay_frames(kInputDelayFrames);
    push.set_resume(resume);
    for (const auto& p : m_config.players)
    {
        auto* info = push.add_players();
        info->set_slot(static_cast<std::uint32_t>(p.slot));
        info->set_player_id(p.playerId);
        info->set_name(p.name);
        info->set_class_id(p.classId);
        info->set_camp(p.camp);
        *info->mutable_appearance() = p.appearance;
    }
    return push;
}

void BattleRoom::pushEnd(BattleEndReason reason, int32_t winnerCamp)
{
    const std::uint32_t endFrame =
        m_authority ? static_cast<std::uint32_t>(m_authority->currentFrame()) : 0;

    PB::Battle::BattleEndPush endPush;
    endPush.set_battle_id(m_config.battleId);
    endPush.set_winner_camp(winnerCamp);
    endPush.set_reason(static_cast<std::int32_t>(reason));
    endPush.set_end_frame(endFrame);
    for (const auto& p : m_config.players)
    {
        auto* result = endPush.add_players();
        result->set_player_id(p.playerId);
        result->set_slot(p.slot);
        result->set_camp(p.camp);
        result->set_win(winnerCamp != 0 && p.camp == winnerCamp);
    }
    for (const auto& p : m_config.players)
        m_sender.sendEnd(p.sessionId, endPush);

    PB::BattleInternal::BattleResultReq result;
    result.set_battle_id(m_config.battleId);
    result.set_winner_camp(winnerCamp);
    result.set_reason(static_cast<std::int32_t>(reason));
    result.set_end_frame(endFrame);
    for (const auto& p : m_config.players)
    {
        auto* pr = result.add_players();
        pr->set_player_id(p.playerId);
        pr->set_slot(p.slot);
        pr->set_camp(p.camp);
        pr->set_win(winnerCamp != 0 && p.camp == winnerCamp);
    }
    m_sender.sendResult(result);
}

int32_t BattleRoom::opponentCamp(std::uint32_t leavingSession) const
{
    for (const auto& p : m_config.players)
        if (p.sessionId != leavingSession)
            return p.camp;
    return 0;
}

PlayerSlot* BattleRoom::findSlot(std::uint32_t sessionId)
{
    for (auto& p : m_config.players)
        if (p.sessionId == sessionId)
            return &p;
    return nullptr;
}

const PlayerSlot* BattleRoom::findSlot(std::uint32_t sessionId) const
{
    for (const auto& p : m_config.players)
        if (p.sessionId == sessionId)
            return &p;
    return nullptr;
}

std::uint64_t BattleRoom::nowMs()
{
    return static_cast<std::uint64_t>(
        std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::steady_clock::now().time_since_epoch())
            .count());
}

}  // namespace battle
