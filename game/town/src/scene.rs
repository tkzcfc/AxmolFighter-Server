use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use backend_framework::session::BackendSession;
use protocol::game::{
    PlayerState, SceneEnterType, SceneInfo, ScenePlayerEnterPush, ScenePlayerLeavePush,
    ScenePlayerStatePush, TownEnterSceneReq, TownEnterSceneResp, TownLeaveSceneReq,
    TownLeaveSceneResp, TownPlayerStatePush,
};
use protocol::message_map::MessageType;
use tracing::warn;

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

#[derive(Clone)]
struct Player {
    session_id: u32,
    state: PlayerState,
}

struct Scene {
    #[allow(dead_code)]
    scene_id: u64,
    info: SceneInfo,
    players: HashMap<i64, Player>,
}

#[derive(Default)]
struct PushBatch {
    player_enter: Vec<(u32, PlayerState)>,
    player_leave: Vec<(u32, i64)>,
    player_state: Vec<(u32, PlayerState)>,
}

struct EnterSceneResult {
    response: TownEnterSceneResp,
    pushes: PushBatch,
}

struct LeaveSceneResult {
    response: TownLeaveSceneResp,
    pushes: PushBatch,
}

#[derive(Default)]
struct SceneManager {
    next_scene_id: u64,
    scenes_by_key: HashMap<SceneKey, u64>,
    scenes: HashMap<u64, Scene>,
    player_scenes: HashMap<i64, u64>,
    session_players: HashMap<u32, i64>,
}

impl SceneManager {
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
        let mut state = req.state.unwrap_or_default();
        state.player_id = req.player_id;

        if self.player_scenes.get(&req.player_id).copied() == Some(scene_id) {
            return self.update_player_in_scene(instance_id, scene_id, req.session_id, state);
        }

        let mut pushes = PushBatch::default();
        if let Some(old_player_id) = self.session_players.get(&req.session_id).copied()
            && old_player_id != req.player_id
        {
            self.remove_player(old_player_id, &mut pushes);
        }
        self.remove_player(req.player_id, &mut pushes);

        let scene = self
            .scenes
            .get_mut(&scene_id)
            .expect("scene must exist after ensure_scene");
        for player in scene.players.values() {
            pushes.player_enter.push((player.session_id, state.clone()));
        }

        scene.players.insert(
            req.player_id,
            Player {
                session_id: req.session_id,
                state,
            },
        );
        self.player_scenes.insert(req.player_id, scene_id);
        self.session_players.insert(req.session_id, req.player_id);

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

    fn update_state(&mut self, session_id: u32, push: TownPlayerStatePush) -> PushBatch {
        let mut pushes = PushBatch::default();
        let Some(player_id) = self.session_players.get(&session_id).copied() else {
            warn!("ignored town state push for unknown session {}", session_id);
            return pushes;
        };
        let Some(scene_id) = self.player_scenes.get(&player_id).copied() else {
            warn!(
                "ignored town state push without scene session {}",
                session_id
            );
            return pushes;
        };

        let mut state = push.state.unwrap_or_default();
        state.player_id = player_id;
        let Some(scene) = self.scenes.get_mut(&scene_id) else {
            return pushes;
        };
        let Some(player) = scene.players.get_mut(&player_id) else {
            return pushes;
        };
        player.state = state.clone();

        for other in scene.players.values() {
            if other.session_id != session_id {
                pushes.player_state.push((other.session_id, state.clone()));
            }
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
        state: PlayerState,
    ) -> EnterSceneResult {
        let player_id = state.player_id;
        let mut pushes = PushBatch::default();
        let scene = self
            .scenes
            .get_mut(&scene_id)
            .expect("scene must exist for player_scenes entry");
        if let Some(player) = scene.players.get_mut(&player_id) {
            self.session_players.remove(&player.session_id);
            player.session_id = session_id;
            player.state = state.clone();
            self.session_players.insert(session_id, player_id);
        }

        for other in scene.players.values() {
            if other.session_id != session_id {
                pushes.player_state.push((other.session_id, state.clone()));
            }
        }

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
        for other in scene.players.values() {
            pushes.player_leave.push((other.session_id, player_id));
        }
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
                .values()
                .map(|player| player.state.clone())
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
    pub(crate) fn new(session: Arc<BackendSession>, instance_id: u32) -> Arc<Self> {
        Arc::new(Self {
            session,
            instance_id,
            manager: Mutex::new(SceneManager::default()),
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

    pub(crate) fn update_state(&self, session_id: u32, push: TownPlayerStatePush) {
        let pushes = self.manager.lock().unwrap().update_state(session_id, push);
        self.send_pushes(pushes);
    }

    pub(crate) fn clear(&self) {
        self.manager.lock().unwrap().clear();
    }

    fn send_pushes(&self, pushes: PushBatch) {
        for (session_id, state) in pushes.player_enter {
            self.session.send_msg(
                &MessageType::GameScenePlayerEnterPush(ScenePlayerEnterPush { state: Some(state) }),
                0,
                session_id,
            );
        }
        for (session_id, player_id) in pushes.player_leave {
            self.session.send_msg(
                &MessageType::GameScenePlayerLeavePush(ScenePlayerLeavePush { player_id }),
                0,
                session_id,
            );
        }
        for (session_id, state) in pushes.player_state {
            self.session.send_msg(
                &MessageType::GameScenePlayerStatePush(ScenePlayerStatePush { state: Some(state) }),
                0,
                session_id,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(player_id: i64, x: f32) -> PlayerState {
        PlayerState {
            player_id,
            pos_x: x,
            pos_y: 2.0,
            hp: 100,
            state: "idle".to_string(),
        }
    }

    fn enter_map(session_id: u32, player_id: i64, map_id: u32, x: f32) -> TownEnterSceneReq {
        TownEnterSceneReq {
            session_id,
            player_id,
            r#type: SceneEnterType::Map as i32,
            map_id,
            owner_uid: 0,
            state: Some(state(player_id, x)),
        }
    }

    #[test]
    fn player_enters_map_and_gets_self_snapshot() {
        let mut manager = SceneManager::default();
        let result = manager.enter_scene(7, enter_map(10, 100, 1001, 1.0));

        assert_eq!(result.response.code, 0);
        assert_eq!(result.response.town_instance_id, 7);
        assert_eq!(result.response.players.len(), 1);
        assert_eq!(result.response.players[0].player_id, 100);
        assert!(result.pushes.player_enter.is_empty());
    }

    #[test]
    fn second_player_entering_same_map_notifies_existing_player() {
        let mut manager = SceneManager::default();
        manager.enter_scene(7, enter_map(10, 100, 1001, 1.0));
        let result = manager.enter_scene(7, enter_map(11, 101, 1001, 3.0));

        assert_eq!(result.response.players.len(), 2);
        assert_eq!(result.pushes.player_enter.len(), 1);
        assert_eq!(result.pushes.player_enter[0].0, 10);
        assert_eq!(result.pushes.player_enter[0].1.player_id, 101);
    }

    #[test]
    fn player_switching_scene_notifies_old_scene_and_new_scene() {
        let mut manager = SceneManager::default();
        manager.enter_scene(7, enter_map(10, 100, 1001, 1.0));
        manager.enter_scene(7, enter_map(11, 101, 1001, 3.0));
        let result = manager.enter_scene(7, enter_map(10, 100, 2002, 5.0));

        assert_eq!(result.response.players.len(), 1);
        assert_eq!(result.pushes.player_leave.len(), 1);
        assert_eq!(result.pushes.player_leave[0], (11, 100));
        assert!(result.pushes.player_enter.is_empty());
    }

    #[test]
    fn state_sync_broadcasts_to_same_scene_and_uses_server_player_id() {
        let mut manager = SceneManager::default();
        manager.enter_scene(7, enter_map(10, 100, 1001, 1.0));
        manager.enter_scene(7, enter_map(11, 101, 1001, 3.0));

        let pushes = manager.update_state(
            10,
            TownPlayerStatePush {
                state: Some(state(999, 9.0)),
            },
        );

        assert_eq!(pushes.player_state.len(), 1);
        assert_eq!(pushes.player_state[0].0, 11);
        assert_eq!(pushes.player_state[0].1.player_id, 100);
        assert_eq!(pushes.player_state[0].1.pos_x, 9.0);
    }

    #[test]
    fn session_stop_leaves_scene_and_broadcasts_leave() {
        let mut manager = SceneManager::default();
        manager.enter_scene(7, enter_map(10, 100, 1001, 1.0));
        manager.enter_scene(7, enter_map(11, 101, 1001, 3.0));

        let pushes = manager.leave_session(10);

        assert_eq!(pushes.player_leave, vec![(11, 100)]);
        assert_eq!(manager.session_players.get(&10), None);
        assert_eq!(manager.player_scenes.get(&100), None);
    }
}
