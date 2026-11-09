#pragma once

#include "battle/BattleTypes.h"
#include "battle/IBattleSender.h"
#include "mugen/sim_legacy/WorldSetup.h"
#include "mugen/sim_legacy/net/LockstepAuthority.h"

#include <cstdint>
#include <deque>
#include <map>
#include <memory>
#include <string>
#include <vector>

namespace battle
{

// 单场战斗实例：持有服务器权威世界，负责就绪收集、固定帧推进、输入提交、
// 帧广播、重同步与结束结算。网络发送经 IBattleSender 解耦。
class BattleRoom
{
public:
    // 以解析后的创建参数构造权威世界；配置缺失或引导失败返回 nullptr。
    static std::unique_ptr<BattleRoom> create(IBattleSender& sender, const BattleRoomConfig& config);

    ~BattleRoom();

    // 标记某会话就绪；槽位未知返回 false。战斗已在运行时（重连或迟到的槽位）
    // 会向该会话补发 BattleStartPush + 当前世界 BattleSnapshotPush。
    bool markReady(std::uint32_t sessionId);
    // 提交客户端输入帧；未运行或战斗不匹配时忽略。
    void submitInput(std::uint32_t sessionId, const PB::Battle::BattleInputPush& push);
    // 处理重同步请求，下发当前世界快照。
    void handleResync(std::uint32_t sessionId, const PB::Battle::BattleResyncReq& req);
    // 处理投降 / 主动离开。
    void handleLeave(std::uint32_t sessionId);
    // 会话上线 / 下线（断线标记与宽限计时）。下线后该槽位的输入被替换为空输入并松开所有按键，
    // 战斗继续运行，直到重连宽限耗尽才判负。
    void setSessionOnline(std::uint32_t sessionId, bool online);
    // 玩家用新会话重新接入：把槽位改绑到 newSessionId，清除断线计时，并等待新会话
    // 发 BattleReadyReq 后补发开始参数与快照。oldSessionId 返回被替换的会话（供 Manager 更新路由）。
    RejoinResult rejoin(std::int64_t playerId, std::uint32_t newSessionId, std::uint32_t& oldSessionId);
    // 推进战斗状态机；dt 为墙钟秒数。
    void tick(float dt);

    std::uint32_t battleId() const { return m_config.battleId; }
    bool finished() const { return m_state == State::Ended; }
    // 结束后累计的宽限秒数，供 Manager 延迟销毁。
    float endedSeconds() const { return m_endedSeconds; }
    bool hasSession(std::uint32_t sessionId) const { return findSlot(sessionId) != nullptr; }
    // 只读访问权威锁步世界（供测试与诊断）。
    const mugen::sim_legacy::net::LockstepAuthority& authority() const { return *m_authority; }

private:
    enum class State
    {
        WaitingReady,
        Running,
        Ended,
    };

    BattleRoom(IBattleSender& sender, BattleRoomConfig config);

    void start();
    void end(BattleEndReason reason, int32_t winnerCamp);
    // 为某会话构造 BattleStartPush；resume=true 表示战斗已在进行，随后会有快照。
    PB::Battle::BattleStartPush makeStartPush(const PlayerSlot& slot, bool resume) const;
    // 向会话下发当前世界快照（重同步与重连共用）。
    void sendWorldSnapshot(std::uint32_t sessionId);
    // 广播结束并上报 game 服。
    void pushEnd(BattleEndReason reason, int32_t winnerCamp);
    // 找到除离场会话外的剩余阵营，作为判负的胜方。
    int32_t opponentCamp(std::uint32_t leavingSession) const;

    PlayerSlot* findSlot(std::uint32_t sessionId);
    const PlayerSlot* findSlot(std::uint32_t sessionId) const;

    static std::uint64_t nowMs();

private:
    IBattleSender& m_sender;
    BattleRoomConfig m_config;
    // 随开始广播下发的世界配置（含 seed/room/mapKey/玩家）。
    mugen::sim_legacy::WorldSetup m_setup;
    std::unique_ptr<mugen::sim_legacy::net::LockstepAuthority> m_authority;
    State m_state = State::WaitingReady;

    // 墙钟累计毫秒，驱动目标帧。
    double m_elapsedMs = 0.0;
    // 开始运行时记录的墙钟基准（帧 0 对应时间戳）。
    std::uint64_t m_startServerTimeMs = 0;
    // 就绪等待累计秒数。
    float m_waitingSeconds = 0.0f;
    // 结束后累计的宽限秒数。
    float m_endedSeconds = 0.0f;
    // 已广播过的最大哈希帧号。
    std::uint64_t m_lastSentHashFrame = 0;
    // 已确认帧历史环，供重同步查询。
    std::deque<std::string> m_frameHistory;
};

}  // namespace battle
