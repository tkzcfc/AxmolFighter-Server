use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use backend_framework::session::BackendSession;
use protocol::message_map::MessageType;
use protocol::town::{
    ScenePlayerAppearancePush, ScenePlayerEnterPush, ScenePlayerLeavePush, ScenePlayerStatesPush,
    TownMoveReport, TownPositionCorrectionPush, TownStateEntry,
};
use protocol::town_internal::{
    TownEnterSceneReq, TownEnterSceneResp, TownLeaveSceneReq, TownLeaveSceneResp,
    TownUpdateAppearanceReq,
};
use protocol::types::{
    SceneEnterType, SceneInfo, TownAppearance, TownMovement, TownPlayerInfo, TownVec3, TownVel,
};
use tracing::warn;

/// 坐标/位移定点缩放：真实值 = 字段值 / POSITION_SCALE（1/1000 像素）。
const POSITION_SCALE: f32 = 1000.0;

/// 广播 tick 之间允许的最大服务端时间间隔（毫秒）。用于避免停机/长间隔导致的
/// 位移上限放大到不合理程度。
const MAX_TICK_DELTA_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SceneKey {
    Map(u32),
    Home(u64),
}

impl SceneKey {
    fn from_parts(scene_type: i32, map_id: u32, owner_uid: u64) -> Result<Self, &'static str> {
        match SceneEnterType::try_from(scene_type) {
            Ok(SceneEnterType::Map) if map_id > 0 => Ok(Self::Map(map_id)),
            Ok(SceneEnterType::Map) => Err("map_id must be greater than 0"),
            Ok(SceneEnterType::Home) if owner_uid > 0 => Ok(Self::Home(owner_uid)),
            Ok(SceneEnterType::Home) => Err("owner_uid must be greater than 0"),
            _ => Err("invalid scene enter type"),
        }
    }

    fn scene_info(self) -> SceneInfo {
        match self {
            Self::Map(map_id) => SceneInfo {
                r#type: SceneEnterType::Map as i32,
                map_id,
                owner_uid: 0,
            },
            Self::Home(owner_uid) => SceneInfo {
                r#type: SceneEnterType::Home as i32,
                map_id: 0,
                owner_uid,
            },
        }
    }
}

/// 场景可选矩形边界（定点坐标）。用于把违规位移 clamp 回边界内。
#[derive(Clone, Copy, Debug)]
pub struct SceneBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub min_z: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub max_z: i32,
}

/// 城镇同步配置（移动校验 / 广播 / AOI）。
#[derive(Clone, Copy, Debug)]
pub struct SyncConfig {
    /// 允许的最大移动速度（像素/秒）。
    pub max_speed: f32,
    /// 位移校验容差（相对 max_speed 的倍数）。
    pub tolerance: f32,
    /// 位移预算（token bucket）的突发时长（秒）。
    pub burst_seconds: f32,
    /// AOI 半径（像素）。0 表示关闭 AOI（整个场景可见）。
    pub aoi_radius: f32,
    /// 广播 tick 间隔（毫秒）。
    pub tick_ms: u64,
    /// 场景边界（可选）。None 表示不校验边界。
    pub bounds: Option<SceneBounds>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            max_speed: 600.0,
            tolerance: 1.5,
            burst_seconds: 0.5,
            aoi_radius: 0.0,
            tick_ms: 100,
            bounds: None,
        }
    }
}

impl SyncConfig {
    /// AOI 半径的定点平方值；0 表示关闭 AOI。
    fn aoi_radius_sq_scaled(&self) -> i64 {
        let radius_scaled = (self.aoi_radius * POSITION_SCALE) as i64;
        radius_scaled.saturating_mul(radius_scaled)
    }

    /// 位移预算上限（像素）：max_speed × tolerance × burst_seconds。
    fn budget_cap_px(&self) -> f32 {
        self.max_speed.max(0.0) * self.tolerance * self.burst_seconds.max(0.0)
    }

    /// 位移预算恢复速率（像素/秒）：max_speed × tolerance。
    fn budget_rate_px_per_s(&self) -> f32 {
        self.max_speed.max(0.0) * self.tolerance
    }
}

/// 服务端保存的「最后接受的权威移动状态」。
#[derive(Clone, Debug)]
pub struct AcceptedMovement {
    pub pos: TownVec3,
    pub facing: i32,
    pub moving: bool,
    pub vel: TownVel,
    pub state: String,
}

impl AcceptedMovement {
    fn to_proto(&self) -> TownMovement {
        TownMovement {
            pos: Some(self.pos.clone()),
            facing: self.facing,
            moving: self.moving,
            vel: Some(self.vel.clone()),
            state: self.state.clone(),
        }
    }
}

impl From<TownMovement> for AcceptedMovement {
    fn from(m: TownMovement) -> Self {
        Self {
            pos: m.pos.unwrap_or_default(),
            facing: m.facing,
            moving: m.moving,
            vel: m.vel.unwrap_or_default(),
            state: m.state,
        }
    }
}

/// 移动校验结果：服务端权威接受的移动状态，以及是否发生了校正（需要回发校正包）。
#[derive(Debug)]
struct ValidateResult {
    accepted: AcceptedMovement,
    corrected: bool,
}

/// 纯函数：校验一次移动上报。
///
/// - `last`：最后一次接受的权威状态（首个上报时与出生状态一致）。
/// - `reported`：客户端本次上报。
/// - `budget`：玩家当前的位移预算（像素）。通过 `&mut` 传入，校验时按接受/截断增减。
/// - `config`：边界/朝向校验配置。
///
/// 校验规则：位移（欧氏距离，像素）不得超过预算；超出则沿方向 clamp 到预算并把预算
/// 清零；随后按可选边界 clamp；朝向非法（非 0/1）归 0。
fn validate_movement(
    last: &AcceptedMovement,
    reported: &AcceptedMovement,
    budget: &mut f32,
    config: &SyncConfig,
) -> ValidateResult {
    let mut accepted = reported.clone();
    let mut corrected = false;

    let dx = (reported.pos.x - last.pos.x) as i64;
    let dy = (reported.pos.y - last.pos.y) as i64;
    let dz = (reported.pos.z - last.pos.z) as i64;
    let dist_sq = dx.saturating_mul(dx) + dy.saturating_mul(dy) + dz.saturating_mul(dz);
    let dist_px = ((dist_sq as f64).sqrt() / POSITION_SCALE as f64) as f32;

    if dist_px > *budget {
        // 沿方向 clamp 到预算上限（dist_sq > 0，因为超过预算必然有位移）。
        let allowed_scaled = (*budget as f64) * (POSITION_SCALE as f64);
        let allowed_sq = (allowed_scaled * allowed_scaled) as i64;
        let scale = scale_to_allowed(allowed_sq, dist_sq);
        accepted.pos.x = last.pos.x + scale_fixed(dx, scale);
        accepted.pos.y = last.pos.y + scale_fixed(dy, scale);
        accepted.pos.z = last.pos.z + scale_fixed(dz, scale);
        *budget = 0.0;
        corrected = true;
    } else {
        *budget -= dist_px;
    }

    // 可选边界 clamp。
    if let Some(bounds) = config.bounds {
        let clamped_x = accepted.pos.x.clamp(bounds.min_x, bounds.max_x);
        let clamped_y = accepted.pos.y.clamp(bounds.min_y, bounds.max_y);
        let clamped_z = accepted.pos.z.clamp(bounds.min_z, bounds.max_z);
        if clamped_x != accepted.pos.x || clamped_y != accepted.pos.y || clamped_z != accepted.pos.z
        {
            accepted.pos.x = clamped_x;
            accepted.pos.y = clamped_y;
            accepted.pos.z = clamped_z;
            corrected = true;
        }
    }

    // 朝向规范化。
    if accepted.facing != 0 && accepted.facing != 1 {
        accepted.facing = 0;
        corrected = true;
    }

    ValidateResult { accepted, corrected }
}

/// 给定平方上限与平方距离，返回位移缩放因子（<= 1.0）。
fn scale_to_allowed(allowed_sq: i64, dist_sq: i64) -> f64 {
    if dist_sq <= 0 {
        return 0.0;
    }
    ((allowed_sq as f64) / (dist_sq as f64)).sqrt().min(1.0)
}

/// 按缩放因子缩放定点位移。
fn scale_fixed(v: i64, scale: f64) -> i32 {
    ((v as f64) * scale).round() as i32
}

/// 广播条目（TickEntry 的原始结构，避免直接依赖 prost 类型便于测试）。
#[derive(Clone, Debug)]
struct TickEntry {
    player_id: i64,
    server_time_ms: u64,
    movement: AcceptedMovement,
}

impl TickEntry {
    fn to_proto(&self) -> TownStateEntry {
        TownStateEntry {
            player_id: self.player_id,
            server_time_ms: self.server_time_ms,
            movement: Some(self.movement.to_proto()),
        }
    }
}

#[derive(Clone)]
struct Player {
    session_id: u32,
    appearance: TownAppearance,
    movement: AcceptedMovement,
    /// 自上次广播后是否发生变化（send-on-change）。
    dirty: bool,
    /// 当前 AOI 邻居（开启 AOI 时维护）。
    neighbors: HashSet<i64>,
    /// 上次接受移动上报的时刻（用于位移预算的时间间隔累积）。
    last_report_at: Instant,
    /// 位移预算（token bucket，像素）。进入场景时置满，随上报消耗、随时间恢复。
    move_budget_px: f32,
    /// 待客户端确认的校正序号（已下发的最近一次校正的 client_seq）。
    pending_correction_seq: Option<u32>,
}

#[derive(Default)]
struct PushBatch {
    /// (receiver_session, entering player info)
    player_enter: Vec<(u32, TownPlayerInfo)>,
    /// (receiver_session, leaving player id)
    player_leave: Vec<(u32, i64)>,
    /// (receiver_session, appearance push)
    player_appearance: Vec<(u32, ScenePlayerAppearancePush)>,
    /// (receiver_session, state entries)
    player_states: Vec<(u32, Vec<TownStateEntry>)>,
}

struct EnterSceneResult {
    response: TownEnterSceneResp,
    pushes: PushBatch,
}

struct LeaveSceneResult {
    response: TownLeaveSceneResp,
    pushes: PushBatch,
}

/// 移动上报处理结果：可选的校正包 + 待广播批次。
#[derive(Default)]
struct MoveReportResult {
    correction: Option<TownPositionCorrectionPush>,
    pushes: PushBatch,
}

struct Scene {
    #[allow(dead_code)]
    scene_id: u64,
    info: SceneInfo,
    players: HashMap<i64, Player>,
}

struct SceneManager {
    next_scene_id: u64,
    scenes_by_key: HashMap<SceneKey, u64>,
    scenes: HashMap<u64, Scene>,
    player_scenes: HashMap<i64, u64>,
    session_players: HashMap<u32, i64>,
    config: SyncConfig,
}

impl SceneManager {
    fn new(config: SyncConfig) -> Self {
        Self {
            next_scene_id: 0,
            scenes_by_key: HashMap::new(),
            scenes: HashMap::new(),
            player_scenes: HashMap::new(),
            session_players: HashMap::new(),
            config,
        }
    }

    fn enter_scene(&mut self, instance_id: u32, req: TownEnterSceneReq) -> EnterSceneResult {
        let key = match SceneKey::from_parts(req.r#type, req.map_id, req.owner_uid) {
            Ok(key) => key,
            Err(message) => {
                return EnterSceneResult {
                    response: TownEnterSceneResp {
                        code: 400,
                        message: message.to_string(),
                        town_instance_id: instance_id,
                        scene: None,
                        players: vec![],
                    },
                    pushes: PushBatch::default(),
                };
            }
        };

        let scene_id = self.ensure_scene(key);
        let appearance = req.appearance.unwrap_or_default();
        let movement: AcceptedMovement = req.movement.unwrap_or_default().into();

        if self.player_scenes.get(&req.player_id).copied() == Some(scene_id) {
            return self.update_player_in_scene(
                instance_id,
                scene_id,
                req.session_id,
                req.player_id,
                appearance,
                movement,
            );
        }

        let mut pushes = PushBatch::default();
        if let Some(old_player_id) = self.session_players.get(&req.session_id).copied()
            && old_player_id != req.player_id
        {
            self.remove_player(old_player_id, &mut pushes);
        }
        self.remove_player(req.player_id, &mut pushes);

        // 先插入进入者（邻居集合先置空，稍后 refresh_all_neighbors 重建）。
        let entering_info = TownPlayerInfo {
            player_id: req.player_id,
            appearance: Some(appearance.clone()),
            movement: Some(movement.to_proto()),
        };
        let neighbors = {
            let scene = self
                .scenes
                .get(&scene_id)
                .expect("scene must exist after ensure_scene");
            // 进入者进入时向「在半径内的既有玩家」广播进入推送。
            let neighbors = self.neighbors_of(scene, &movement.pos, req.player_id);
            for &other_id in &neighbors {
                if let Some(other) = scene.players.get(&other_id) {
                    pushes.player_enter.push((other.session_id, entering_info.clone()));
                }
            }
            neighbors
        };
        {
            let scene = self
                .scenes
                .get_mut(&scene_id)
                .expect("scene must exist after ensure_scene");
            scene.players.insert(
                req.player_id,
                Player {
                    session_id: req.session_id,
                    appearance,
                    movement,
                    dirty: true,
                    neighbors,
                    last_report_at: Instant::now(),
                    move_budget_px: self.config.budget_cap_px(),
                    pending_correction_seq: None,
                },
            );
        }
        self.player_scenes.insert(req.player_id, scene_id);
        self.session_players.insert(req.session_id, req.player_id);
        self.refresh_all_neighbors(scene_id);

        EnterSceneResult {
            response: self.enter_response(instance_id, scene_id),
            pushes,
        }
    }

    fn leave_scene(&mut self, req: TownLeaveSceneReq) -> LeaveSceneResult {
        let mut pushes = PushBatch::default();
        match self.session_players.get(&req.session_id).copied() {
            Some(player_id) if player_id == req.player_id => {
                self.remove_player(player_id, &mut pushes);
            }
            Some(_) => {
                return LeaveSceneResult {
                    response: TownLeaveSceneResp {
                        code: 403,
                        message: "session player mismatch".to_string(),
                    },
                    pushes,
                };
            }
            None => {}
        }

        LeaveSceneResult {
            response: TownLeaveSceneResp {
                code: 0,
                message: String::new(),
            },
            pushes,
        }
    }

    fn leave_session(&mut self, session_id: u32) -> PushBatch {
        let mut pushes = PushBatch::default();
        if let Some(player_id) = self.session_players.get(&session_id).copied() {
            self.remove_player(player_id, &mut pushes);
        }
        pushes
    }

    /// 处理一次移动上报：校验 → 更新权威状态 → 生成校正/广播。
    fn move_report(&mut self, session_id: u32, report: TownMoveReport) -> MoveReportResult {
        self.move_report_at(session_id, report, Instant::now())
    }

    /// `move_report` 的内层实现，`now` 显式传入以便测试注入时间。
    fn move_report_at(
        &mut self,
        session_id: u32,
        report: TownMoveReport,
        now: Instant,
    ) -> MoveReportResult {
        let mut result = MoveReportResult::default();
        let Some(player_id) = self.session_players.get(&session_id).copied() else {
            warn!("ignored town move report for unknown session {}", session_id);
            return result;
        };
        let Some(scene_id) = self.player_scenes.get(&player_id).copied() else {
            warn!("ignored town move report without scene session {}", session_id);
            return result;
        };

        let reported: AcceptedMovement = report.movement.unwrap_or_default().into();

        // 校验并更新权威状态（借用 scene 可变）。
        let (accepted, corrected, old_neighbors) = {
            let Some(scene) = self.scenes.get_mut(&scene_id) else {
                return result;
            };
            let Some(player) = scene.players.get_mut(&player_id) else {
                return result;
            };

            // 校正确认：若上一份校正尚未被客户端确认，丢弃中间在途上报，
            // 既不校验也不更新权威状态，同时不推进预算累积时间（时间继续累积预算）。
            if let Some(pending) = player.pending_correction_seq {
                if report.ack_correction_seq < pending {
                    warn!(
                        "dropped town move report seq={} ack={} while correction seq={} pending",
                        report.client_seq, report.ack_correction_seq, pending
                    );
                    return result;
                }
                player.pending_correction_seq = None;
            }

            // 预算恢复：按服务端接收时间间隔累积，再 cap 到突发上限。
            let elapsed_ms = now
                .saturating_duration_since(player.last_report_at)
                .as_millis()
                .min(MAX_TICK_DELTA_MS as u128) as u64;
            let elapsed_s = elapsed_ms as f32 / 1000.0;
            let gain = self.config.budget_rate_px_per_s() * elapsed_s;
            player.move_budget_px =
                (player.move_budget_px + gain).min(self.config.budget_cap_px());

            let validate =
                validate_movement(&player.movement, &reported, &mut player.move_budget_px, &self.config);

            let old_neighbors = player.neighbors.clone();
            player.movement = validate.accepted.clone();
            player.last_report_at = now;
            if validate.corrected {
                player.pending_correction_seq = Some(report.client_seq);
            }
            player.dirty = true;
            (validate.accepted, validate.corrected, old_neighbors)
        };

        if corrected {
            result.correction = Some(TownPositionCorrectionPush {
                client_seq: report.client_seq,
                pos: Some(accepted.pos),
            });
        }

        // 刷新 AOI 邻居（保持双向对称），并据此生成跨越进入/离开推送。
        self.refresh_all_neighbors(scene_id);
        self.emit_aoi_crossing(scene_id, player_id, &old_neighbors, &mut result.pushes);

        result
    }

    /// 依据玩家移动后的邻居集合与旧集合之差，生成 AOI 进入/离开推送。
    fn emit_aoi_crossing(
        &self,
        scene_id: u64,
        player_id: i64,
        old_neighbors: &HashSet<i64>,
        pushes: &mut PushBatch,
    ) {
        let Some(scene) = self.scenes.get(&scene_id) else {
            return;
        };
        let Some(mover) = scene.players.get(&player_id) else {
            return;
        };
        let new_neighbors = &mover.neighbors;
        let entering_info = TownPlayerInfo {
            player_id,
            appearance: Some(mover.appearance.clone()),
            movement: Some(mover.movement.to_proto()),
        };

        // 新可见但旧不可见：向进入者广播「对方进入」。
        for other_id in new_neighbors.difference(old_neighbors) {
            if let Some(other) = scene.players.get(other_id) {
                pushes.player_enter.push((other.session_id, entering_info.clone()));
            }
        }
        // 旧可见但新不可见：向离开者广播「对方离开」。
        for other_id in old_neighbors.difference(new_neighbors) {
            if let Some(other) = scene.players.get(other_id) {
                pushes.player_leave.push((other.session_id, player_id));
            }
        }
    }

    /// 重建整个场景的 AOI 邻居集合（双向对称）。关闭 AOI 时邻居 = 场景内其他全部玩家。
    fn refresh_all_neighbors(&mut self, scene_id: u64) {
        let Some(scene) = self.scenes.get(&scene_id) else {
            return;
        };
        // 先快照位置，避免在迭代 scene 时进行可变借用。
        let snapshot: Vec<(i64, TownVec3)> = scene
            .players
            .iter()
            .map(|(id, p)| (*id, p.movement.pos.clone()))
            .collect();
        let radius_sq = self.config.aoi_radius_sq_scaled();
        let mut new_neighbors: HashMap<i64, HashSet<i64>> = snapshot
            .iter()
            .map(|(id, _)| (*id, HashSet::new()))
            .collect();

        for (id_a, pos_a) in &snapshot {
            for (id_b, pos_b) in &snapshot {
                if id_a == id_b {
                    continue;
                }
                let dx = (pos_b.x - pos_a.x) as i64;
                let dy = (pos_b.y - pos_a.y) as i64;
                let dz = (pos_b.z - pos_a.z) as i64;
                let dist_sq =
                    dx.saturating_mul(dx) + dy.saturating_mul(dy) + dz.saturating_mul(dz);
                let visible = radius_sq <= 0 || dist_sq <= radius_sq;
                if visible {
                    new_neighbors.entry(*id_a).or_default().insert(*id_b);
                }
            }
        }

        if let Some(scene) = self.scenes.get_mut(&scene_id) {
            for (id, neighbors) in new_neighbors {
                if let Some(player) = scene.players.get_mut(&id) {
                    player.neighbors = neighbors;
                }
            }
        }
    }

    /// 更新玩家外观并向同场景成员广播外观变化。
    fn update_appearance(&mut self, req: TownUpdateAppearanceReq) -> PushBatch {
        let mut pushes = PushBatch::default();
        let Some(player_id) = self.session_players.get(&req.session_id).copied() else {
            warn!(
                "ignored town appearance update for unknown session {}",
                req.session_id
            );
            return pushes;
        };
        if player_id != req.player_id {
            warn!(
                "ignored town appearance update with player mismatch session {}",
                req.session_id
            );
            return pushes;
        }
        let Some(scene_id) = self.player_scenes.get(&player_id).copied() else {
            return pushes;
        };
        let Some(scene) = self.scenes.get_mut(&scene_id) else {
            return pushes;
        };
        let Some(player) = scene.players.get_mut(&player_id) else {
            return pushes;
        };
        player.appearance = req.appearance.unwrap_or_default();

        let push = ScenePlayerAppearancePush {
            player_id,
            appearance: Some(player.appearance.clone()),
        };
        for other in scene.players.values() {
            if other.session_id != req.session_id {
                pushes.player_appearance.push((other.session_id, push.clone()));
            }
        }
        pushes
    }

    /// 服务端广播 tick：把「发生变化」的玩家批量下发给同场景（AOI 内）其他客户端。
    fn tick(&mut self, server_time_ms: u64) -> PushBatch {
        let mut pushes = PushBatch::default();

        let dirty: Vec<(i64, Vec<u32>, TickEntry)> = {
            let mut acc = Vec::new();
            for scene in self.scenes.values_mut() {
                // 先收集脏玩家的邻居，避免可变借用期间再读 scene.players。
                let dirty_players: Vec<(i64, Vec<u32>, TickEntry)> = scene
                    .players
                    .iter()
                    .filter(|(_, p)| p.dirty)
                    .map(|(player_id, player)| {
                        let receivers = scene
                            .players
                            .iter()
                            .filter(|(other_id, _)| **other_id != *player_id)
                            .filter(|(other_id, _)| {
                                self.config.aoi_radius <= 0.0 || player.neighbors.contains(other_id)
                            })
                            .map(|(_, other)| other.session_id)
                            .collect();
                        (
                            *player_id,
                            receivers,
                            TickEntry {
                                player_id: *player_id,
                                server_time_ms,
                                movement: player.movement.clone(),
                            },
                        )
                    })
                    .collect();

                for (player_id, receivers, entry) in dirty_players {
                    if let Some(player) = scene.players.get_mut(&player_id) {
                        player.dirty = false;
                    }
                    acc.push((player_id, receivers, entry));
                }
            }
            acc
        };

        let mut by_receiver: HashMap<u32, Vec<TownStateEntry>> = HashMap::new();
        for (_player_id, receivers, entry) in dirty {
            // 没有接收者时也无需广播。
            if receivers.is_empty() {
                continue;
            }
            for session_id in receivers {
                by_receiver
                    .entry(session_id)
                    .or_default()
                    .push(entry.to_proto());
            }
        }

        for (session_id, entries) in by_receiver {
            pushes.player_states.push((session_id, entries));
        }
        pushes
    }

    fn clear(&mut self) {
        self.scenes_by_key.clear();
        self.scenes.clear();
        self.player_scenes.clear();
        self.session_players.clear();
    }

    fn ensure_scene(&mut self, key: SceneKey) -> u64 {
        if let Some(scene_id) = self.scenes_by_key.get(&key).copied() {
            return scene_id;
        }

        self.next_scene_id = self.next_scene_id.saturating_add(1);
        let scene_id = self.next_scene_id;
        self.scenes_by_key.insert(key, scene_id);
        self.scenes.insert(
            scene_id,
            Scene {
                scene_id,
                info: key.scene_info(),
                players: HashMap::new(),
            },
        );
        scene_id
    }

    fn update_player_in_scene(
        &mut self,
        instance_id: u32,
        scene_id: u64,
        session_id: u32,
        player_id: i64,
        appearance: TownAppearance,
        movement: AcceptedMovement,
    ) -> EnterSceneResult {
        let pushes = PushBatch::default();
        let scene = self
            .scenes
            .get_mut(&scene_id)
            .expect("scene must exist for player_scenes entry");
        if let Some(player) = scene.players.get_mut(&player_id) {
            self.session_players.remove(&player.session_id);
            player.session_id = session_id;
            player.appearance = appearance;
            player.movement = movement;
            player.dirty = true;
            player.last_report_at = Instant::now();
            player.move_budget_px = self.config.budget_cap_px();
            player.pending_correction_seq = None;
            self.session_players.insert(session_id, player_id);
        }
        self.refresh_all_neighbors(scene_id);

        EnterSceneResult {
            response: self.enter_response(instance_id, scene_id),
            pushes,
        }
    }

    fn remove_player(&mut self, player_id: i64, pushes: &mut PushBatch) {
        let Some(scene_id) = self.player_scenes.remove(&player_id) else {
            return;
        };
        let Some(scene) = self.scenes.get_mut(&scene_id) else {
            return;
        };
        let Some(player) = scene.players.remove(&player_id) else {
            return;
        };

        self.session_players.remove(&player.session_id);
        // 通知离开者的 AOI 邻居。
        for other_id in &player.neighbors {
            if let Some(other) = scene.players.get(other_id) {
                pushes.player_leave.push((other.session_id, player_id));
            }
        }
        // 从其他玩家的邻居集合中移除该玩家。
        for other in scene.players.values_mut() {
            other.neighbors.remove(&player_id);
        }
    }

    /// 计算 `position` 处、`exclude_id` 之外的 AOI 邻居（关闭 AOI 时为场景内全部其他玩家）。
    fn neighbors_of(&self, scene: &Scene, position: &TownVec3, exclude_id: i64) -> HashSet<i64> {
        let radius_sq = self.config.aoi_radius_sq_scaled();
        scene
            .players
            .keys()
            .copied()
            .filter(|id| *id != exclude_id)
            .filter(|id| {
                if radius_sq <= 0 {
                    return true;
                }
                let Some(other) = scene.players.get(id) else {
                    return false;
                };
                let dx = (other.movement.pos.x - position.x) as i64;
                let dy = (other.movement.pos.y - position.y) as i64;
                let dz = (other.movement.pos.z - position.z) as i64;
                dx.saturating_mul(dx) + dy.saturating_mul(dy) + dz.saturating_mul(dz) <= radius_sq
            })
            .collect()
    }

    fn enter_response(&self, instance_id: u32, scene_id: u64) -> TownEnterSceneResp {
        let scene = self
            .scenes
            .get(&scene_id)
            .expect("scene must exist for response");
        TownEnterSceneResp {
            code: 0,
            message: String::new(),
            town_instance_id: instance_id,
            scene: Some(scene.info.clone()),
            players: scene
                .players
                .iter()
                .map(|(player_id, player)| TownPlayerInfo {
                    player_id: *player_id,
                    appearance: Some(player.appearance.clone()),
                    movement: Some(player.movement.to_proto()),
                })
                .collect(),
        }
    }
}

pub(crate) struct TownShared {
    session: Arc<BackendSession>,
    instance_id: u32,
    manager: Mutex<SceneManager>,
}

impl TownShared {
    pub(crate) fn new(session: Arc<BackendSession>, instance_id: u32, config: SyncConfig) -> Arc<Self> {
        Arc::new_cyclic(move |weak| {
            let shared = Self {
                session,
                instance_id,
                manager: Mutex::new(SceneManager::new(config)),
            };
            // 启动固定频率广播 tick。
            spawn_tick_task(weak.clone(), config);
            shared
        })
    }

    pub(crate) fn enter_scene(&self, req: TownEnterSceneReq) -> TownEnterSceneResp {
        let result = self
            .manager
            .lock()
            .unwrap()
            .enter_scene(self.instance_id, req);
        self.send_pushes(result.pushes);
        result.response
    }

    pub(crate) fn leave_scene(&self, req: TownLeaveSceneReq) -> TownLeaveSceneResp {
        let result = self.manager.lock().unwrap().leave_scene(req);
        self.send_pushes(result.pushes);
        result.response
    }

    pub(crate) fn leave_session(&self, session_id: u32) {
        let pushes = self.manager.lock().unwrap().leave_session(session_id);
        self.send_pushes(pushes);
    }

    pub(crate) fn move_report(&self, session_id: u32, report: TownMoveReport) {
        let result = self.manager.lock().unwrap().move_report(session_id, report);
        if let Some(correction) = result.correction {
            self.session
                .send_msg(&MessageType::TownTownPositionCorrectionPush(correction), 0, session_id);
        }
        self.send_pushes(result.pushes);
    }

    pub(crate) fn update_appearance(&self, req: TownUpdateAppearanceReq) {
        let pushes = self.manager.lock().unwrap().update_appearance(req);
        self.send_pushes(pushes);
    }

    fn tick(&self) {
        let now_ms = epoch_ms();
        let pushes = self.manager.lock().unwrap().tick(now_ms);
        self.send_pushes(pushes);
    }

    pub(crate) fn clear(&self) {
        self.manager.lock().unwrap().clear();
    }

    fn send_pushes(&self, pushes: PushBatch) {
        for (session_id, info) in pushes.player_enter {
            self.session.send_msg(
                &MessageType::TownScenePlayerEnterPush(ScenePlayerEnterPush { player: Some(info) }),
                0,
                session_id,
            );
        }
        for (session_id, player_id) in pushes.player_leave {
            self.session.send_msg(
                &MessageType::TownScenePlayerLeavePush(ScenePlayerLeavePush { player_id }),
                0,
                session_id,
            );
        }
        for (session_id, push) in pushes.player_appearance {
            self.session
                .send_msg(&MessageType::TownScenePlayerAppearancePush(push), 0, session_id);
        }
        for (session_id, entries) in pushes.player_states {
            self.session.send_msg(
                &MessageType::TownScenePlayerStatesPush(ScenePlayerStatesPush { players: entries }),
                0,
                session_id,
            );
        }
    }
}

/// 返回当前 Unix 毫秒时间戳。
fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 启动固定频率广播 tick 任务。
fn spawn_tick_task(weak: std::sync::Weak<TownShared>, config: SyncConfig) {
    let period = std::time::Duration::from_millis(config.tick_ms.max(10));
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let Some(shared) = weak.upgrade() else {
                break;
            };
            shared.tick();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // 测试位置以像素表示，按定点缩放（1/1000 像素）转换为协议定点单位。
    fn px(v: i32) -> i32 {
        v * 1000
    }

    fn vec3(x: i32, y: i32, z: i32) -> TownVec3 {
        TownVec3 {
            x: px(x),
            y: px(y),
            z: px(z),
        }
    }

    fn mov(pos: TownVec3) -> AcceptedMovement {
        AcceptedMovement {
            pos,
            facing: 1,
            moving: false,
            vel: TownVel::default(),
            state: "idle".to_string(),
        }
    }

    fn appearance(class_id: i32, name: &str) -> TownAppearance {
        TownAppearance {
            class_id,
            name: name.to_string(),
            level: 1,
            fashions: vec![],
            pet_id: 0,
            base_skins: vec![],
            armament_id: 0,
        }
    }

    fn enter_map(
        session_id: u32,
        player_id: i64,
        map_id: u32,
        pos: TownVec3,
    ) -> TownEnterSceneReq {
        TownEnterSceneReq {
            session_id,
            player_id,
            r#type: SceneEnterType::Map as i32,
            map_id,
            owner_uid: 0,
            movement: Some(TownMovement {
                pos: Some(pos),
                facing: 1,
                moving: false,
                vel: Some(TownVel::default()),
                state: "idle".to_string(),
            }),
            appearance: Some(appearance(1, "p")),
        }
    }

    fn report(pos: TownVec3, seq: u32) -> TownMoveReport {
        report_with_ack(pos, seq, 0)
    }

    fn report_with_ack(pos: TownVec3, seq: u32, ack_correction_seq: u32) -> TownMoveReport {
        TownMoveReport {
            client_seq: seq,
            client_time_ms: 0,
            ack_correction_seq,
            movement: Some(TownMovement {
                pos: Some(pos),
                facing: 1,
                moving: true,
                vel: Some(TownVel::default()),
                state: "walk".to_string(),
            }),
        }
    }

    /// 读取玩家当前权威位置（定点）。
    fn player_pos(manager: &SceneManager, player_id: i64) -> TownVec3 {
        let scene_id = *manager.player_scenes.get(&player_id).unwrap();
        let scene = manager.scenes.get(&scene_id).unwrap();
        scene.players.get(&player_id).unwrap().movement.pos.clone()
    }

    /// 覆盖玩家的上次上报时刻（用于注入历史时间）。
    fn set_player_last_report(manager: &mut SceneManager, player_id: i64, at: Instant) {
        let scene_id = *manager.player_scenes.get(&player_id).unwrap();
        let scene = manager.scenes.get_mut(&scene_id).unwrap();
        scene.players.get_mut(&player_id).unwrap().last_report_at = at;
    }

    /// 校验测试用配置：max_speed=1000, tolerance=1.0, burst=0.5 → 预算上限 500px、恢复 1000px/s。
    fn validate_config() -> SyncConfig {
        SyncConfig {
            max_speed: 1000.0,
            tolerance: 1.0,
            burst_seconds: 0.5,
            aoi_radius: 0.0,
            tick_ms: 100,
            bounds: None,
        }
    }

    // ── 校验：预算 clamp 与校正 ──────────────────────────────

    #[test]
    fn validate_clamps_excessive_displacement() {
        let config = validate_config();
        let last = mov(vec3(0, 0, 0));
        // 预算 1000px；上报 2000px 应被 clamp 回 1000px 处，预算清零。
        let reported = mov(vec3(2000, 0, 0));
        let mut budget = 1000.0;
        let result = validate_movement(&last, &reported, &mut budget, &config);
        assert!(result.corrected);
        assert_eq!(result.accepted.pos.x, px(1000));
        assert_eq!(result.accepted.pos.y, 0);
        assert_eq!(budget, 0.0);
    }

    #[test]
    fn validate_accepts_within_budget() {
        let config = validate_config();
        let last = mov(vec3(0, 0, 0));
        let reported = mov(vec3(500, 0, 0));
        let mut budget = 1000.0;
        let result = validate_movement(&last, &reported, &mut budget, &config);
        assert!(!result.corrected);
        assert_eq!(result.accepted.pos.x, px(500));
        assert_eq!(budget, 500.0);
    }

    #[test]
    fn validate_normalizes_bad_facing() {
        let config = SyncConfig::default();
        let last = mov(vec3(0, 0, 0));
        let mut reported = mov(vec3(0, 0, 0));
        reported.facing = 5;
        let mut budget = 100.0;
        let result = validate_movement(&last, &reported, &mut budget, &config);
        assert!(result.corrected);
        assert_eq!(result.accepted.facing, 0);
    }

    #[test]
    fn validate_clamps_to_bounds() {
        let config = SyncConfig {
            bounds: Some(SceneBounds {
                min_x: px(0),
                min_y: px(0),
                min_z: px(0),
                max_x: px(100),
                max_y: px(100),
                max_z: px(100),
            }),
            ..SyncConfig::default()
        };
        let last = mov(vec3(0, 0, 0));
        let reported = mov(vec3(500, 0, 0));
        // 预算充足（1000px）时不触发速度 clamp，但超出 max_x=100px 边界，应被 clamp。
        let mut budget = 1000.0;
        let result = validate_movement(&last, &reported, &mut budget, &config);
        assert!(result.corrected);
        assert_eq!(result.accepted.pos.x, px(100));
    }

    // ── 移动上报：位移预算（token bucket） ───────────────────

    #[test]
    fn move_report_burst_absorbs_bunched_reports() {
        let mut manager = SceneManager::new(validate_config());
        manager.enter_scene(7, enter_map(10, 100, 1001, vec3(0, 0, 0)));

        let base = Instant::now();
        // 注入 1 秒历史时间，使预算累积到上限（500px）。
        set_player_last_report(&mut manager, 100, base - std::time::Duration::from_secs(1));

        // 两次诚实上报几乎同时到达（~0 间隔）：预算吸收突发，均不应校正。
        let r1 = manager.move_report_at(10, report(vec3(55, 0, 0), 1), base);
        assert!(r1.correction.is_none());
        let r2 = manager.move_report_at(
            10,
            report(vec3(110, 0, 0), 2),
            base + std::time::Duration::from_millis(1),
        );
        assert!(r2.correction.is_none());
        assert_eq!(player_pos(&manager, 100), vec3(110, 0, 0));
    }

    #[test]
    fn move_report_clamps_teleport_jump_to_budget() {
        let mut manager = SceneManager::new(validate_config());
        manager.enter_scene(7, enter_map(10, 100, 1001, vec3(0, 0, 0)));

        // 进入场景预算置满（500px）；传送级跳跃应被 clamp 到预算上限并校正。
        let result = manager.move_report(10, report(vec3(10000, 0, 0), 1));
        assert!(result.correction.is_some());
        let correction = result.correction.unwrap();
        assert_eq!(correction.client_seq, 1);
        assert_eq!(correction.pos, Some(vec3(500, 0, 0)));
        assert_eq!(player_pos(&manager, 100), vec3(500, 0, 0));
    }

    // ── 校正确认：丢弃在途上报 ───────────────────────────────

    #[test]
    fn move_report_drops_stale_ack_and_processes_acked() {
        let mut manager = SceneManager::new(validate_config());
        manager.enter_scene(7, enter_map(10, 100, 1001, vec3(0, 0, 0)));

        let base = Instant::now();
        // 传送级跳跃 → 校正到预算上限（500px），pending = seq 1。
        let r1 = manager.move_report_at(10, report(vec3(10000, 0, 0), 1), base);
        assert!(r1.correction.is_some());
        assert_eq!(player_pos(&manager, 100), vec3(500, 0, 0));

        // 在途报告（ack=0 < pending=1）应被丢弃：位置不变、无新校正、预算时间不推进。
        let r2 = manager.move_report_at(
            10,
            report(vec3(9000, 0, 0), 2),
            base + std::time::Duration::from_millis(10),
        );
        assert!(r2.correction.is_none());
        assert_eq!(player_pos(&manager, 100), vec3(500, 0, 0));

        // 1 秒后 ack>=pending 的报告被正常处理（预算已恢复，小位移直接接受）。
        let r3 = manager.move_report_at(
            10,
            report_with_ack(vec3(600, 0, 0), 3, 1),
            base + std::time::Duration::from_secs(1),
        );
        assert!(r3.correction.is_none());
        assert_eq!(player_pos(&manager, 100), vec3(600, 0, 0));
    }

    // ── send-on-change 批处理 ───────────────────────────────

    #[test]
    fn tick_batches_only_changed_players() {
        let mut manager = SceneManager::new(SyncConfig::default());
        manager.enter_scene(7, enter_map(10, 100, 1001, vec3(0, 0, 0)));
        manager.enter_scene(7, enter_map(11, 101, 1001, vec3(0, 0, 0)));

        // 首次 tick：两者都刚进入（dirty），应各自广播给对方。
        let pushes = manager.tick(1_000);
        // 玩家 100 与 101 各自收到一条 states 批次。
        assert_eq!(pushes.player_states.len(), 2);

        // 第二次 tick：无人变化，无广播。
        let pushes = manager.tick(2_000);
        assert!(pushes.player_states.is_empty());
    }

    // ── AOI 进入/离开 ───────────────────────────────────────

    #[test]
    fn aoi_move_into_radius_sends_enter_push() {
        let mut manager = SceneManager::new(SyncConfig {
            aoi_radius: 10.0,
            ..SyncConfig::default()
        });
        manager.enter_scene(7, enter_map(10, 100, 1001, vec3(0, 0, 0)));
        // 101 出生在 20px 外（>10px），初始不在 100 的 AOI 内。
        manager.enter_scene(7, enter_map(11, 101, 1001, vec3(20, 0, 0)));

        let result = manager.move_report(11, report(vec3(1, 0, 0), 2));
        assert!(
            result.correction.is_none(),
            "move should be within speed limit"
        );
        // 101 进入 100 的 AOI：100 收到 101 的进入推送。
        assert!(result
            .pushes
            .player_enter
            .iter()
            .any(|(sid, _)| *sid == 10));
    }

    #[test]
    fn aoi_move_out_of_radius_sends_leave_push() {
        let mut manager = SceneManager::new(SyncConfig {
            aoi_radius: 10.0,
            ..SyncConfig::default()
        });
        manager.enter_scene(7, enter_map(10, 100, 1001, vec3(0, 0, 0)));
        // 101 出生在 100 附近（AOI 内）。
        manager.enter_scene(7, enter_map(11, 101, 1001, vec3(1, 0, 0)));

        let result = manager.move_report(11, report(vec3(20, 0, 0), 2));
        // 101 离开 100 的 AOI：100 收到 101 的离开推送。
        assert!(result
            .pushes
            .player_leave
            .iter()
            .any(|(sid, pid)| *sid == 10 && *pid == 101));
    }

    // ── 传送门换图：进入新场景重置校验基线 ────────────────────

    #[test]
    fn enter_scene_teleport_resets_validation_baseline() {
        let mut manager = SceneManager::new(validate_config());
        // 玩家先进入场景 A，出生在原点。
        manager.enter_scene(7, enter_map(10, 100, 1001, vec3(0, 0, 0)));

        // 传送到场景 B，出生点远在 (10000, 0, 0)。
        let result = manager.enter_scene(7, enter_map(10, 100, 1002, vec3(10000, 0, 0)));
        assert_eq!(result.response.code, 0);
        assert_eq!(result.response.scene.as_ref().map(|s| s.map_id), Some(1002));

        // 上报出生点附近的位置：若基线未重置到出生点（仍停留在旧场景原点），10500px
        // 会因超预算被校正；重置后预算置满（500px），500px 位移在预算内，不应校正。
        let move_result = manager.move_report(10, report(vec3(10500, 0, 0), 1));
        assert!(
            move_result.correction.is_none(),
            "teleport must reset the validation baseline so nearby movement is accepted"
        );

        // 场景 A 已移除该玩家。
        if let Some(scene_a_id) = manager.scenes_by_key.get(&SceneKey::Map(1001)).copied() {
            assert!(!manager.scenes.get(&scene_a_id).unwrap().players.contains_key(&100));
        }
    }
}
