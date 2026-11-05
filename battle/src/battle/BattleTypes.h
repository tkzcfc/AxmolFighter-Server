#pragma once

#include "game_types.pb.h"

#include <cstdint>
#include <string>
#include <vector>

namespace battle
{

// 服务器层战斗结束原因。前两项与 mugen::sim::PvpRoundEndReason 数值对齐，
// 后三项是服务器主动终结（投降 / 断线 / 无人就绪）。
enum class BattleEndReason : int32_t
{
    // 一方阵营被清空（对齐 PvpRoundEndReason::Ko）。
    Ko = 0,
    // 限时到达（对齐 PvpRoundEndReason::Timeout）。
    Timeout = 1,
    // 客户端 BattleLeaveReq 投降。
    Surrender = 2,
    // 玩家断线超时判负。
    Disconnect = 3,
    // 无人就绪，创建即中止。
    Abort = 4,
};

// 广播帧的毫秒数，与 mugen::sim::kLogicDtMs 对齐并取整 33。
inline constexpr std::uint32_t kBattleFrameMs = 33;
// 客户端输入领先量（BattleStartPush.input_delay_frames）的初始值与建议下限；客户端会按
// 服务器反馈的输入到达富余帧数自适应调大，上限由 mugen::sim::net::kMaxInputLead 约束。
inline constexpr std::uint32_t kInputDelayFrames = 4;
// 等待全部槽位 BattleReadyReq 的超时秒数。
inline constexpr float kReadyTimeoutSeconds = 20.0f;
// 玩家断线后允许重连的默认宽限秒数（可由 battle.toml 的 reconnect_grace_seconds 覆盖）。
inline constexpr float kDefaultReconnectGraceSeconds = 60.0f;
// 结束后销毁房间的宽限秒数。
inline constexpr float kEndDestroySeconds = 3.0f;
// 单次 tick 追赶帧数上限。
inline constexpr std::uint32_t kMaxCatchUpFrames = 5;
// 已确认帧历史环容量，供重同步查询。
inline constexpr std::size_t kConfirmedFrameHistory = 300;

// 单个已入座玩家。
struct PlayerSlot
{
    std::uint32_t sessionId = 0;
    std::int64_t playerId = 0;
    std::string name;
    // 职业 CharacterClass（1..7）。
    std::int32_t classId = 0;
    std::int32_t slot = 0;
    std::int32_t camp = 0;
    std::int32_t level = 1;
    // 外观，仅随 BattleStartPush 原样转发给客户端，不进入模拟。
    PB::Types::TownAppearance appearance;
    bool ready = false;
    bool connected = true;
    // 断线后累计秒数，重连时清零。
    float disconnectedSeconds = 0.0f;
    // 当前 session 是否已收到 BattleStartPush（重连后置 false，待新 session 发 BattleReadyReq 补发）。
    bool startSent = false;
    // 最近一次收到的 BattleInputPush.client_time_ms，下一条 BattleFramesPush 回显后清零（0 表示无待回显）。
    std::uint64_t echoClientTimeMs = 0;
};

// 战斗房间创建参数（已从 BattleCreateReq 解析）。
struct BattleRoomConfig
{
    std::uint32_t battleId = 0;
    std::int32_t roomId = 0;
    std::uint64_t randomSeed = 0;
    bool pvp = false;
    // 断线重连宽限秒数，超时未回来判负 Disconnect。
    float reconnectGraceSeconds = kDefaultReconnectGraceSeconds;
    std::vector<PlayerSlot> players;
};

// BattleRoom::rejoin 的结果。
enum class RejoinResult : int32_t
{
    Ok = 0,
    // 战斗不存在（仅 Manager 返回）。
    NoBattle = 1,
    // 战斗已结束。
    Ended = 2,
    // 玩家不在该战斗中。
    NoPlayer = 3,
};

}  // namespace battle
