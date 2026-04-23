use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use std::io::Cursor;
use std::time::{Duration, Instant};

use bson::Document;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{RwLock, mpsc, watch};
use tokio::time::{MissedTickBehavior, interval, interval_at, sleep, sleep_until};

use crate::auth;
use crate::constants::{fishing, movement, network, protocol as ids, timing, tutorial};
use crate::logging::{Direction, Logger, TransportKind};
use crate::lua_runtime::{self, LuaScriptHandle};
use crate::models::{
    AuthInput, InventoryItem, LuaCollectableSnapshot, LuaGrowingTileSnapshot,
    LuaScriptStatusSnapshot, LuaTileSnapshot, LuaWorldObjectsSnapshot, LuaWorldSnapshot,
    LuaWorldSpawnSnapshot, LuaWorldTilesSnapshot, MinimapSnapshot, PlayerPosition,
    RemotePlayerSnapshot, SessionSnapshot, SessionStatus, TileCount, WorldSnapshot,
};
use crate::net;
use crate::pathfinding::astar;
use crate::protocol;
use crate::world;

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);
static RUNTIME_COUNTER: AtomicU64 = AtomicU64::new(1);
static BLOCK_TYPES: OnceLock<HashMap<u16, BlockTypeInfo>> = OnceLock::new();

#[derive(Debug, Clone, serde::Deserialize)]
struct BlockTypeInfo {
    id: u16,
    name: String,
    #[serde(rename = "type")]
    inventory_type: u16,
    #[serde(rename = "typeName")]
    type_name: String,
}

#[derive(Debug, Clone)]
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, Arc<BotSession>>>>,
    lua_scripts: Arc<RwLock<HashMap<String, LuaScriptHandle>>>,
    logger: Logger,
}

impl SessionManager {
    pub fn new(logger: Logger) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            lua_scripts: Arc::new(RwLock::new(HashMap::new())),
            logger,
        }
    }

    pub async fn create_session(&self, auth: AuthInput) -> Arc<BotSession> {
        let id = format!(
            "session-{}",
            SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let session = BotSession::new(id.clone(), auth, self.logger.clone()).await;
        self.sessions.write().await.insert(id, session.clone());
        session
    }

    pub async fn get_session(&self, id: &str) -> Option<Arc<BotSession>> {
        self.sessions.read().await.get(id).cloned()
    }

    pub async fn list_snapshots(&self) -> Vec<SessionSnapshot> {
        let sessions = self.sessions.read().await;
        let mut items = Vec::with_capacity(sessions.len());
        for session in sessions.values() {
            items.push(session.snapshot().await);
        }
        items
    }

    pub async fn start_lua_script(
        &self,
        session_id: &str,
        source: String,
    ) -> Result<LuaScriptStatusSnapshot, String> {
        let session = self
            .get_session(session_id)
            .await
            .ok_or_else(|| "session not found".to_string())?;
        self.stop_lua_script(session_id).await?;
        let handle = lua_runtime::spawn_script(session, source, self.logger.clone());
        let status = handle.status.read().await.clone();
        self.lua_scripts
            .write()
            .await
            .insert(session_id.to_string(), handle);
        Ok(status)
    }

    pub async fn stop_lua_script(
        &self,
        session_id: &str,
    ) -> Result<LuaScriptStatusSnapshot, String> {
        if let Some(handle) = self.lua_scripts.write().await.remove(session_id) {
            handle.cancel.store(true, AtomicOrdering::Relaxed);
            handle.task.abort();
            Ok(handle.status.read().await.clone())
        } else {
            Ok(lua_runtime::idle_status())
        }
    }

    pub async fn lua_script_status(
        &self,
        session_id: &str,
    ) -> Result<LuaScriptStatusSnapshot, String> {
        self.get_session(session_id)
            .await
            .ok_or_else(|| "session not found".to_string())?;
        if let Some(handle) = self.lua_scripts.read().await.get(session_id) {
            Ok(handle.status.read().await.clone())
        } else {
            Ok(lua_runtime::idle_status())
        }
    }
}

#[derive(Debug)]
pub struct BotSession {
    id: String,
    auth: AuthInput,
    state: Arc<RwLock<SessionState>>,
    controller_tx: mpsc::Sender<ControllerEvent>,
    logger: Logger,
}

impl BotSession {
    pub(crate) fn id_string(&self) -> String {
        self.id.clone()
    }

    async fn new(id: String, auth: AuthInput, logger: Logger) -> Arc<Self> {
        let (controller_tx, controller_rx) = mpsc::channel(512);
        let device_id = auth.device_id();
        let state = Arc::new(RwLock::new(SessionState {
            status: SessionStatus::Idle,
            device_id: if device_id.is_empty() {
                network::DEFAULT_DEVICE_ID.to_string()
            } else {
                device_id
            },
            current_host: net::default_host(),
            current_port: net::default_port(),
            current_world: None,
            pending_world: None,
            pending_world_is_instance: false,
            username: None,
            user_id: None,
            world: None,
            world_foreground_tiles: Vec::new(),
            world_background_tiles: Vec::new(),
            world_water_tiles: Vec::new(),
            world_wiring_tiles: Vec::new(),
            current_outbound_tx: None,
            growing_tiles: HashMap::new(),
            player_position: PlayerPosition {
                map_x: None,
                map_y: None,
                world_x: None,
                world_y: None,
            },
            other_players: HashMap::new(),
            inventory: Vec::new(),
            collectables: HashMap::new(),
            last_error: None,
            awaiting_ready: false,
            tutorial_spawn_pod_confirmed: false,
            tutorial_automation_running: false,
            fishing: FishingAutomationState::default(),
            ping_ms: None,
        }));

        let session = Arc::new(Self {
            id,
            auth,
            state,
            controller_tx,
            logger,
        });

        let cloned = session.clone();
        tokio::spawn(async move {
            cloned.run_controller(controller_rx).await;
        });

        session
    }

    pub async fn snapshot(&self) -> SessionSnapshot {
        let state = self.state.read().await;
        SessionSnapshot {
            id: self.id.clone(),
            status: state.status.clone(),
            device_id: state.device_id.clone(),
            current_host: state.current_host.clone(),
            current_port: state.current_port,
            current_world: state.current_world.clone(),
            pending_world: state.pending_world.clone(),
            username: state.username.clone(),
            user_id: state.user_id.clone(),
            world: state.world.clone(),
            player_position: state.player_position.clone(),
            inventory: state
                .inventory
                .iter()
                .map(|e| InventoryItem {
                    block_id: e.block_id,
                    inventory_type: e.inventory_type,
                    amount: e.amount,
                })
                .collect(),
            last_error: state.last_error.clone(),
            ping_ms: state.ping_ms,
        }
    }

    pub async fn connect(&self) -> Result<(), String> {
        self.send_command(SessionCommand::Connect).await
    }

    pub async fn join_world(&self, world: String, instance: bool) -> Result<(), String> {
        self.send_command(SessionCommand::JoinWorld { world, instance }).await
    }

    pub async fn leave_world(&self) -> Result<(), String> {
        self.send_command(SessionCommand::LeaveWorld).await
    }

    pub async fn disconnect(&self) -> Result<(), String> {
        self.send_command(SessionCommand::Disconnect).await
    }

    pub async fn reconnect(&self) -> Result<(), String> {
        self.send_command(SessionCommand::Disconnect).await?;
        self.send_command(SessionCommand::Connect).await
    }

    pub async fn minimap_snapshot(&self) -> Result<MinimapSnapshot, String> {
        let state = self.state.read().await;
        let world = state
            .world
            .clone()
            .ok_or_else(|| "no world loaded yet".to_string())?;
        if state.world_foreground_tiles.is_empty() {
            return Err("no world tiles loaded yet".to_string());
        }
        Ok(MinimapSnapshot {
            width: world.width,
            height: world.height,
            foreground_tiles: state.world_foreground_tiles.clone(),
            background_tiles: state.world_background_tiles.clone(),
            water_tiles: state.world_water_tiles.clone(),
            wiring_tiles: state.world_wiring_tiles.clone(),
            player_position: state.player_position.clone(),
            other_players: state
                .other_players
                .iter()
                .map(|(user_id, position)| RemotePlayerSnapshot {
                    user_id: user_id.clone(),
                    position: position.clone(),
                })
                .collect(),
        })
    }

    pub async fn automate_tutorial(&self) -> Result<String, String> {
        self.send_command(SessionCommand::AutomateTutorial).await?;
        Ok("Tutorial automation queued.".to_string())
    }

    pub async fn is_tile_ready_to_harvest(&self, map_x: i32, map_y: i32) -> Result<bool, String> {
        let state = self.state.read().await;
        is_tile_ready_to_harvest_at(&state, map_x, map_y, protocol::csharp_ticks())
    }

    pub async fn queue_drop_item(
        &self,
        block_id: i32,
        amount: i32,
    ) -> Result<String, String> {
        if amount <= 0 {
            return Err("amount must be greater than 0".to_string());
        }
        if let Some(type_name) = block_type_name_for(block_id as u16) {
            if type_name == "WearableItem" {
                let name = block_name_for(block_id as u16)
                    .unwrap_or_else(|| format!("block {block_id}"));
                return Err(format!(
                    "{name} is a {type_name}; server rejects dropping this category"
                ));
            }
        }
        self.send_command(SessionCommand::DropItem {
            block_id,
            amount,
        })
        .await?;
        Ok(format!("drop queued: {amount}x block {block_id}"))
    }

    pub async fn queue_wear_item(&self, block_id: i32, equip: bool) -> Result<String, String> {
        self.send_command(SessionCommand::WearItem { block_id, equip })
            .await?;
        let action = if equip { "equip" } else { "unequip" };
        Ok(format!("{action} queued for block {block_id}"))
    }

    pub async fn queue_punch(&self, offset_x: i32, offset_y: i32) -> Result<String, String> {
        self.send_command(SessionCommand::Punch { offset_x, offset_y })
            .await?;
        Ok(format!("punch queued at offset ({offset_x}, {offset_y})"))
    }

    pub async fn queue_place(
        &self,
        offset_x: i32,
        offset_y: i32,
        block_id: i32,
    ) -> Result<String, String> {
        self.send_command(SessionCommand::Place {
            offset_x,
            offset_y,
            block_id,
        })
        .await?;
        Ok(format!(
            "place queued for block {block_id} at offset ({offset_x}, {offset_y})"
        ))
    }

    pub async fn queue_move_direction(&self, direction: &str) -> Result<String, String> {
        let normalized = direction.trim().to_ascii_lowercase();
        if !matches!(normalized.as_str(), "left" | "right" | "up" | "down") {
            return Err("direction must be left, right, up, or down".to_string());
        }
        self.send_command(SessionCommand::ManualMove {
            direction: normalized.clone(),
        })
        .await?;
        Ok(format!("queued 1 step {normalized}"))
    }

    pub async fn queue_start_fishing(&self, direction: &str, bait: &str) -> Result<String, String> {
        let normalized = direction.trim().to_ascii_lowercase();
        if !matches!(normalized.as_str(), "left" | "right") {
            return Err("fishing direction must be left or right".to_string());
        }
        let bait = bait.trim();
        if bait.is_empty() {
            return Err("bait is required".to_string());
        }
        self.send_command(SessionCommand::StartFishing {
            direction: normalized.clone(),
            bait: bait.to_string(),
        })
        .await?;
        Ok(format!(
            "auto-fishing queued to the {normalized} using {bait}"
        ))
    }

    pub async fn queue_stop_fishing(&self) -> Result<String, String> {
        self.send_command(SessionCommand::StopFishing).await?;
        Ok("fishing stop queued".to_string())
    }

    pub async fn queue_talk(&self, message: &str) -> Result<String, String> {
        let message = message.trim();
        if message.is_empty() {
            return Err("message is required".to_string());
        }
        self.send_command(SessionCommand::Talk {
            message: message.to_string(),
        })
        .await?;
        Ok("chat message queued".to_string())
    }

    pub async fn queue_start_spam(&self, message: &str, delay_ms: u64) -> Result<String, String> {
        let message = message.trim();
        if message.is_empty() {
            return Err("message is required".to_string());
        }
        if delay_ms < 250 {
            return Err("spam delay must be at least 250ms".to_string());
        }
        if delay_ms > 3_600_000 {
            return Err("spam delay must be at most 3600000ms".to_string());
        }
        self.send_command(SessionCommand::StartSpam {
            message: message.to_string(),
            delay_ms,
        })
        .await?;
        Ok(format!("spam loop queued at {delay_ms}ms"))
    }

    pub async fn queue_stop_spam(&self) -> Result<String, String> {
        self.send_command(SessionCommand::StopSpam).await?;
        Ok("spam stop queued".to_string())
    }

    pub(crate) async fn walk(
        &self,
        offset_x: i32,
        offset_y: i32,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        ensure_not_cancelled(cancel)?;
        let (target_x, target_y) = {
            let state = self.state.read().await;
            (
                state
                    .player_position
                    .map_x
                    .ok_or_else(|| "player map x is not known yet".to_string())?
                    .round() as i32
                    + offset_x,
                state
                    .player_position
                    .map_y
                    .ok_or_else(|| "player map y is not known yet".to_string())?
                    .round() as i32
                    + offset_y,
            )
        };
        self.walk_to(target_x, target_y, cancel).await
    }

    pub(crate) async fn walk_to(
        &self,
        map_x: i32,
        map_y: i32,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        ensure_not_cancelled(cancel)?;
        let outbound_tx = self
            .state
            .read()
            .await
            .current_outbound_tx
            .clone()
            .ok_or_else(|| "connect the session before walking".to_string())?;
        walk_to_map_cancellable(&self.state, &outbound_tx, map_x, map_y, cancel).await
    }

    pub(crate) async fn find_path(
        &self,
        map_x: i32,
        map_y: i32,
    ) -> Result<Vec<(i32, i32)>, String> {
        let (start_x, start_y) = {
            let state = self.state.read().await;
            (
                state
                    .player_position
                    .map_x
                    .ok_or_else(|| "player map x is not known yet".to_string())?
                    .round() as i32,
                state
                    .player_position
                    .map_y
                    .ok_or_else(|| "player map y is not known yet".to_string())?
                    .round() as i32,
            )
        };

        Ok(
            planned_path(&self.state, (start_x, start_y), (map_x, map_y))
                .await
                .unwrap_or_else(|| fallback_straight_line_path((start_x, start_y), (map_x, map_y))),
        )
    }

    pub(crate) async fn punch(
        &self,
        offset_x: i32,
        offset_y: i32,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        ensure_not_cancelled(cancel)?;
        let outbound_tx = self
            .state
            .read()
            .await
            .current_outbound_tx
            .clone()
            .ok_or_else(|| "connect the session before punching".to_string())?;
        manual_punch(
            &self.id,
            &self.logger,
            &self.state,
            &outbound_tx,
            offset_x,
            offset_y,
        )
        .await
    }

    pub(crate) async fn place(
        &self,
        offset_x: i32,
        offset_y: i32,
        block_id: i32,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        ensure_not_cancelled(cancel)?;
        let outbound_tx = self
            .state
            .read()
            .await
            .current_outbound_tx
            .clone()
            .ok_or_else(|| "connect the session before placing blocks".to_string())?;
        manual_place(
            &self.id,
            &self.logger,
            &self.state,
            &outbound_tx,
            offset_x,
            offset_y,
            block_id,
        )
        .await
    }

    pub(crate) async fn wear(
        &self,
        block_id: i32,
        equip: bool,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        ensure_not_cancelled(cancel)?;
        let outbound_tx = self
            .state
            .read()
            .await
            .current_outbound_tx
            .clone()
            .ok_or_else(|| "connect the session before wearing items".to_string())?;
        let packet = if equip {
            protocol::make_wear_item(block_id)
        } else {
            protocol::make_unwear_item(block_id)
        };
        send_doc(&outbound_tx, packet).await
    }

    pub(crate) async fn talk(&self, message: &str, cancel: &AtomicBool) -> Result<(), String> {
        ensure_not_cancelled(cancel)?;
        let outbound_tx = self
            .state
            .read()
            .await
            .current_outbound_tx
            .clone()
            .ok_or_else(|| "connect the session before sending chat".to_string())?;
        send_world_chat(&self.id, &self.logger, &outbound_tx, message).await
    }

    pub(crate) async fn collect(&self, cancel: &AtomicBool) -> Result<(), String> {
        ensure_not_cancelled(cancel)?;
        let outbound_tx = self
            .state
            .read()
            .await
            .current_outbound_tx
            .clone()
            .ok_or_else(|| "connect the session before collecting".to_string())?;
        collect_all_visible_collectables_cancellable(&self.state, &outbound_tx, cancel).await
    }

    pub(crate) async fn warp(&self, world: &str, cancel: &AtomicBool) -> Result<(), String> {
        self.warp_inner(world, false, cancel).await
    }

    pub(crate) async fn warp_instance(
        &self,
        world: &str,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        self.warp_inner(world, true, cancel).await
    }

    async fn warp_inner(
        &self,
        world: &str,
        instance: bool,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        ensure_not_cancelled(cancel)?;
        let world = world.trim().to_uppercase();
        if world.is_empty() {
            return Err("world is required".to_string());
        }
        let outbound_tx = self
            .state
            .read()
            .await
            .current_outbound_tx
            .clone()
            .ok_or_else(|| "connect the session before warping".to_string())?;
        ensure_world_cancellable(
            &self.id,
            &self.logger,
            &self.state,
            &self.controller_tx,
            &outbound_tx,
            &world,
            instance,
            cancel,
        )
        .await
    }

    pub(crate) async fn send_packet(
        &self,
        packet: Document,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        ensure_not_cancelled(cancel)?;
        let outbound_tx = self
            .state
            .read()
            .await
            .current_outbound_tx
            .clone()
            .ok_or_else(|| "connect the session before sending packets".to_string())?;
        send_doc(&outbound_tx, packet).await
    }

    pub(crate) async fn position(&self) -> PlayerPosition {
        self.state.read().await.player_position.clone()
    }

    pub(crate) async fn current_world(&self) -> Option<String> {
        self.state.read().await.current_world.clone()
    }

    pub(crate) async fn status(&self) -> SessionStatus {
        self.state.read().await.status.clone()
    }

    pub(crate) async fn is_in_world(&self) -> bool {
        self.state.read().await.status == SessionStatus::InWorld
    }

    pub(crate) async fn inventory_count(&self, block_id: u16) -> u32 {
        self.state
            .read()
            .await
            .inventory
            .iter()
            .filter(|entry| entry.block_id == block_id)
            .map(|entry| entry.amount as u32)
            .sum()
    }

    pub(crate) async fn collectables(&self) -> Vec<LuaCollectableSnapshot> {
        let mut collectables = self
            .state
            .read()
            .await
            .collectables
            .values()
            .cloned()
            .map(|item| LuaCollectableSnapshot {
                id: item.collectable_id,
                block_type: item.block_type,
                amount: item.amount,
                inventory_type: item.inventory_type,
                pos_x: item.pos_x,
                pos_y: item.pos_y,
                is_gem: item.is_gem,
            })
            .collect::<Vec<_>>();
        collectables.sort_by_key(|item| item.id);
        collectables
    }

    pub(crate) async fn world(&self) -> Result<LuaWorldSnapshot, String> {
        let state = self.state.read().await;
        let world = state
            .world
            .as_ref()
            .ok_or_else(|| "no world loaded yet".to_string())?;
        let mut growing_tiles = state
            .growing_tiles
            .iter()
            .map(|(&(x, y), item)| LuaGrowingTileSnapshot {
                x,
                y,
                block_id: item.block_id,
                growth_end_time: item.growth_end_time,
                growth_duration_secs: item.growth_duration_secs,
                mixed: item.mixed,
                harvest_seeds: item.harvest_seeds,
                harvest_blocks: item.harvest_blocks,
                harvest_gems: item.harvest_gems,
                harvest_extra_blocks: item.harvest_extra_blocks,
            })
            .collect::<Vec<_>>();
        growing_tiles.sort_by_key(|item| (item.y, item.x));

        let mut collectables = state
            .collectables
            .values()
            .cloned()
            .map(|item| LuaCollectableSnapshot {
                id: item.collectable_id,
                block_type: item.block_type,
                amount: item.amount,
                inventory_type: item.inventory_type,
                pos_x: item.pos_x,
                pos_y: item.pos_y,
                is_gem: item.is_gem,
            })
            .collect::<Vec<_>>();
        collectables.sort_by_key(|item| item.id);

        Ok(LuaWorldSnapshot {
            name: world.world_name.clone(),
            width: world.width,
            height: world.height,
            spawn: LuaWorldSpawnSnapshot {
                map_x: world.spawn_map_x,
                map_y: world.spawn_map_y,
                world_x: world.spawn_world_x,
                world_y: world.spawn_world_y,
            },
            tiles: LuaWorldTilesSnapshot {
                foreground: state.world_foreground_tiles.clone(),
                background: state.world_background_tiles.clone(),
                water: state.world_water_tiles.clone(),
                wiring: state.world_wiring_tiles.clone(),
            },
            objects: LuaWorldObjectsSnapshot {
                collectables,
                growing_tiles,
            },
        })
    }

    pub(crate) async fn tile(&self, map_x: i32, map_y: i32) -> Result<LuaTileSnapshot, String> {
        let state = self.state.read().await;
        tile_snapshot_at(&state, map_x, map_y)
    }

    async fn send_command(&self, command: SessionCommand) -> Result<(), String> {
        self.controller_tx
            .send(ControllerEvent::Command(command))
            .await
            .map_err(|error| error.to_string())
    }

    async fn resolve_fishing_target(
        &self,
        direction: &str,
        bait_query: &str,
    ) -> Result<FishingTarget, String> {
        let state = self.state.read().await;
        let player_x = state
            .player_position
            .map_x
            .ok_or_else(|| "player position is unknown; join a world before fishing".to_string())?
            .round() as i32;
        let player_y = state
            .player_position
            .map_y
            .ok_or_else(|| "player position is unknown; join a world before fishing".to_string())?
            .round() as i32;
        let target = find_fishing_map_point(
            state.world.as_ref(),
            &state.world_water_tiles,
            player_x,
            player_y,
            direction,
        )?;
        find_inventory_bait(&state.inventory, bait_query)?;
        Ok(FishingTarget {
            direction: direction.to_string(),
            bait_query: bait_query.trim().to_string(),
            map_x: target.0,
            map_y: target.1,
        })
    }

    async fn clear_fishing_state(&self, status: Option<FishingPhase>) {
        let mut state = self.state.write().await;
        state.fishing.active = false;
        state.fishing.phase = status.unwrap_or(FishingPhase::Idle);
        state.fishing.target_map_x = None;
        state.fishing.target_map_y = None;
        state.fishing.bait_name = None;
        state.fishing.last_result = None;
    }

    async fn run_controller(self: Arc<Self>, mut rx: mpsc::Receiver<ControllerEvent>) {
        let mut runtime: Option<ActiveRuntime> = None;
        let mut spam_stop_tx: Option<watch::Sender<bool>> = None;
        let mut fishing_stop_tx: Option<watch::Sender<bool>> = None;

        while let Some(event) = rx.recv().await {
            match event {
                ControllerEvent::Command(command) => match command {
                    SessionCommand::Connect => {
                        if runtime.is_some() {
                            continue;
                        }
                        match self.establish_connection(None).await {
                            Ok(active) => runtime = Some(active),
                            Err(error) => self.set_error(error).await,
                        }
                    }
                    SessionCommand::JoinWorld { world, instance } => {
                        {
                            let mut state = self.state.write().await;
                            state.pending_world = Some(world.to_uppercase());
                            state.pending_world_is_instance = instance;
                            state.status = SessionStatus::JoiningWorld;
                            state.other_players.clear();
                        }
                        self.publish_snapshot().await;
                        if let Some(active) = &runtime {
                            if instance {
                                let _ = send_docs_exclusive(
                                    &active.outbound_tx,
                                    vec![
                                        protocol::make_world_load_args(&[0]),
                                        protocol::make_join_world_special(&world, 0),
                                    ],
                                )
                                .await;
                            } else {
                                let _ =
                                    send_doc(&active.outbound_tx, protocol::make_join_world(&world))
                                        .await;
                            }
                        }
                    }
                    SessionCommand::LeaveWorld => {
                        stop_background_worker(&mut spam_stop_tx);
                        stop_background_worker(&mut fishing_stop_tx);
                        if let Some(active) = &runtime {
                            let _ =
                                send_doc(&active.outbound_tx, protocol::make_leave_world()).await;
                            let _ = send_scheduler_cmd(
                                &active.outbound_tx,
                                SchedulerCommand::SetPhase {
                                    phase: SchedulerPhase::MenuIdle,
                                },
                            )
                            .await;
                        }
                        self.reset_world_state(SessionStatus::MenuReady).await;
                    }
                    SessionCommand::Disconnect => {
                        stop_background_worker(&mut spam_stop_tx);
                        stop_background_worker(&mut fishing_stop_tx);
                        if let Some(active) = runtime.take() {
                            let _ = active.stop_tx.send(true);
                        }
                        self.reset_world_state(SessionStatus::Disconnected).await;
                    }
                    SessionCommand::AutomateTutorial => {
                        let already_running = {
                            let state = self.state.read().await;
                            state.tutorial_automation_running
                        };
                        if already_running {
                            self.logger.state(
                                Some(&self.id),
                                "tutorial automation is already running for this session",
                            );
                            continue;
                        }
                        {
                            let mut state = self.state.write().await;
                            state.tutorial_automation_running = true;
                        }
                        let Some(active) = &runtime else {
                            {
                                let mut state = self.state.write().await;
                                state.tutorial_automation_running = false;
                            }
                            self.set_error(
                                "connect the session before starting tutorial automation"
                                    .to_string(),
                            )
                            .await;
                            continue;
                        };
                        let outbound_tx = active.outbound_tx.clone();
                        let controller_tx = self.controller_tx.clone();
                        let state = self.state.clone();
                        let logger = self.logger.clone();
                        let session_id = self.id.clone();
                        tokio::spawn(async move {
                            let result = run_tutorial_script(
                                session_id.clone(),
                                logger.clone(),
                                state.clone(),
                                controller_tx,
                                outbound_tx,
                            )
                            .await;
                            state.write().await.tutorial_automation_running = false;
                            if let Err(error) = result {
                                logger.error("tutorial", Some(&session_id), error);
                            }
                        });
                    }
                    SessionCommand::ManualMove { direction } => {
                        let Some(active) = &runtime else {
                            self.set_error(
                                "connect the session before sending manual movement".to_string(),
                            )
                            .await;
                            continue;
                        };
                        let outbound_tx = active.outbound_tx.clone();
                        let state = self.state.clone();
                        let logger = self.logger.clone();
                        let session_id = self.id.clone();
                        tokio::spawn(async move {
                            if let Err(error) =
                                manual_move(&session_id, &logger, &state, &outbound_tx, &direction)
                                    .await
                            {
                                logger.error("movement", Some(&session_id), error);
                            }
                        });
                    }
                    SessionCommand::WearItem { block_id, equip } => {
                        let Some(active) = &runtime else {
                            self.set_error("connect the session before wearing items".to_string())
                                .await;
                            continue;
                        };
                        let outbound_tx = active.outbound_tx.clone();
                        let packet = if equip {
                            protocol::make_wear_item(block_id)
                        } else {
                            protocol::make_unwear_item(block_id)
                        };
                        let _ = send_doc(&outbound_tx, packet).await;
                    }
                    SessionCommand::Punch { offset_x, offset_y } => {
                        let Some(active) = &runtime else {
                            self.set_error("connect the session before punching".to_string())
                                .await;
                            continue;
                        };
                        let outbound_tx = active.outbound_tx.clone();
                        let state = self.state.clone();
                        let logger = self.logger.clone();
                        let session_id = self.id.clone();
                        tokio::spawn(async move {
                            if let Err(error) = manual_punch(
                                &session_id,
                                &logger,
                                &state,
                                &outbound_tx,
                                offset_x,
                                offset_y,
                            )
                            .await
                            {
                                logger.error("punch", Some(&session_id), error);
                            }
                        });
                    }
                    SessionCommand::Place {
                        offset_x,
                        offset_y,
                        block_id,
                    } => {
                        let Some(active) = &runtime else {
                            self.set_error("connect the session before placing blocks".to_string())
                                .await;
                            continue;
                        };
                        let outbound_tx = active.outbound_tx.clone();
                        let state = self.state.clone();
                        let logger = self.logger.clone();
                        let session_id = self.id.clone();
                        tokio::spawn(async move {
                            if let Err(error) = manual_place(
                                &session_id,
                                &logger,
                                &state,
                                &outbound_tx,
                                offset_x,
                                offset_y,
                                block_id,
                            )
                            .await
                            {
                                logger.error("place", Some(&session_id), error);
                            }
                        });
                    }
                    SessionCommand::StartFishing { direction, bait } => {
                        stop_background_worker(&mut fishing_stop_tx);
                        let Some(active) = &runtime else {
                            self.set_error(
                                "connect the session before starting fishing".to_string(),
                            )
                            .await;
                            continue;
                        };
                        let target = match self.resolve_fishing_target(&direction, &bait).await {
                            Ok(target) => target,
                            Err(error) => {
                                self.set_error(error).await;
                                continue;
                            }
                        };
                        let (stop_tx, stop_rx) = watch::channel(false);
                        fishing_stop_tx = Some(stop_tx);
                        let outbound_tx = active.outbound_tx.clone();
                        let state = self.state.clone();
                        let logger = self.logger.clone();
                        let session_id = self.id.clone();
                        tokio::spawn(async move {
                            if let Err(error) = fishing_loop(
                                &session_id,
                                &logger,
                                &state,
                                &outbound_tx,
                                stop_rx,
                                target,
                            )
                            .await
                            {
                                logger.error("fishing", Some(&session_id), error);
                            }
                        });
                    }
                    SessionCommand::StopFishing => {
                        stop_background_worker(&mut fishing_stop_tx);
                        if let Some(active) = &runtime {
                            let _ = send_doc(
                                &active.outbound_tx,
                                protocol::make_stop_fishing_game(false),
                            )
                            .await;
                            let _ = send_doc(
                                &active.outbound_tx,
                                protocol::make_stop_fishing_game(true),
                            )
                            .await;
                        }
                        self.clear_fishing_state(None).await;
                    }
                    SessionCommand::Talk { message } => {
                        let Some(active) = &runtime else {
                            self.set_error("connect the session before sending chat".to_string())
                                .await;
                            continue;
                        };
                        if let Err(error) =
                            send_world_chat(&self.id, &self.logger, &active.outbound_tx, &message)
                                .await
                        {
                            self.set_error(error).await;
                        }
                    }
                    SessionCommand::StartSpam { message, delay_ms } => {
                        stop_background_worker(&mut spam_stop_tx);
                        let Some(active) = &runtime else {
                            self.set_error("connect the session before starting spam".to_string())
                                .await;
                            continue;
                        };
                        let (stop_tx, stop_rx) = watch::channel(false);
                        spam_stop_tx = Some(stop_tx);
                        let outbound_tx = active.outbound_tx.clone();
                        let logger = self.logger.clone();
                        let session_id = self.id.clone();
                        tokio::spawn(async move {
                            if let Err(error) = spam_loop(
                                &session_id,
                                &logger,
                                &outbound_tx,
                                stop_rx,
                                message,
                                delay_ms,
                            )
                            .await
                            {
                                logger.error("spam", Some(&session_id), error);
                            }
                        });
                    }
                    SessionCommand::StopSpam => {
                        stop_background_worker(&mut spam_stop_tx);
                    }
                    SessionCommand::DropItem {
                        block_id,
                        amount,
                    } => {
                        let Some(active) = &runtime else {
                            self.set_error("connect the session before dropping items".to_string())
                                .await;
                            continue;
                        };
                        let outbound_tx = active.outbound_tx.clone();
                        let (tile_x, tile_y, inventory_type) = {
                            let state = self.state.read().await;
                            let pos = &state.player_position;
                            let inventory_type = block_inventory_type_for(block_id as u16)
                                .map(i32::from)
                                .or_else(|| {
                                    state
                                        .inventory
                                        .iter()
                                        .find(|entry| {
                                            entry.block_id == block_id as u16 && entry.amount > 0
                                        })
                                        .map(|entry| entry.inventory_type as i32)
                                })
                                .unwrap_or_default();
                            if let (Some(world_x), Some(world_y)) = (pos.world_x, pos.world_y) {
                                let (map_x, map_y) = protocol::world_to_map(world_x, world_y);
                                (map_x.floor() as i32, map_y.floor() as i32 + 1, inventory_type)
                            } else {
                                (
                                    pos.map_x.unwrap_or(0.0).floor() as i32,
                                    pos.map_y.unwrap_or(0.0).floor() as i32 + 1,
                                    inventory_type,
                                )
                            }
                        };
                        let _ = send_docs_immediate(
                            &outbound_tx,
                            vec![
                                protocol::make_empty_movement(),
                                protocol::make_drop_item(
                                    tile_x,
                                    tile_y,
                                    block_id,
                                    inventory_type,
                                    amount,
                                ),
                                protocol::make_progress_signal(0),
                            ],
                        )
                        .await;
                    }
                },
                ControllerEvent::Inbound(runtime_id, message) => {
                    if let Some(active) = runtime.as_mut() {
                        if active.id != runtime_id {
                            continue;
                        }
                        if let Err(error) = self.handle_inbound(active, message).await {
                            stop_background_worker(&mut spam_stop_tx);
                            stop_background_worker(&mut fishing_stop_tx);
                            self.set_error(error).await;
                        }
                    }
                }
                ControllerEvent::ReadLoopStopped(runtime_id, reason) => {
                    let Some(active) = runtime.as_ref() else {
                        continue;
                    };
                    if active.id != runtime_id {
                        continue;
                    }
                    stop_background_worker(&mut spam_stop_tx);
                    stop_background_worker(&mut fishing_stop_tx);
                    runtime = None;
                    self.state.write().await.current_outbound_tx = None;
                    self.set_error(reason).await;
                }
            }
        }
    }

    async fn establish_connection(
        &self,
        host_override: Option<String>,
    ) -> Result<ActiveRuntime, String> {
        self.update_status(SessionStatus::Connecting, None).await;
        let resolved =
            auth::resolve_auth(self.auth.clone(), self.logger.clone(), self.id.clone()).await?;
        {
            let mut state = self.state.write().await;
            state.device_id = resolved.device_id.clone();
            if let Some(host) = host_override {
                state.current_host = host;
            }
            state.current_port = net::default_port();
            state.last_error = None;
        }
        self.publish_snapshot().await;

        self.update_status(SessionStatus::Authenticating, None)
            .await;
        let host = self.state.read().await.current_host.clone();
        self.logger.state(
            Some(&self.id),
            format!("connecting to {host}:{}", net::default_port()),
        );

        let mut stream = net::connect_tcp(&host, net::default_port()).await?;
        self.send_and_expect(
            &mut stream,
            &[protocol::make_vchk(&resolved.device_id)],
            ids::PACKET_ID_VCHK,
        )
        .await?;
        let gpd = self
            .send_and_expect(
                &mut stream,
                &[protocol::make_gpd(&resolved.jwt)],
                ids::PACKET_ID_GPD,
            )
            .await?;
        self.apply_profile(&gpd).await;

        let runtime_id = RUNTIME_COUNTER.fetch_add(1, Ordering::Relaxed);
        let (read_half, write_half) = stream.into_split();
        let (outbound_tx, outbound_rx) = mpsc::channel(512);
        let (stop_tx, stop_rx) = watch::channel(false);
        let controller_tx = self.controller_tx.clone();
        let session_id = self.id.clone();
        let logger = self.logger.clone();
        tokio::spawn(async move {
            read_loop(read_half, controller_tx, logger, session_id, runtime_id).await;
        });
        let session_id = self.id.clone();
        let logger = self.logger.clone();
        let state_for_scheduler = self.state.clone();
        tokio::spawn(async move {
            scheduler_loop(write_half, outbound_rx, stop_rx, logger, session_id, state_for_scheduler).await;
        });

        self.state.write().await.current_outbound_tx = Some(outbound_tx.clone());
        self.update_status(SessionStatus::MenuReady, None).await;
        Ok(ActiveRuntime {
            id: runtime_id,
            outbound_tx,
            stop_tx,
        })
    }

    async fn send_and_expect(
        &self,
        stream: &mut tokio::net::TcpStream,
        messages: &[Document],
        expected_id: &str,
    ) -> Result<Document, String> {
        self.logger.transport(
            TransportKind::Tcp,
            Direction::Outgoing,
            "tcp",
            Some(&self.id),
            protocol::summarize_messages(messages),
        );
        protocol::write_batch(stream, messages).await?;

        let mut received_batches = Vec::new();
        for _ in 0..16 {
            let response = protocol::read_packet(stream).await?;
            let extracted = protocol::extract_messages(&response);
            self.logger.transport(
                TransportKind::Tcp,
                Direction::Incoming,
                "tcp",
                Some(&self.id),
                protocol::summarize_messages(&extracted),
            );

            if extracted.is_empty() {
                received_batches.push("empty response batch".to_string());
                continue;
            }

            for message in &extracted {
                if message.get_str("ID").unwrap_or_default() == ids::PACKET_ID_GPD {
                    self.apply_profile(message).await;
                }
            }

            if let Some(found) = extracted
                .iter()
                .find(|message| message.get_str("ID").unwrap_or_default() == expected_id)
            {
                return Ok(found.clone());
            }

            received_batches.push(protocol::summarize_messages(&extracted));
        }

        Err(format!(
            "expected {expected_id}, got {}",
            received_batches.join(" -> ")
        ))
    }

    async fn send_and_receive(
        &self,
        stream: &mut tokio::net::TcpStream,
        messages: &[Document],
    ) -> Result<Document, String> {
        self.logger.transport(
            TransportKind::Tcp,
            Direction::Outgoing,
            "tcp",
            Some(&self.id),
            protocol::summarize_messages(messages),
        );
        protocol::write_batch(stream, messages).await?;
        let response = protocol::read_packet(stream).await?;
        self.logger.transport(
            TransportKind::Tcp,
            Direction::Incoming,
            "tcp",
            Some(&self.id),
            protocol::summarize_messages(&protocol::extract_messages(&response)),
        );
        Ok(response)
    }

    async fn apply_profile(&self, profile: &Document) {
        let mut state = self.state.write().await;
        state.username = profile.get_str("UN").ok().map(ToOwned::to_owned);
        state.user_id = profile.get_str("U").ok().map(ToOwned::to_owned);
        state.inventory = decode_inventory(profile);
        state.last_error = None;
        drop(state);
        self.publish_snapshot().await;
    }

    async fn handle_inbound(
        &self,
        runtime: &mut ActiveRuntime,
        message: Document,
    ) -> Result<(), String> {
        let id = message.get_str("ID").unwrap_or_default();
        match id {
            ids::PACKET_ID_ST => {
                let _ = send_scheduler_cmd(
                    &runtime.outbound_tx,
                    SchedulerCommand::StResponseReceived,
                )
                .await;
            }
            ids::PACKET_ID_KEEPALIVE
            | ids::PACKET_ID_VCHK
            | ids::PACKET_ID_WREU
            | ids::PACKET_ID_BCSU
            | ids::PACKET_ID_DAILY_BONUS
            | ids::PACKET_ID_GET_LSI => {}
            ids::PACKET_ID_GPD => self.apply_profile(&message).await,
            ids::PACKET_ID_MOVEMENT | "U" | "AnP" => {
                self.maybe_update_player_positions(&message).await;
            }
            ids::PACKET_ID_PLAYER_LEAVE => {
                self.remove_other_player(&message).await;
            }
            "A" => {
                self.maybe_apply_spawn_pot_selection(&message).await;
            }
            ids::PACKET_ID_SET_BLOCK => {
                self.apply_set_block_message(&message).await;
            }
            ids::PACKET_ID_SEED_BLOCK => {
                self.apply_seed_growth_message(&message).await;
            }
            ids::PACKET_ID_DESTROY_BLOCK => {
                self.apply_destroy_block_message(&message).await;
            }
            ids::PACKET_ID_NEW_COLLECTABLE | ids::PACKET_ID_COLLECTABLE_REQUEST => {
                self.track_collectable(&message).await;
            }
            ids::PACKET_ID_COLLECTABLE_REMOVE => {
                self.remove_collectable(&message).await;
            }
            ids::PACKET_ID_FISHING_GAME_ACTION => {
                self.apply_fishing_message(&message, &runtime.outbound_tx).await;
            }
            ids::PACKET_ID_FISHING_RESULT => {
                let result = message
                    .get_i32("IK")
                    .map(|item| format!("fishing reward inventory_key={item}"))
                    .unwrap_or_else(|_| "fishing reward received".to_string());
                {
                    let mut state = self.state.write().await;
                    state.fishing.active = false;
                    state.fishing.phase = FishingPhase::CleanupPending;
                    state.fishing.cleanup_pending = true;
                    state.fishing.last_result = Some(result.clone());
                }
                let _ = send_doc(
                    &runtime.outbound_tx,
                    protocol::make_fishing_cleanup_action(),
                )
                .await;
                self.logger.state(Some(&self.id), result);
            }
            ids::PACKET_ID_STOP_MINIGAME => {
                let mut state = self.state.write().await;
                if state.fishing.cleanup_pending {
                    state.fishing = FishingAutomationState::default();
                    state.fishing.phase = FishingPhase::Completed;
                } else if state.fishing.active {
                    state.fishing = FishingAutomationState::default();
                }
            }
            ids::PACKET_ID_JOIN_WORLD => {
                let denied = message.get_i32("JR").unwrap_or_default() != 0;
                if denied {
                    let err = message
                        .get_str("E")
                        .or_else(|_| message.get_str("Err"))
                        .unwrap_or("join denied");
                    self.logger
                        .warn("session", Some(&self.id), format!("TTjW denied: {err}"));
                    self.set_error(format!("TTjW denied: {err}")).await;
                } else {
                    let world = message
                        .get_str("WN")
                        .ok()
                        .map(ToOwned::to_owned)
                        .or_else(|| {
                            self.state
                                .try_read()
                                .ok()
                                .and_then(|state| state.pending_world.clone())
                        });
                    {
                        let mut state = self.state.write().await;
                        state.current_world = world.clone();
                        state.status = SessionStatus::LoadingWorld;
                        state.other_players.clear();
                    }
                    self.publish_snapshot().await;
                    if let Some(world) = world {
                        let _ = send_docs_exclusive(
                            &runtime.outbound_tx,
                            protocol::make_enter_world(&world),
                        )
                        .await;
                    }
                }
            }
            ids::PACKET_ID_GET_WORLD_CONTENT => {
                let raw = protocol::binary_bytes(message.get("W")).unwrap_or_default();
                let world_name = self.state.read().await.current_world.clone();
                let decoded_world = world::decode_gwc(world_name.clone(), &raw)?;
                {
                    let mut state = self.state.write().await;
                    state.world = Some(decoded_world.snapshot.clone());
                    state.world_foreground_tiles = decoded_world.foreground_tiles;
                    state.world_background_tiles = decoded_world.background_tiles;
                    state.world_water_tiles = decoded_world.water_tiles;
                    state.world_wiring_tiles = decoded_world.wiring_tiles;
                    state.growing_tiles.clear();
                    state.collectables.clear();
                    state.other_players.clear();
                    state.player_position = PlayerPosition {
                        map_x: decoded_world.snapshot.spawn_map_x,
                        map_y: decoded_world.snapshot.spawn_map_y,
                        world_x: decoded_world.snapshot.spawn_world_x,
                        world_y: decoded_world.snapshot.spawn_world_y,
                    };
                    state.status = SessionStatus::AwaitingReady;
                    state.awaiting_ready = true;
                }
                self.publish_snapshot().await;

                if let Some(world) = world_name {
                    let _ = send_docs_exclusive(
                        &runtime.outbound_tx,
                        protocol::make_spawn_location_sync(&world),
                    )
                    .await;
                    let _ = send_docs_exclusive(&runtime.outbound_tx, protocol::make_spawn_setup())
                        .await;
                }
            }
            ids::PACKET_ID_R_OP => {
                self.update_status(SessionStatus::AwaitingReady, None).await;
            }
            ids::PACKET_ID_R_AI => {
                let should_ready = self.state.read().await.awaiting_ready;
                if should_ready {
                    let _ =
                        send_docs_exclusive(&runtime.outbound_tx, protocol::make_ready_to_play())
                            .await;

                    sleep(Duration::from_millis(1000)).await;

                    if let Some(world) = self.state.read().await.world.clone() {
                        if let (Some(map_x), Some(map_y), Some(world_x), Some(world_y)) = (
                            world.spawn_map_x,
                            world.spawn_map_y,
                            world.spawn_world_x,
                            world.spawn_world_y,
                        ) {
                            let _ = send_docs_exclusive(
                                &runtime.outbound_tx,
                                protocol::make_spawn_packets(
                                map_x.round() as i32,
                                map_y.round() as i32,
                                world_x,
                                world_y,
                                ),
                            )
                            .await;
                        }
                    }

                    {
                        let mut state = self.state.write().await;
                        state.awaiting_ready = false;
                        state.status = SessionStatus::InWorld;
                    }
                    let _ = send_scheduler_cmd(
                        &runtime.outbound_tx,
                        SchedulerCommand::SetPhase {
                            phase: SchedulerPhase::WorldIdle,
                        },
                    )
                    .await;
                    self.publish_snapshot().await;
                }
            }
            ids::PACKET_ID_REDIRECT => {
                let redirect_host = message.get_str("IP").unwrap_or_default().to_string();
                let (fallback, is_instance) = {
                    let state = self.state.read().await;
                    (
                        state
                            .pending_world
                            .clone()
                            .or_else(|| state.current_world.clone()),
                        state.pending_world_is_instance,
                    )
                };
                let world = message
                    .get_str("WN")
                    .ok()
                    .map(ToOwned::to_owned)
                    .or(fallback);
                let _ = runtime.stop_tx.send(true);
                self.update_status(SessionStatus::Redirecting, None).await;
                let new_runtime = self.establish_connection(Some(redirect_host)).await?;
                *runtime = new_runtime;
                if let Some(world) = world {
                    {
                        let mut state = self.state.write().await;
                        state.pending_world = Some(world.clone());
                    }
                    if is_instance {
                        let _ = send_docs_exclusive(
                            &runtime.outbound_tx,
                            vec![
                                protocol::make_world_load_args(&[0]),
                                protocol::make_join_world_special(&world, 0),
                            ],
                        )
                        .await;
                    } else {
                        let _ = send_doc(&runtime.outbound_tx, protocol::make_join_world(&world)).await;
                    }
                }
            }
            ids::PACKET_ID_ALREADY_CONNECTED => {
                self.set_error("server reported Already Connected".to_string())
                    .await;
            }
            ids::PACKET_ID_KICK_ERROR => {
                let error_code = message.get_i32("ER").unwrap_or_default();
                self.set_error(format!("kicked by server (code {error_code})"))
                    .await;
            }
            _ => {}
        }
        Ok(())
    }

    async fn update_status(&self, status: SessionStatus, last_error: Option<String>) {
        {
            let mut state = self.state.write().await;
            state.status = status;
            state.last_error = last_error;
        }
        self.publish_snapshot().await;
    }

    async fn set_error(&self, error: String) {
        self.logger.error("session", Some(&self.id), &error);
        {
            let mut state = self.state.write().await;
            state.status = SessionStatus::Error;
            state.last_error = Some(error);
        }
        self.publish_snapshot().await;
    }

    async fn reset_world_state(&self, status: SessionStatus) {
        {
            let mut state = self.state.write().await;
            state.status = status;
            state.current_world = None;
            state.pending_world = None;
            state.pending_world_is_instance = false;
            state.world = None;
            state.world_foreground_tiles.clear();
            state.world_background_tiles.clear();
            state.world_water_tiles.clear();
            state.world_wiring_tiles.clear();
            state.current_outbound_tx = None;
            state.growing_tiles.clear();
            state.collectables.clear();
            state.other_players.clear();
            state.player_position = PlayerPosition {
                map_x: None,
                map_y: None,
                world_x: None,
                world_y: None,
            };
            state.awaiting_ready = false;
            state.tutorial_spawn_pod_confirmed = false;
            state.tutorial_automation_running = false;
            state.fishing = FishingAutomationState::default();
            state.last_error = None;
        }
        self.publish_snapshot().await;
    }

    async fn publish_snapshot(&self) {
        let snapshot = self.snapshot().await;
        self.logger.session_snapshot(snapshot);
    }

    async fn maybe_update_player_positions(&self, message: &Document) {
        let packet_uid = message.get_str("U").ok();
        let local_uid = self.state.read().await.user_id.clone();
        if let Some(uid) = packet_uid {
            let mut state = self.state.write().await;
            if local_uid.as_deref() == Some(uid) {
                let changed = update_player_position_from_message(message, &mut state.player_position);
                drop(state);
                if changed {
                    self.publish_snapshot().await;
                }
                return;
            }

            let remote = state
                .other_players
                .entry(uid.to_string())
                .or_insert(PlayerPosition {
                    map_x: None,
                    map_y: None,
                    world_x: None,
                    world_y: None,
                });
            let changed = update_player_position_from_message(message, remote);
            drop(state);
            if changed {
                self.publish_snapshot().await;
            }
            return;
        }
    }

    async fn remove_other_player(&self, message: &Document) {
        let Ok(user_id) = message.get_str("U") else {
            return;
        };
        let local_uid = self.state.read().await.user_id.clone();
        if local_uid.as_deref() == Some(user_id) {
            return;
        }

        let removed = self.state.write().await.other_players.remove(user_id).is_some();
        if removed {
            self.publish_snapshot().await;
        }
    }

    async fn apply_set_block_message(&self, message: &Document) {
        let Ok(map_x) = message.get_i32("x") else {
            return;
        };
        let Ok(map_y) = message.get_i32("y") else {
            return;
        };
        let block_id = match message.get_i32("BlockType") {
            Ok(value) if value >= 0 => value as u16,
            _ => return,
        };

        let changed = {
            let mut state = self.state.write().await;
            apply_foreground_block_change(&mut state, map_x, map_y, block_id)
        };
        if changed {
            self.publish_snapshot().await;
        }
    }

    async fn apply_seed_growth_message(&self, message: &Document) {
        let Ok(map_x) = message.get_i32("x") else {
            return;
        };
        let Ok(map_y) = message.get_i32("y") else {
            return;
        };
        let Ok(growth_end_time) = message.get_i64("GrowthEndTime") else {
            return;
        };
        let Ok(block_id) = message.get_i32("BlockType") else {
            return;
        };

        let growth = GrowingTileState {
            block_id: block_id.max(0) as u16,
            growth_end_time,
            growth_duration_secs: message.get_i32("GrowthDuration").unwrap_or_default().max(0),
            mixed: message.get_bool("Mixed").unwrap_or(false),
            harvest_seeds: message.get_i32("HarvestSeeds").unwrap_or_default().max(0),
            harvest_blocks: message.get_i32("HarvestBlocks").unwrap_or_default().max(0),
            harvest_gems: message.get_i32("HarvestGems").unwrap_or_default().max(0),
            harvest_extra_blocks: message
                .get_i32("HarvestExtraBlocks")
                .unwrap_or_default()
                .max(0),
        };

        self.state
            .write()
            .await
            .growing_tiles
            .insert((map_x, map_y), growth);
    }

    async fn apply_destroy_block_message(&self, message: &Document) {
        let Ok(map_x) = message.get_i32("x") else {
            return;
        };
        let Ok(map_y) = message.get_i32("y") else {
            return;
        };

        let changed = {
            let mut state = self.state.write().await;
            state.growing_tiles.remove(&(map_x, map_y));
            apply_destroy_block_change(&mut state, map_x, map_y)
        };
        if changed {
            self.publish_snapshot().await;
        }
    }

    async fn maybe_apply_spawn_pot_selection(&self, message: &Document) {
        let Ok(values) = message.get_array("APu") else {
            return;
        };

        let picked = values
            .iter()
            .filter_map(|value| match value {
                bson::Bson::Int32(value) => Some(*value),
                bson::Bson::Int64(value) => i32::try_from(*value).ok(),
                _ => None,
            })
            .collect::<Vec<_>>();

        if picked != tutorial::POST_CHARACTER_POD_CONFIRMATION {
            return;
        }

        let should_reflect = {
            let state = self.state.read().await;
            state.current_world.as_deref() == Some(tutorial::TUTORIAL_WORLD)
                || state.pending_world.as_deref() == Some(tutorial::TUTORIAL_WORLD)
        };
        if !should_reflect {
            return;
        }

        self.logger.state(
            Some(&self.id),
            format!(
                "server confirmed tutorial pod APu={picked:?}, bot will walk to map=({}, {})",
                tutorial::SPAWN_POT_MAP_X,
                tutorial::SPAWN_POT_MAP_Y,
            ),
        );
        let mut state = self.state.write().await;
        state.tutorial_spawn_pod_confirmed = true;
    }

    async fn track_collectable(&self, message: &Document) {
        let Some(collectable_id) = message.get_i32("CollectableID").ok() else {
            return;
        };

        let collectable = CollectableState {
            collectable_id,
            block_type: message.get_i32("BlockType").unwrap_or_default(),
            amount: message.get_i32("Amount").unwrap_or_default(),
            inventory_type: message.get_i32("InventoryType").unwrap_or_default(),
            pos_x: message.get_f64("PosX").unwrap_or_default(),
            pos_y: message.get_f64("PosY").unwrap_or_default(),
            is_gem: message.get_bool("IsGem").unwrap_or(false),
        };

        self.state
            .write()
            .await
            .collectables
            .insert(collectable_id, collectable);
    }

    async fn remove_collectable(&self, message: &Document) {
        let Some(collectable_id) = message.get_i32("CollectableID").ok() else {
            return;
        };
        self.state
            .write()
            .await
            .collectables
            .remove(&collectable_id);
    }

    async fn apply_fishing_message(
        &self,
        message: &Document,
        outbound_tx: &OutboundHandle,
    ) {
        let minigame_type = message.get_i32("MGT").unwrap_or_default();
        if minigame_type != 2 {
            return;
        }

        let mut state = self.state.write().await;
        if !state.fishing.active {
            return;
        }

        let mgd = message
            .get_i64("MGD")
            .ok()
            .or_else(|| message.get_i32("MGD").ok().map(i64::from))
            .unwrap_or_default();

        match mgd {
            2 => {
                state.fishing.phase = FishingPhase::HookPrompted;
                if !state.fishing.hook_sent {
                    state.fishing.hook_sent = true;
                    let _ = send_doc(outbound_tx, protocol::make_fishing_hook_action()).await;
                }
            }
            3 => {
                state.fishing.phase = FishingPhase::GaugeActive;
                state.fishing.fish_block = message.get_i32("BT").ok();
                state.fishing.rod_block = message.get_i32("WBT").ok();
                let now = Instant::now();
                state.fishing.gauge_entered_at = Some(now);
                initialize_fishing_gauge(&mut state.fishing, now);
            }
            1 | 5 => {
                state.fishing = FishingAutomationState::default();
            }
            _ => {}
        }
    }
}

fn update_player_position_from_message(message: &Document, position: &mut PlayerPosition) -> bool {
    let previous = position.clone();
    if let Ok(x) = message.get_f64("x") {
        position.world_x = Some(x);
        let (map_x, _) = protocol::world_to_map(x, position.world_y.unwrap_or_default());
        position.map_x = Some(map_x);
    }
    if let Ok(y) = message.get_f64("y") {
        position.world_y = Some(y);
        let (_, map_y) = protocol::world_to_map(position.world_x.unwrap_or_default(), y);
        position.map_y = Some(map_y);
    }

    position.map_x != previous.map_x
        || position.map_y != previous.map_y
        || position.world_x != previous.world_x
        || position.world_y != previous.world_y
}

#[derive(Debug)]
struct ActiveRuntime {
    id: u64,
    outbound_tx: OutboundHandle,
    stop_tx: watch::Sender<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendMode {
    Mergeable,
    ExclusiveBatch,
    ImmediateExclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueuePriority {
    BeforeGenerated,
    AfterGenerated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchedulerPhase {
    Disconnected,
    MenuIdle,
    MenuStBurst,
    WorldIdle,
    WorldMoving,
}

#[derive(Debug)]
enum SchedulerCommand {
    EnqueuePackets {
        docs: Vec<Document>,
        mode: SendMode,
        priority: QueuePriority,
    },
    UpdateMovement {
        world_x: f64,
        world_y: f64,
        is_moving: bool,
        anim: i32,
        direction: i32,
    },
    SetPhase {
        phase: SchedulerPhase,
    },
    StResponseReceived,
    Shutdown,
}

type OutboundHandle = mpsc::Sender<SchedulerCommand>;

#[derive(Debug, Clone)]
struct MovementTickState {
    in_world: bool,
    world_x: f64,
    world_y: f64,
    is_moving: bool,
    anim: i32,
    direction: i32,
}

impl Default for MovementTickState {
    fn default() -> Self {
        Self {
            in_world: false,
            world_x: 0.0,
            world_y: 0.0,
            is_moving: false,
            anim: movement::ANIM_IDLE,
            direction: movement::DIR_RIGHT,
        }
    }
}

#[derive(Debug)]
enum PendingBatch {
    Mergeable {
        docs: Vec<Document>,
        priority: QueuePriority,
    },
    Exclusive(Vec<Document>),
}

struct StSyncState {
    samples: [i32; timing::ST_SAMPLE_COUNT],
    sample_count: usize,
    success_counter: i32,
    interval_secs: i32,
    last_sent_at: Option<std::time::Instant>,
}

impl StSyncState {
    fn new() -> Self {
        Self {
            samples: [i32::MAX; timing::ST_SAMPLE_COUNT],
            sample_count: 0,
            success_counter: 0,
            interval_secs: timing::ST_INTERVAL_INIT_SECS,
            last_sent_at: None,
        }
    }

    fn record_sample(&mut self, rtt_ms: i32) {
        let idx = self.sample_count % timing::ST_SAMPLE_COUNT;
        self.samples[idx] = rtt_ms;
        self.sample_count += 1;

        let valid_count = self.sample_count.min(timing::ST_SAMPLE_COUNT);
        let mut sorted = self.samples;
        sorted[..valid_count].sort_unstable();
        let median = sorted[valid_count / 2];

        let deviation = (rtt_ms - median).abs();

        if deviation >= self.interval_secs * 1000 {
            self.interval_secs =
                (self.interval_secs - timing::ST_INTERVAL_STEP_SECS).max(timing::ST_INTERVAL_MIN_SECS);
            self.success_counter = 0;
        } else {
            self.success_counter += 1;
            if self.success_counter > timing::ST_SUCCESS_THRESHOLD {
                self.success_counter = 0;
                self.interval_secs =
                    (self.interval_secs + timing::ST_INTERVAL_STEP_SECS).min(timing::ST_INTERVAL_MAX_SECS);
            }
        }
    }
}

struct SchedulerState {
    phase: SchedulerPhase,
    movement: MovementTickState,
    st_sync: StSyncState,
    pending: VecDeque<PendingBatch>,
    immediate: VecDeque<Vec<Document>>,
    menu_keepalive_due: bool,
    st_due: bool,
}

impl SchedulerState {
    fn new() -> Self {
        Self {
            phase: SchedulerPhase::MenuStBurst,
            movement: MovementTickState::default(),
            st_sync: StSyncState::new(),
            pending: VecDeque::new(),
            immediate: VecDeque::new(),
            menu_keepalive_due: false,
            st_due: true,
        }
    }

    fn has_immediate_batch(&self) -> bool {
        !self.immediate.is_empty()
    }

    fn is_menu_phase(&self) -> bool {
        matches!(
            self.phase,
            SchedulerPhase::MenuIdle | SchedulerPhase::MenuStBurst
        )
    }

    fn enqueue_packets(&mut self, docs: Vec<Document>, mode: SendMode, priority: QueuePriority) {
        if docs.is_empty() {
            return;
        }
        match mode {
            SendMode::ImmediateExclusive => self.immediate.push_back(docs),
            SendMode::ExclusiveBatch => self.pending.push_back(PendingBatch::Exclusive(docs)),
            SendMode::Mergeable => match self.pending.back_mut() {
                Some(PendingBatch::Mergeable {
                    docs: queued,
                    priority: queued_priority,
                }) if *queued_priority == priority => queued.extend(docs),
                _ => self
                    .pending
                    .push_back(PendingBatch::Mergeable { docs, priority }),
            },
        }
    }

    fn update_movement(
        &mut self,
        world_x: f64,
        world_y: f64,
        is_moving: bool,
        anim: i32,
        direction: i32,
    ) {
        self.movement.world_x = world_x;
        self.movement.world_y = world_y;
        self.movement.is_moving = is_moving;
        self.movement.anim = anim;
        self.movement.direction = direction;
        if matches!(
            self.phase,
            SchedulerPhase::WorldIdle | SchedulerPhase::WorldMoving
        ) {
            self.phase = if is_moving {
                SchedulerPhase::WorldMoving
            } else {
                SchedulerPhase::WorldIdle
            };
        }
    }

    fn set_phase(&mut self, phase: SchedulerPhase) {
        self.phase = match phase {
            SchedulerPhase::WorldIdle | SchedulerPhase::WorldMoving => {
                self.movement.in_world = true;
                if self.movement.is_moving {
                    SchedulerPhase::WorldMoving
                } else {
                    SchedulerPhase::WorldIdle
                }
            }
            SchedulerPhase::MenuIdle | SchedulerPhase::MenuStBurst => {
                self.movement.in_world = false;
                self.movement.is_moving = false;
                self.menu_keepalive_due = false;
                phase
            }
            SchedulerPhase::Disconnected => {
                self.movement.in_world = false;
                self.movement.is_moving = false;
                SchedulerPhase::Disconnected
            }
        };
    }

    fn mark_menu_keepalive_due(&mut self) {
        if self.is_menu_phase() {
            self.menu_keepalive_due = true;
        }
    }

    fn mark_st_due(&mut self) {
        self.st_due = true;
    }

    fn take_immediate_batch(&mut self) -> Option<Vec<Document>> {
        self.immediate.pop_front()
    }

    fn take_slot_batch(&mut self) -> Option<Vec<Document>> {
        if let Some(PendingBatch::Exclusive(_)) = self.pending.front() {
            if let Some(PendingBatch::Exclusive(docs)) = self.pending.pop_front() {
                return Some(docs);
            }
        }

        let mut before_generated = Vec::new();
        let mut after_generated = Vec::new();
        while let Some(PendingBatch::Mergeable { .. }) = self.pending.front() {
            if let Some(PendingBatch::Mergeable { docs, priority }) = self.pending.pop_front() {
                match priority {
                    QueuePriority::BeforeGenerated => before_generated.extend(docs),
                    QueuePriority::AfterGenerated => after_generated.extend(docs),
                }
            }
        }

        let mut batch = before_generated;
        match self.phase {
            SchedulerPhase::Disconnected => {
                if batch.is_empty() && after_generated.is_empty() {
                    return None;
                }
            }
            SchedulerPhase::MenuIdle => {
                batch.extend(after_generated);
                if self.menu_keepalive_due {
                    batch.push(protocol::make_keepalive());
                    self.menu_keepalive_due = false;
                }
                if self.st_due {
                    batch.push(protocol::make_st());
                    self.st_due = false;
                }
                if batch.is_empty() {
                    return Some(Vec::new());
                }
            }
            SchedulerPhase::MenuStBurst => {
                batch.extend(after_generated);
                if self.menu_keepalive_due {
                    batch.push(protocol::make_keepalive());
                    self.menu_keepalive_due = false;
                }
                if self.st_due {
                    batch.push(protocol::make_st());
                    self.st_due = false;
                }
                if batch.is_empty() {
                    return None;
                }
            }
            SchedulerPhase::WorldIdle => {
                batch.push(protocol::make_empty_movement());
                batch.extend(after_generated);
                if self.st_due {
                    batch.push(protocol::make_st());
                    self.st_due = false;
                }
            }
            SchedulerPhase::WorldMoving => {
                batch.push(protocol::make_movement_packet(
                    self.movement.world_x,
                    self.movement.world_y,
                    self.movement.anim,
                    self.movement.direction,
                    false,
                ));
                batch.extend(after_generated);
                if self.st_due {
                    batch.push(protocol::make_st());
                    self.st_due = false;
                }
            }
        }

        Some(batch)
    }

    fn on_batch_sent(&mut self, batch: &[Document]) -> bool {
        let sent_st = batch.iter().any(|doc| packet_id(doc) == ids::PACKET_ID_ST);
        if sent_st {
            self.st_sync.last_sent_at = Some(std::time::Instant::now());
        }
        sent_st
    }

    fn handle_st_response(&mut self) -> Option<Duration> {
        let sent_at = self.st_sync.last_sent_at.take()?;
        let rtt_ms = sent_at.elapsed().as_millis() as i32;
        self.st_sync.record_sample(rtt_ms);
        if self.st_sync.sample_count < timing::ST_SAMPLE_COUNT {
            self.st_due = true;
            if self.phase == SchedulerPhase::MenuIdle {
                self.phase = SchedulerPhase::MenuStBurst;
            }
            None
        } else {
            if self.phase == SchedulerPhase::MenuStBurst {
                self.phase = SchedulerPhase::MenuIdle;
            }
            Some(Duration::from_secs(self.st_sync.interval_secs as u64))
        }
    }
}

fn packet_id(doc: &Document) -> &str {
    doc.get_str("ID").unwrap_or_default()
}

async fn write_logged_batch(
    writer: &mut OwnedWriteHalf,
    logger: &Logger,
    session_id: &str,
    batch: &[Document],
) -> Result<(), String> {
    logger.transport(
        TransportKind::Tcp,
        Direction::Outgoing,
        "tcp",
        Some(session_id),
        protocol::log_batch(batch),
    );
    protocol::write_batch(writer, batch).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FishingPhase {
    Idle,
    WaitingForHook,
    HookPrompted,
    GaugeActive,
    CleanupPending,
    Completed,
}

#[derive(Debug, Clone)]
struct FishingAutomationState {
    active: bool,
    phase: FishingPhase,
    target_map_x: Option<i32>,
    target_map_y: Option<i32>,
    bait_name: Option<String>,
    last_result: Option<String>,
    fish_block: Option<i32>,
    rod_block: Option<i32>,
    gauge_entered_at: Option<Instant>,
    hook_sent: bool,
    land_sent: bool,
    cleanup_pending: bool,
    sim_last_at: Option<Instant>,
    sim_fish_position: f64,
    sim_target_position: f64,
    sim_progress: f64,
    sim_overlap_threshold: f64,
    sim_fill_rate: f64,
    sim_target_speed: f64,
    sim_fish_move_speed: f64,
    sim_run_frequency: f64,
    sim_pull_strength: f64,
    sim_min_land_delay: f64,
    sim_phase: f64,
    sim_overlap: bool,
    sim_ready_since: Option<Instant>,
    sim_difficulty_meter: f64,
    sim_size_multiplier: f64,
    sim_drag_extra: f64,
    sim_run_active: bool,
    sim_run_until: Option<Instant>,
    sim_force_land_after: Option<Instant>,
}

impl Default for FishingAutomationState {
    fn default() -> Self {
        Self {
            active: false,
            phase: FishingPhase::Idle,
            target_map_x: None,
            target_map_y: None,
            bait_name: None,
            last_result: None,
            fish_block: None,
            rod_block: None,
            gauge_entered_at: None,
            hook_sent: false,
            land_sent: false,
            cleanup_pending: false,
            sim_last_at: None,
            sim_fish_position: 0.5,
            sim_target_position: 0.5,
            sim_progress: 0.5,
            sim_overlap_threshold: 0.13,
            sim_fill_rate: 0.12,
            sim_target_speed: 0.4,
            sim_fish_move_speed: 0.8,
            sim_run_frequency: 0.04,
            sim_pull_strength: 3.4,
            sim_min_land_delay: 4.8,
            sim_phase: 0.0,
            sim_overlap: false,
            sim_ready_since: None,
            sim_difficulty_meter: 0.0,
            sim_size_multiplier: 0.0,
            sim_drag_extra: 1.0,
            sim_run_active: false,
            sim_run_until: None,
            sim_force_land_after: None,
        }
    }
}

#[derive(Debug)]
struct SessionState {
    status: SessionStatus,
    device_id: String,
    current_host: String,
    current_port: u16,
    current_world: Option<String>,
    pending_world: Option<String>,
    pending_world_is_instance: bool,
    username: Option<String>,
    user_id: Option<String>,
    world: Option<WorldSnapshot>,
    world_foreground_tiles: Vec<u16>,
    world_background_tiles: Vec<u16>,
    world_water_tiles: Vec<u16>,
    world_wiring_tiles: Vec<u16>,
    current_outbound_tx: Option<OutboundHandle>,
    growing_tiles: HashMap<(i32, i32), GrowingTileState>,
    player_position: PlayerPosition,
    other_players: HashMap<String, PlayerPosition>,
    inventory: Vec<InventoryEntry>,
    collectables: HashMap<i32, CollectableState>,
    last_error: Option<String>,
    awaiting_ready: bool,
    tutorial_spawn_pod_confirmed: bool,
    tutorial_automation_running: bool,
    fishing: FishingAutomationState,
    ping_ms: Option<u32>,
}

#[derive(Debug)]
enum SessionCommand {
    Connect,
    JoinWorld { world: String, instance: bool },
    LeaveWorld,
    Disconnect,
    AutomateTutorial,
    ManualMove {
        direction: String,
    },
    WearItem {
        block_id: i32,
        equip: bool,
    },
    Punch {
        offset_x: i32,
        offset_y: i32,
    },
    Place {
        offset_x: i32,
        offset_y: i32,
        block_id: i32,
    },
    StartFishing {
        direction: String,
        bait: String,
    },
    StopFishing,
    Talk {
        message: String,
    },
    StartSpam {
        message: String,
        delay_ms: u64,
    },
    StopSpam,
    DropItem {
        block_id: i32,
        amount: i32,
    },
}

#[derive(Debug)]
enum ControllerEvent {
    Command(SessionCommand),
    Inbound(u64, Document),
    ReadLoopStopped(u64, String),
}

#[derive(Debug, Clone)]
struct InventoryEntry {
    inventory_key: i32,
    block_id: u16,
    inventory_type: u16,
    amount: u16,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CollectableState {
    collectable_id: i32,
    block_type: i32,
    amount: i32,
    inventory_type: i32,
    pos_x: f64,
    pos_y: f64,
    is_gem: bool,
}

#[derive(Debug, Clone)]
struct FishingTarget {
    direction: String,
    bait_query: String,
    map_x: i32,
    map_y: i32,
}

#[derive(Debug, Clone)]
struct NamedInventoryEntry {
    inventory_key: i32,
    block_id: u16,
    name: String,
}

#[derive(Debug, Clone)]
struct GrowingTileState {
    block_id: u16,
    growth_end_time: i64,
    growth_duration_secs: i32,
    mixed: bool,
    harvest_seeds: i32,
    harvest_blocks: i32,
    harvest_gems: i32,
    harvest_extra_blocks: i32,
}

fn stop_background_worker(stop_tx: &mut Option<watch::Sender<bool>>) {
    if let Some(tx) = stop_tx.take() {
        let _ = tx.send(true);
    }
}

fn ensure_not_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(AtomicOrdering::Relaxed) {
        Err("lua script stopped".to_string())
    } else {
        Ok(())
    }
}

fn apply_foreground_block_change(
    state: &mut SessionState,
    map_x: i32,
    map_y: i32,
    block_id: u16,
) -> bool {
    let Some(world) = state.world.as_ref() else {
        return false;
    };
    let Some(index) = tile_index(world, map_x, map_y) else {
        return false;
    };
    let Some(tile) = state.world_foreground_tiles.get_mut(index) else {
        return false;
    };
    if *tile == block_id {
        return false;
    }

    *tile = block_id;
    let Some(world) = state.world.as_mut() else {
        return false;
    };
    world.tile_counts = summarize_tile_counts(&state.world_foreground_tiles);
    true
}

fn apply_destroy_block_change(state: &mut SessionState, map_x: i32, map_y: i32) -> bool {
    let Some(world) = state.world.as_ref() else {
        return false;
    };
    let Some(index) = tile_index(world, map_x, map_y) else {
        return false;
    };

    if let Some(tile) = state.world_foreground_tiles.get_mut(index) {
        if *tile != 0 {
            *tile = 0;
            if let Some(world) = state.world.as_mut() {
                world.tile_counts = summarize_tile_counts(&state.world_foreground_tiles);
            }
            return true;
        }
    }

    if let Some(tile) = state.world_background_tiles.get_mut(index) {
        if *tile != 0 {
            *tile = 0;
            return true;
        }
    }

    false
}

fn tile_index(world: &WorldSnapshot, map_x: i32, map_y: i32) -> Option<usize> {
    if map_x < 0 || map_y < 0 {
        return None;
    }

    let width = world.width as usize;
    let height = world.height as usize;
    if width == 0 || height == 0 {
        return None;
    }

    let map_x = map_x as usize;
    let map_y = map_y as usize;
    if map_x >= width || map_y >= height {
        return None;
    }

    Some(map_y * width + map_x)
}

fn is_tile_ready_to_harvest_at(
    state: &SessionState,
    map_x: i32,
    map_y: i32,
    now_ticks: i64,
) -> Result<bool, String> {
    if state.world.is_none() {
        return Err("no world loaded yet".to_string());
    }

    let Some(growth) = state.growing_tiles.get(&(map_x, map_y)) else {
        return Ok(false);
    };

    Ok(now_ticks >= growth.growth_end_time)
}

fn tile_snapshot_at(
    state: &SessionState,
    map_x: i32,
    map_y: i32,
) -> Result<LuaTileSnapshot, String> {
    let world = state
        .world
        .as_ref()
        .ok_or_else(|| "no world loaded yet".to_string())?;
    let index = tile_index(world, map_x, map_y)
        .ok_or_else(|| format!("tile ({map_x}, {map_y}) is out of bounds"))?;
    Ok(LuaTileSnapshot {
        foreground: state
            .world_foreground_tiles
            .get(index)
            .copied()
            .unwrap_or_default(),
        background: state
            .world_background_tiles
            .get(index)
            .copied()
            .unwrap_or_default(),
        water: state
            .world_water_tiles
            .get(index)
            .copied()
            .unwrap_or_default(),
        wiring: state
            .world_wiring_tiles
            .get(index)
            .copied()
            .unwrap_or_default(),
        ready_to_harvest: is_tile_ready_to_harvest_at(
            state,
            map_x,
            map_y,
            protocol::csharp_ticks(),
        )?,
    })
}

fn summarize_tile_counts(tiles: &[u16]) -> Vec<TileCount> {
    let mut counts = BTreeMap::<u16, u32>::new();
    for &tile_id in tiles {
        *counts.entry(tile_id).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(tile_id, count)| TileCount { tile_id, count })
        .collect()
}

fn block_types() -> &'static HashMap<u16, BlockTypeInfo> {
    BLOCK_TYPES.get_or_init(|| {
        serde_json::from_str::<Vec<BlockTypeInfo>>(include_str!("../../block_types.json"))
            .unwrap_or_default()
            .into_iter()
            .map(|entry| (entry.id, entry))
            .collect()
    })
}

fn block_names() -> HashMap<u16, String> {
    block_types()
        .iter()
        .map(|(id, entry)| (*id, entry.name.clone()))
        .collect()
}

fn block_inventory_type_for(block_id: u16) -> Option<u16> {
    block_types().get(&block_id).map(|entry| entry.inventory_type)
}

fn block_name_for(block_id: u16) -> Option<String> {
    block_types().get(&block_id).map(|entry| entry.name.clone())
}

fn block_type_name_for(block_id: u16) -> Option<String> {
    block_types().get(&block_id).map(|entry| entry.type_name.clone())
}

fn normalize_block_name(name: &str) -> String {
    name.chars()
        .filter(|char| char.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn find_inventory_bait(
    inventory: &[InventoryEntry],
    bait_query: &str,
) -> Result<NamedInventoryEntry, String> {
    let bait_query = bait_query.trim();
    if bait_query.is_empty() {
        return Err("bait is required".to_string());
    }

    if let Ok(block_id) = bait_query.parse::<u16>() {
        if let Some(item) = inventory
            .iter()
            .find(|item| item.block_id == block_id && item.amount > 0)
        {
            return Ok(NamedInventoryEntry {
                inventory_key: item.inventory_key,
                block_id: item.block_id,
                name: block_name_for(item.block_id).unwrap_or_else(|| format!("#{}", item.block_id)),
            });
        }
    }

    let normalized_query = normalize_block_name(bait_query);
    inventory
        .iter()
        .filter(|item| item.amount > 0)
        .find_map(|item| {
            let name = block_name_for(item.block_id)?;
            (normalize_block_name(&name) == normalized_query).then_some(NamedInventoryEntry {
                inventory_key: item.inventory_key,
                block_id: item.block_id,
                name,
            })
        })
        .ok_or_else(|| format!("bait '{bait_query}' was not found in inventory"))
}

fn find_fishing_map_point(
    world: Option<&WorldSnapshot>,
    _water_tiles: &[u16],
    player_x: i32,
    player_y: i32,
    direction: &str,
) -> Result<(i32, i32), String> {
    let world = world.ok_or_else(|| "join a world before starting fishing".to_string())?;
    if world.width == 0 || world.height == 0 {
        return Err("world data is not loaded yet".to_string());
    }

    let width = world.width as i32;
    let height = world.height as i32;
    let target_x = player_x + if direction == "left" { -1 } else { 1 };
    let target_y = player_y - 1;
    if target_x < 0 || target_x >= width || target_y < 0 || target_y >= height {
        return Err(format!(
            "fishing target ({target_x}, {target_y}) is outside world bounds"
        ));
    }
    Ok((target_x, target_y))
}

fn rod_family_name(rod_block: Option<i32>) -> &'static str {
    let rod = rod_block.unwrap_or(2406);
    let index = if (2406..=2421).contains(&rod) {
        (rod - 2406) % 4
    } else {
        0
    };
    match index {
        0 => "bamboo",
        1 => "fiberglass",
        2 => "carbon",
        3 => "titanium",
        _ => "bamboo",
    }
}

fn initialize_fishing_gauge(fishing: &mut FishingAutomationState, now: Instant) {
    let rod_block = fishing.rod_block.unwrap_or(2406);
    let fish_name = fishing
        .fish_block
        .and_then(|id| block_name_for(id as u16))
        .unwrap_or_default();
    let normalized = normalize_block_name(&fish_name);
    let bucket = fishing::fish_bucket_from_name(&normalized);
    let rod_profile = fishing::rod_profile(rod_block);

    fishing.sim_overlap_threshold = 0.095 + (rod_profile.slider_size * 0.035);
    fishing.sim_fill_rate = rod_profile.fill_multiplier * 0.10;
    fishing.sim_target_speed = rod_profile.slider_speed * 0.20;
    fishing.sim_fish_move_speed = bucket.fish_move_speed;
    fishing.sim_run_frequency = bucket.run_frequency;
    fishing.sim_pull_strength = fishing::pull_strength(bucket, rod_family_name(Some(rod_block)));
    fishing.sim_min_land_delay = bucket.min_land_delay;
    fishing.sim_last_at = Some(now);
    fishing.sim_fish_position = fishing::DEFAULT_FISH_POSITION;
    fishing.sim_target_position = fishing::DEFAULT_TARGET_POSITION;
    fishing.sim_progress = fishing::DEFAULT_PROGRESS;
    fishing.sim_phase = 0.0;
    fishing.sim_overlap = false;
    fishing.sim_ready_since = None;
    fishing.sim_difficulty_meter = 0.0;
    fishing.sim_size_multiplier = 0.0;
    fishing.sim_drag_extra = fishing::DEFAULT_DRAG_EXTRA;
    fishing.sim_run_active = false;
    fishing.sim_run_until = None;
    fishing.sim_force_land_after = Some(
        now + Duration::from_secs_f64(
            (bucket.min_land_delay + fishing::FORCE_LAND_EXTRA_DELAY_SECS)
                .max(fishing::FORCE_LAND_MIN_SECS),
        ),
    );
    fishing.land_sent = false;
}

fn current_fishing_land_values(fishing: &FishingAutomationState) -> (i32, i32, f64) {
    let size_multiplier = fishing
        .sim_size_multiplier
        .clamp(0.001, fishing::MAX_SIZE_MULTIPLIER);
    let difficulty_meter = fishing
        .sim_difficulty_meter
        .clamp(0.001, fishing::MAX_DIFFICULTY_METER);
    let vendor_index = (size_multiplier * 1000.0).max(1.0) as i32;
    let index_key = (difficulty_meter * 1000.0).max(1.0) as i32;
    let amount = fishing.sim_fish_position - fishing.sim_drag_extra;
    (vendor_index, index_key, amount)
}

fn service_fishing_simulation(
    fishing: &mut FishingAutomationState,
    now: Instant,
) -> Option<Document> {
    if fishing.phase != FishingPhase::GaugeActive || fishing.cleanup_pending {
        return None;
    }

    let Some(last_at) = fishing.sim_last_at else {
        fishing.sim_last_at = Some(now);
        return None;
    };
    let dt = (now - last_at).as_secs_f64().clamp(0.0, 0.25);
    fishing.sim_last_at = Some(now);
    if dt <= 0.0 {
        return None;
    }

    fishing.sim_phase += dt;
    let prev_fish = fishing.sim_fish_position;
    let prev_target = fishing.sim_target_position;
    let move_speed = fishing.sim_fish_move_speed;
    let run_frequency = fishing.sim_run_frequency;
    let base_wave = 0.18 + (move_speed * 0.05);
    let burst_wave = 0.08 + (run_frequency * 1.1);

    if !fishing.sim_run_active
        && fishing
            .gauge_entered_at
            .map(|entered| (now - entered).as_secs_f64() >= fishing::RUN_START_AFTER_SECS)
            .unwrap_or(false)
        && (fishing.sim_phase * (0.75 + move_speed)).sin() > (0.985 - run_frequency)
    {
        fishing.sim_run_active = true;
        fishing.sim_run_until = Some(now + Duration::from_millis(fishing::RUN_DURATION_MS));
    }

    let run_boost = if fishing.sim_run_active { 0.22 } else { 0.0 };
    let center =
        0.5 + (base_wave + run_boost) * (fishing.sim_phase * (0.9 + move_speed * 0.55)).sin();
    let burst = burst_wave * (fishing.sim_phase * (2.3 + move_speed * 1.1)).sin();
    let fish = (center + burst).clamp(0.0, 1.0);
    if fishing.sim_run_active
        && fishing
            .sim_run_until
            .map(|until| now >= until)
            .unwrap_or(false)
    {
        fishing.sim_run_active = false;
        fishing.sim_run_until = None;
    }

    let distance = (fish - prev_target).abs();
    let should_overlap = distance <= (fishing.sim_overlap_threshold * 1.35);
    let force_finish = fishing
        .sim_force_land_after
        .map(|deadline| now >= deadline)
        .unwrap_or(false);

    let mut target = prev_target;
    if force_finish {
        target = fish;
    } else if should_overlap {
        let step = fishing.sim_target_speed * dt;
        target = if fish > target {
            (target + step).min(fish)
        } else {
            (target - step).max(fish)
        };
    } else {
        let step = (fishing.sim_target_speed * 0.35) * dt;
        target = if fish > target {
            (target + step).min(fish)
        } else {
            (target - step).max(fish)
        };
    }
    target = target.clamp(0.0, 1.0);

    fishing.sim_fish_position = fish;
    fishing.sim_target_position = target;
    fishing.sim_difficulty_meter += (target - prev_target).abs();
    fishing.sim_size_multiplier += (fish - prev_fish).abs();
    fishing.sim_drag_extra = fish + 0.5;

    let off_distance = (fish - target).abs();
    let is_overlapping = off_distance <= fishing.sim_overlap_threshold;
    if is_overlapping != fishing.sim_overlap {
        fishing.sim_overlap = is_overlapping;
        return Some(if is_overlapping {
            protocol::make_fish_on_area()
        } else {
            protocol::make_fish_off_area(off_distance)
        });
    }

    if force_finish {
        fishing.sim_progress = (fishing.sim_progress.max(0.985)
            + (fishing.sim_fill_rate * 2.5).max(0.22) * dt)
            .clamp(0.0, 1.0);
    } else if is_overlapping {
        fishing.sim_progress = (fishing.sim_progress + fishing.sim_fill_rate * dt).clamp(0.0, 1.0);
    } else {
        let drain_rate = ((((off_distance * 2.5) + 0.5) * fishing.sim_pull_strength) * 0.05) * dt;
        fishing.sim_progress = (fishing.sim_progress - drain_rate).clamp(0.0, 1.0);
    }

    let can_land = fishing.sim_progress >= 0.999
        && is_overlapping
        && fishing
            .gauge_entered_at
            .map(|entered| (now - entered).as_secs_f64() >= fishing.sim_min_land_delay)
            .unwrap_or(false);

    if can_land {
        if fishing.sim_ready_since.is_none() {
            fishing.sim_ready_since = Some(now);
        } else if !fishing.land_sent
            && fishing
                .sim_ready_since
                .map(|ready_since| {
                    (now - ready_since).as_secs_f64() >= fishing::READY_TO_LAND_DELAY_SECS
                })
                .unwrap_or(false)
        {
            let (vendor_index, index_key, amount) = current_fishing_land_values(fishing);
            fishing.land_sent = true;
            return Some(protocol::make_fishing_land_action(
                vendor_index,
                index_key,
                amount,
            ));
        }
    } else {
        fishing.sim_ready_since = None;
    }

    None
}

async fn spam_loop(
    session_id: &str,
    logger: &Logger,
    outbound_tx: &OutboundHandle,
    mut stop_rx: watch::Receiver<bool>,
    message: String,
    delay_ms: u64,
) -> Result<(), String> {
    send_world_chat(session_id, logger, outbound_tx, &message).await?;

    let mut tick = interval(Duration::from_millis(delay_ms));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    return Ok(());
                }
            }
            _ = tick.tick() => {
                send_world_chat(session_id, logger, outbound_tx, &message).await?;
            }
        }
    }
}

async fn send_world_chat(
    _session_id: &str,
    _logger: &Logger,
    outbound_tx: &OutboundHandle,
    message: &str,
) -> Result<(), String> {
    send_docs_exclusive(
        outbound_tx,
        vec![
            protocol::make_empty_movement(),
            protocol::make_world_chat(message),
            protocol::make_progress_signal(0),
        ],
    )
    .await
}

async fn fishing_loop(
    session_id: &str,
    logger: &Logger,
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
    mut stop_rx: watch::Receiver<bool>,
    target: FishingTarget,
) -> Result<(), String> {
    loop {
        if *stop_rx.borrow() {
            return stop_fishing_game(state, outbound_tx).await;
        }

        let bait = match consume_fishing_bait(state, &target.bait_query).await {
            Ok(bait) => bait,
            Err(_) => {
                {
                    let mut session = state.write().await;
                    session.fishing = FishingAutomationState::default();
                }
                publish_state_snapshot(logger, session_id, state).await;
                logger.state(
                    Some(session_id),
                    format!(
                        "auto-fishing stopped: no more '{}' found in inventory",
                        target.bait_query
                    ),
                );
                return Ok(());
            }
        };

        {
            let mut session = state.write().await;
            session.fishing = FishingAutomationState::default();
            session.fishing.active = true;
            session.fishing.phase = FishingPhase::WaitingForHook;
            session.fishing.target_map_x = Some(target.map_x);
            session.fishing.target_map_y = Some(target.map_y);
            session.fishing.bait_name = Some(bait.name.clone());
            session.fishing.last_result = None;
        }
        publish_state_snapshot(logger, session_id, state).await;

        logger.state(
            Some(session_id),
            format!(
                "starting fishing at map=({}, {}) dir={} bait={}",
                target.map_x, target.map_y, target.direction, bait.name
            ),
        );

        send_docs_exclusive(
            outbound_tx,
            vec![
                protocol::make_select_belt_item(bait.inventory_key),
                protocol::make_try_to_fish_from_map_point(
                    target.map_x,
                    target.map_y,
                    bait.block_id as i32,
                ),
                protocol::make_start_fishing_game(
                    target.map_x,
                    target.map_y,
                    bait.block_id as i32,
                ),
            ],
        )
        .await?;

        loop {
            if *stop_rx.borrow() {
                return stop_fishing_game(state, outbound_tx).await;
            }

            let fishing = state.read().await.fishing.clone();
            let phase = fishing.phase;
            if phase == FishingPhase::CleanupPending || phase == FishingPhase::Completed {
                break;
            }
            if !fishing.active {
                return Err("fishing was reset before hook prompt".to_string());
            }
            if phase == FishingPhase::HookPrompted || phase == FishingPhase::GaugeActive {
                break;
            }
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        return stop_fishing_game(state, outbound_tx).await;
                    }
                }
                _ = sleep(Duration::from_millis(100)) => {}
            }
        }

        let hook_sent = state.read().await.fishing.hook_sent;
        if !hook_sent {
            {
                let mut session = state.write().await;
                session.fishing.hook_sent = true;
            }
            send_docs_exclusive(outbound_tx, vec![protocol::make_fishing_hook_action()]).await?;
        }

        let mut gauge_tick = interval(Duration::from_millis(50));
        gauge_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            if *stop_rx.borrow() {
                return stop_fishing_game(state, outbound_tx).await;
            }

            let fishing = state.read().await.fishing.clone();
            let phase = fishing.phase;
            if phase == FishingPhase::CleanupPending {
                continue;
            }
            if phase == FishingPhase::Completed {
                break;
            }
            if !fishing.active {
                return Err("fishing was reset before reward".to_string());
            }

            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        return stop_fishing_game(state, outbound_tx).await;
                    }
                }
                _ = gauge_tick.tick() => {
                    let packet = {
                        let mut session = state.write().await;
                        service_fishing_simulation(&mut session.fishing, Instant::now())
                    };
                    if let Some(packet) = packet {
                        send_docs(outbound_tx, vec![packet]).await?;
                    }
                }
            }
        }

        sleep(Duration::from_millis(150)).await;
    }
}

async fn consume_fishing_bait(
    state: &Arc<RwLock<SessionState>>,
    bait_query: &str,
) -> Result<NamedInventoryEntry, String> {
    let mut session = state.write().await;
    let bait = find_inventory_bait(&session.inventory, bait_query)?;
    let item = session
        .inventory
        .iter_mut()
        .find(|item| item.inventory_key == bait.inventory_key)
        .ok_or_else(|| format!("bait '{bait_query}' was not found in inventory"))?;
    if item.amount == 0 {
        return Err(format!("bait '{bait_query}' was not found in inventory"));
    }
    item.amount -= 1;
    session.inventory.retain(|item| item.amount > 0);
    Ok(bait)
}

async fn stop_fishing_game(
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
) -> Result<(), String> {
    {
        let mut session = state.write().await;
        session.fishing = FishingAutomationState::default();
    }
    send_docs_exclusive(
        outbound_tx,
        vec![
            protocol::make_fishing_cleanup_action(),
            protocol::make_stop_fishing_game(false),
            protocol::make_stop_fishing_game(true),
        ],
    )
    .await
}

async fn read_loop(
    mut reader: OwnedReadHalf,
    controller_tx: mpsc::Sender<ControllerEvent>,
    logger: Logger,
    session_id: String,
    runtime_id: u64,
) {
    loop {
        match protocol::read_packet(&mut reader).await {
            Ok(packet) => {
                logger.transport(
                    TransportKind::Tcp,
                    Direction::Incoming,
                    "tcp",
                    Some(&session_id),
                    protocol::log_packet(&packet),
                );
                let messages = protocol::extract_messages(&packet);
                for message in messages {
                    if controller_tx
                        .send(ControllerEvent::Inbound(runtime_id, message))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = controller_tx
                    .send(ControllerEvent::ReadLoopStopped(runtime_id, error))
                    .await;
                return;
            }
        }
    }
}

async fn scheduler_loop(
    mut writer: OwnedWriteHalf,
    mut outbound_rx: mpsc::Receiver<SchedulerCommand>,
    mut stop_rx: watch::Receiver<bool>,
    logger: Logger,
    session_id: String,
    state: Arc<RwLock<SessionState>>,
) {
    let mut scheduler = SchedulerState::new();
    let start = tokio::time::Instant::now();
    let mut slot_tick = interval_at(start + timing::send_slot_interval(), timing::send_slot_interval());
    let mut keepalive_tick = interval_at(
        start + timing::menu_keepalive_interval(),
        timing::menu_keepalive_interval(),
    );
    slot_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    keepalive_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let far_future = start + Duration::from_secs(60 * 60 * 24 * 365);
    let mut st_sleep = Box::pin(sleep_until(far_future));

    loop {
        tokio::select! {
            biased;

            _ = async {}, if scheduler.has_immediate_batch() => {
                if let Some(batch) = scheduler.take_immediate_batch() {
                    if write_logged_batch(&mut writer, &logger, &session_id, &batch).await.is_err() {
                        return;
                    }
                    let sent_st = scheduler.on_batch_sent(&batch);
                    if sent_st && scheduler.phase == SchedulerPhase::MenuIdle {
                        scheduler.phase = SchedulerPhase::MenuStBurst;
                    }
                }
            }

            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    return;
                }
            }

            _ = &mut st_sleep => {
                scheduler.mark_st_due();
                st_sleep.as_mut().reset(far_future);
            }

            _ = keepalive_tick.tick() => {
                scheduler.mark_menu_keepalive_due();
            }

            _ = slot_tick.tick() => {
                if let Some(batch) = scheduler.take_slot_batch() {
                    if write_logged_batch(&mut writer, &logger, &session_id, &batch).await.is_err() {
                        return;
                    }
                    let sent_st = scheduler.on_batch_sent(&batch);
                    if sent_st && scheduler.phase == SchedulerPhase::MenuIdle {
                        scheduler.phase = SchedulerPhase::MenuStBurst;
                    }
                }
            }

            Some(cmd) = outbound_rx.recv() => {
                match cmd {
                    SchedulerCommand::EnqueuePackets { docs, mode, priority } => {
                        scheduler.enqueue_packets(docs, mode, priority);
                    }
                    SchedulerCommand::UpdateMovement { world_x, world_y, is_moving, anim, direction } => {
                        scheduler.update_movement(world_x, world_y, is_moving, anim, direction);
                    }
                    SchedulerCommand::SetPhase { phase } => {
                        scheduler.set_phase(phase);
                    }
                    SchedulerCommand::StResponseReceived => {
                        if let Some(rtt_ms) = scheduler.st_sync.last_sent_at.map(|sent_at| sent_at.elapsed().as_millis() as u32) {
                            state.write().await.ping_ms = Some(rtt_ms);
                        }
                        if let Some(next) = scheduler.handle_st_response() {
                            let deadline = tokio::time::Instant::now() + next;
                            st_sleep.as_mut().reset(deadline);
                        }
                    }
                    SchedulerCommand::Shutdown => return,
                }
            }

            else => return,
        }
    }
}

fn decode_inventory(profile: &Document) -> Vec<InventoryEntry> {
    let Some(raw_pd) = protocol::binary_bytes(profile.get("pD")) else {
        return Vec::new();
    };
    let Ok(pd) = Document::from_reader(Cursor::new(raw_pd)) else {
        return Vec::new();
    };
    let Some(inv_blob) = protocol::binary_bytes(pd.get("inv")) else {
        return Vec::new();
    };
    if inv_blob.len() % 6 != 0 {
        return Vec::new();
    }

    let mut entries = Vec::new();
    for chunk in inv_blob.chunks_exact(6) {
        let inventory_key = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as i32;
        let amount = u16::from_le_bytes([chunk[4], chunk[5]]);
        if amount == 0 {
            continue;
        }
        entries.push(InventoryEntry {
            inventory_key,
            block_id: (inventory_key as u32 & 0xFFFF) as u16,
            inventory_type: ((inventory_key as u32 >> 16) & 0xFFFF) as u16,
            amount,
        });
    }
    entries
}

async fn run_tutorial_script(
    _session_id: String,
    _logger: Logger,
    _state: Arc<RwLock<SessionState>>,
    _controller_tx: mpsc::Sender<ControllerEvent>,
    _outbound_tx: OutboundHandle,
) -> Result<(), String> {
    Ok(())
}

async fn ensure_world(
    session_id: &str,
    logger: &Logger,
    state: &Arc<RwLock<SessionState>>,
    controller_tx: &mpsc::Sender<ControllerEvent>,
    outbound_tx: &OutboundHandle,
    world: &str,
) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    ensure_world_cancellable(
        session_id,
        logger,
        state,
        controller_tx,
        outbound_tx,
        world,
        false,
        &cancel,
    )
    .await
}

async fn ensure_world_cancellable(
    session_id: &str,
    logger: &Logger,
    state: &Arc<RwLock<SessionState>>,
    controller_tx: &mpsc::Sender<ControllerEvent>,
    outbound_tx: &OutboundHandle,
    world: &str,
    instance: bool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    ensure_not_cancelled(cancel)?;
    let current = state.read().await.current_world.clone();
    let status = state.read().await.status.clone();
    if current.as_deref() == Some(world) && status == SessionStatus::InWorld {
        return Ok(());
    }

    let should_bootstrap_tutorial = world == tutorial::TUTORIAL_WORLD
        && current.is_none()
        && status == SessionStatus::MenuReady;

    if should_bootstrap_tutorial {
        logger.state(
            Some(session_id),
            format!("bootstrapping {world} directly with Gw/GWC flow from packets.bin"),
        );
        {
            let mut state = state.write().await;
            state.current_world = Some(world.to_string());
            state.pending_world = Some(world.to_string());
            state.status = SessionStatus::LoadingWorld;
            state.world = None;
            state.world_foreground_tiles.clear();
            state.world_background_tiles.clear();
            state.world_water_tiles.clear();
            state.world_wiring_tiles.clear();
            state.collectables.clear();
            state.other_players.clear();
        }
        let eid = if world == tutorial::TUTORIAL_WORLD {
            "Start"
        } else {
            ""
        };
        send_docs_exclusive(outbound_tx, protocol::make_enter_world_eid(world, eid)).await?;
    } else {
        logger.state(
            Some(session_id),
            format!("joining {world}{}",
                if instance { " (instance)" } else { "" }),
        );
        controller_tx
            .send(ControllerEvent::Command(SessionCommand::JoinWorld {
                world: world.to_string(),
                instance,
            }))
            .await
            .map_err(|error| error.to_string())?;
    }

    let deadline = Instant::now() + tutorial::world_join_timeout();
    loop {
        ensure_not_cancelled(cancel)?;
        {
            let state = state.read().await;
            if state.current_world.as_deref() == Some(world)
                && state.status == SessionStatus::InWorld
            {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting to enter {world}"));
        }
        sleep(Duration::from_millis(250)).await;
    }
}

async fn enqueue_packets(
    outbound_tx: &OutboundHandle,
    docs: Vec<Document>,
    mode: SendMode,
    priority: QueuePriority,
) -> Result<(), String> {
    if docs.is_empty() {
        return Ok(());
    }
    outbound_tx
        .send(SchedulerCommand::EnqueuePackets {
            docs,
            mode,
            priority,
        })
        .await
        .map_err(|error| error.to_string())
}

async fn send_doc(outbound_tx: &OutboundHandle, doc: Document) -> Result<(), String> {
    enqueue_packets(
        outbound_tx,
        vec![doc],
        SendMode::Mergeable,
        QueuePriority::AfterGenerated,
    )
    .await
}

async fn send_doc_before_generated(
    outbound_tx: &OutboundHandle,
    doc: Document,
) -> Result<(), String> {
    enqueue_packets(
        outbound_tx,
        vec![doc],
        SendMode::Mergeable,
        QueuePriority::BeforeGenerated,
    )
    .await
}

async fn send_scheduler_cmd(outbound_tx: &OutboundHandle, cmd: SchedulerCommand) -> Result<(), String> {
    outbound_tx.send(cmd).await.map_err(|error| error.to_string())
}

async fn send_docs(outbound_tx: &OutboundHandle, docs: Vec<Document>) -> Result<(), String> {
    enqueue_packets(
        outbound_tx,
        docs,
        SendMode::Mergeable,
        QueuePriority::AfterGenerated,
    )
    .await
}

async fn send_docs_exclusive(
    outbound_tx: &OutboundHandle,
    docs: Vec<Document>,
) -> Result<(), String> {
    enqueue_packets(
        outbound_tx,
        docs,
        SendMode::ExclusiveBatch,
        QueuePriority::AfterGenerated,
    )
    .await
}

async fn send_docs_immediate(outbound_tx: &OutboundHandle, docs: Vec<Document>) -> Result<(), String> {
    enqueue_packets(
        outbound_tx,
        docs,
        SendMode::ImmediateExclusive,
        QueuePriority::AfterGenerated,
    )
    .await
}

async fn walk_to_map(
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
    target_map_x: i32,
    target_map_y: i32,
) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    walk_to_map_cancellable(state, outbound_tx, target_map_x, target_map_y, &cancel).await
}

async fn walk_to_map_cancellable(
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
    target_map_x: i32,
    target_map_y: i32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    ensure_not_cancelled(cancel)?;
    let (start_x, start_y) = {
        let state = state.read().await;
        let x = state
            .player_position
            .map_x
            .unwrap_or(tutorial::PORTAL_APPROACH_X as f64)
            .round() as i32;
        let y = state
            .player_position
            .map_y
            .unwrap_or(tutorial::PORTAL_APPROACH_Y as f64)
            .round() as i32;
        (x, y)
    };

    let path = planned_path(state, (start_x, start_y), (target_map_x, target_map_y)).await;
    let steps = path.unwrap_or_else(|| {
        fallback_straight_line_path((start_x, start_y), (target_map_x, target_map_y))
    });
    let mut last_direction = {
        let state = state.read().await;
        current_facing_direction(&state.player_position)
    };

    for window in steps.windows(2) {
        ensure_not_cancelled(cancel)?;
        let [previous, current] = window else {
            continue;
        };

        let direction = if current.0 < previous.0 {
            movement::DIR_LEFT
        } else {
            movement::DIR_RIGHT
        };
        last_direction = direction;

        move_to_map(
            state,
            outbound_tx,
            current.0,
            current.1,
            direction,
            movement::ANIM_WALK,
        )
        .await?;
        sleep(tutorial::walk_step_pause()).await;
    }

    ensure_not_cancelled(cancel)?;
    let (world_x, world_y) = {
        let s = state.read().await;
        (
            s.player_position.world_x.unwrap_or_default(),
            s.player_position.world_y.unwrap_or_default(),
        )
    };
    send_scheduler_cmd(
        outbound_tx,
        SchedulerCommand::UpdateMovement {
            world_x,
            world_y,
            is_moving: false,
            anim: movement::ANIM_IDLE,
            direction: last_direction,
        },
    )
    .await?;
    Ok(())
}

async fn wait_for_tutorial_spawn_pod_confirmation(
    state: &Arc<RwLock<SessionState>>,
) -> Result<(), String> {
    let deadline = Instant::now() + tutorial::spawn_pod_confirm_timeout();
    loop {
        {
            let mut state = state.write().await;
            if state.tutorial_spawn_pod_confirmed {
                state.tutorial_spawn_pod_confirmed = false;
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            return Err("timed out waiting for tutorial spawn pod confirmation".to_string());
        }

        sleep(Duration::from_millis(50)).await;
    }
}

async fn walk_predefined_path(
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
    steps: &[(i32, i32)],
) -> Result<(), String> {
    let mut previous = {
        let state = state.read().await;
        (
            state
                .player_position
                .map_x
                .unwrap_or(tutorial::INTRO_PORTAL_WALK_PATH[0].0 as f64)
                .round() as i32,
            state
                .player_position
                .map_y
                .unwrap_or(tutorial::INTRO_PORTAL_WALK_PATH[0].1 as f64)
                .round() as i32,
        )
    };

    for &(map_x, map_y) in steps {
        let direction = if map_x < previous.0 {
            movement::DIR_LEFT
        } else {
            movement::DIR_RIGHT
        };
        move_to_map(
            state,
            outbound_tx,
            map_x,
            map_y,
            direction,
            movement::ANIM_WALK,
        )
        .await?;
        previous = (map_x, map_y);
        sleep(tutorial::walk_step_pause()).await;
    }

    let (world_x, world_y) = {
        let s = state.read().await;
        (
            s.player_position.world_x.unwrap_or_default(),
            s.player_position.world_y.unwrap_or_default(),
        )
    };
    let last_direction = if steps.last().copied().unwrap_or_default().0
        < steps.get(steps.len().saturating_sub(2)).copied().unwrap_or_default().0
    {
        movement::DIR_LEFT
    } else {
        movement::DIR_RIGHT
    };
    send_scheduler_cmd(
        outbound_tx,
        SchedulerCommand::UpdateMovement {
            world_x,
            world_y,
            is_moving: false,
            anim: movement::ANIM_IDLE,
            direction: last_direction,
        },
    )
    .await?;
    Ok(())
}

async fn manual_move(
    session_id: &str,
    logger: &Logger,
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
    direction: &str,
) -> Result<(), String> {
    let (target_map_x, target_map_y, facing_direction) = next_manual_step(state, direction).await?;

    let is_blocked = {
        let state = state.read().await;
        if let Some(world) = &state.world {
            if let Some(index) = tile_index(world, target_map_x, target_map_y) {
                if let Some(&tile_id) = state.world_foreground_tiles.get(index) {
                    if !astar::is_walkable_tile(tile_id) {
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        }
    };

    if is_blocked {
        return Err(format!(
            "cannot move {direction} - tile ({target_map_x}, {target_map_y}) is solid"
        ));
    }

    let (world_x, world_y) = protocol::map_to_world(target_map_x as f64, target_map_y as f64);
    logger.info(
        "movement",
        Some(session_id),
        format!(
            "manual move {direction} -> map=({target_map_x}, {target_map_y}) world=({world_x:.2}, {world_y:.2})"
        ),
    );
    set_local_map_position(logger, session_id, state, target_map_x, target_map_y).await;
    let (world_x, world_y) = protocol::map_to_world(target_map_x as f64, target_map_y as f64);
    send_doc_before_generated(outbound_tx, protocol::make_map_point(target_map_x, target_map_y)).await?;
    send_scheduler_cmd(
        outbound_tx,
        SchedulerCommand::UpdateMovement {
            world_x,
            world_y,
            is_moving: true,
            anim: movement::ANIM_WALK,
            direction: facing_direction,
        },
    )
    .await?;
    sleep(tutorial::walk_step_pause()).await;
    send_scheduler_cmd(
        outbound_tx,
        SchedulerCommand::UpdateMovement {
            world_x,
            world_y,
            is_moving: false,
            anim: movement::ANIM_IDLE,
            direction: facing_direction,
        },
    )
    .await?;
    sleep(Duration::from_millis(120)).await;

    Ok(())
}

async fn manual_punch(
    session_id: &str,
    logger: &Logger,
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
    offset_x: i32,
    offset_y: i32,
) -> Result<(), String> {
    let (target_map_x, target_map_y, facing_direction) =
        punch_target_from_offset(state, offset_x, offset_y).await?;
    let (fg, water, bg) = {
        let state = state.read().await;
        let snapshot = tile_snapshot_at(&state, target_map_x, target_map_y)
            .unwrap_or(LuaTileSnapshot {
                foreground: 0,
                background: 0,
                water: 0,
                wiring: 0,
                ready_to_harvest: false,
            });
        (snapshot.foreground, snapshot.water, snapshot.background)
    };
    let (layer, hit_doc) = if fg != 0 {
        ("fg", protocol::make_hit_block(target_map_x, target_map_y))
    } else if water != 0 {
        (
            "water",
            protocol::make_hit_block_water(target_map_x, target_map_y),
        )
    } else if bg != 0 {
        (
            "bg",
            protocol::make_hit_block_background(target_map_x, target_map_y),
        )
    } else {
        ("fg", protocol::make_hit_block(target_map_x, target_map_y))
    };
    logger.info(
        "punch",
        Some(session_id),
        format!(
            "manual punch offset=({offset_x}, {offset_y}) -> target=({target_map_x}, {target_map_y}) layer={layer}"
        ),
    );
    send_docs_exclusive(
        outbound_tx,
        vec![
            movement_doc(state, movement::ANIM_PUNCH, facing_direction).await,
            hit_doc,
        ],
    )
    .await?;
    Ok(())
}

async fn manual_place(
    session_id: &str,
    logger: &Logger,
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
    offset_x: i32,
    offset_y: i32,
    block_id: i32,
) -> Result<(), String> {
    let (target_map_x, target_map_y, facing_direction) =
        punch_target_from_offset(state, offset_x, offset_y).await?;
    logger.info(
        "place",
        Some(session_id),
        format!(
            "manual place block={block_id} offset=({offset_x}, {offset_y}) -> target=({target_map_x}, {target_map_y})"
        ),
    );
    send_docs_exclusive(
        outbound_tx,
        vec![
            movement_doc(state, movement::ANIM_IDLE, facing_direction).await,
            protocol::make_place_block(target_map_x, target_map_y, block_id),
        ],
    )
    .await?;
    Ok(())
}

async fn next_manual_step(
    state: &Arc<RwLock<SessionState>>,
    direction: &str,
) -> Result<(i32, i32, i32), String> {
    let state = state.read().await;
    let current_map_x = state
        .player_position
        .map_x
        .ok_or_else(|| "player map x is not known yet".to_string())?
        .round() as i32;
    let current_map_y = state
        .player_position
        .map_y
        .ok_or_else(|| "player map y is not known yet".to_string())?
        .round() as i32;

    let (dx, dy, facing_direction) = match direction {
        "left" => (-1, 0, movement::DIR_LEFT),
        "right" => (1, 0, movement::DIR_RIGHT),
        "up" => (0, 1, current_facing_direction(&state.player_position)),
        "down" => (0, -1, current_facing_direction(&state.player_position)),
        _ => return Err(format!("unsupported movement direction: {direction}")),
    };

    Ok((current_map_x + dx, current_map_y + dy, facing_direction))
}

async fn punch_target_from_offset(
    state: &Arc<RwLock<SessionState>>,
    offset_x: i32,
    offset_y: i32,
) -> Result<(i32, i32, i32), String> {
    let state = state.read().await;
    let current_map_x = state
        .player_position
        .map_x
        .ok_or_else(|| "player map x is not known yet".to_string())?
        .round() as i32;
    let current_map_y = state
        .player_position
        .map_y
        .ok_or_else(|| "player map y is not known yet".to_string())?
        .round() as i32;
    let facing_direction = if offset_x < 0 {
        movement::DIR_LEFT
    } else if offset_x > 0 {
        movement::DIR_RIGHT
    } else {
        current_facing_direction(&state.player_position)
    };

    Ok((
        current_map_x + offset_x,
        current_map_y + offset_y,
        facing_direction,
    ))
}

fn current_facing_direction(position: &PlayerPosition) -> i32 {
    match (position.map_x, position.world_x) {
        (Some(_), Some(_)) => movement::DIR_RIGHT,
        _ => movement::DIR_RIGHT,
    }
}

async fn planned_path(
    state: &Arc<RwLock<SessionState>>,
    start: (i32, i32),
    goal: (i32, i32),
) -> Option<Vec<(i32, i32)>> {
    let state = state.read().await;
    let world = state.world.as_ref()?;
    let width = world.width as usize;
    let height = world.height as usize;
    let tiles = &state.world_foreground_tiles;

    astar::find_path(width, height, start, goal, |x, y| {
        is_walkable_map_position(tiles, width, height, start, goal, x, y)
    })
}

fn is_walkable_map_position(
    tiles: &[u16],
    width: usize,
    height: usize,
    start: (i32, i32),
    goal: (i32, i32),
    x: i32,
    y: i32,
) -> bool {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return false;
    }
    if (x, y) == start || (x, y) == goal {
        return true;
    }

    let index = y as usize * width + x as usize;
    matches!(tiles.get(index), Some(0))
}

fn fallback_straight_line_path(start: (i32, i32), goal: (i32, i32)) -> Vec<(i32, i32)> {
    let mut path = vec![start];
    let mut current_x = start.0;
    let mut current_y = start.1;

    while current_x != goal.0 || current_y != goal.1 {
        if current_x < goal.0 {
            current_x += 1;
        } else if current_x > goal.0 {
            current_x -= 1;
        } else if current_y < goal.1 {
            current_y += 1;
        } else if current_y > goal.1 {
            current_y -= 1;
        }
        path.push((current_x, current_y));
    }

    path
}

async fn move_to_map(
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
    map_x: i32,
    map_y: i32,
    direction: i32,
    anim: i32,
) -> Result<(), String> {
    let (world_x, world_y) = {
        let mut state = state.write().await;
        let (world_x, world_y) = protocol::map_to_world(map_x as f64, map_y as f64);
        state.player_position.map_x = Some(map_x as f64);
        state.player_position.map_y = Some(map_y as f64);
        state.player_position.world_x = Some(world_x);
        state.player_position.world_y = Some(world_y);
        (world_x, world_y)
    };
    // Declare the step to the server (once per step).
    send_doc_before_generated(outbound_tx, protocol::make_map_point(map_x, map_y)).await?;
    // Hand position to the scheduler so movement updates continue while walking.
    send_scheduler_cmd(
        outbound_tx,
        SchedulerCommand::UpdateMovement {
            world_x,
            world_y,
            is_moving: anim != movement::ANIM_IDLE,
            anim,
            direction,
        },
    )
    .await
}

async fn wait_for_map_position(
    state: &Arc<RwLock<SessionState>>,
    target_map_x: i32,
    target_map_y: i32,
    tolerance: f64,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let state = state.read().await;
            if let (Some(map_x), Some(map_y)) =
                (state.player_position.map_x, state.player_position.map_y)
            {
                let dx = (map_x - target_map_x as f64).abs();
                let dy = (map_y - target_map_y as f64).abs();
                if dx <= tolerance && dy <= tolerance {
                    return Ok(());
                }
            }
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting to reach map position ({target_map_x}, {target_map_y})"
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn movement_doc(state: &Arc<RwLock<SessionState>>, anim: i32, direction: i32) -> Document {
    let state = state.read().await;
    let world_x = state.player_position.world_x.unwrap_or_else(|| {
        protocol::map_to_world(
            tutorial::TUTORIAL_LANDING_X as f64,
            tutorial::TUTORIAL_LANDING_Y as f64,
        )
        .0
    });
    let world_y = state.player_position.world_y.unwrap_or_else(|| {
        protocol::map_to_world(
            tutorial::TUTORIAL_LANDING_X as f64,
            tutorial::TUTORIAL_LANDING_Y as f64,
        )
        .1
    });
    protocol::make_movement_packet(world_x, world_y, anim, direction, false)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bson::{Document, doc};

    use super::{
        BotSession, FishingAutomationState, GrowingTileState, QueuePriority, SchedulerPhase,
        SchedulerState, SendMode, SessionState,
        apply_destroy_block_change, apply_foreground_block_change, is_tile_ready_to_harvest_at,
        update_player_position_from_message,
    };
    use crate::{
        constants::{movement, protocol as ids, timing},
        logging::{EventHub, Logger},
        models::{PlayerPosition, SessionStatus, WorldSnapshot},
        protocol,
    };
    use std::{sync::Arc, time::Duration};

    fn test_state(
        width: u32,
        height: u32,
        foreground_tiles: Vec<u16>,
        background_tiles: Vec<u16>,
    ) -> SessionState {
        SessionState {
            status: SessionStatus::InWorld,
            device_id: String::new(),
            current_host: String::new(),
            current_port: 0,
            current_world: Some("TEST".to_string()),
            pending_world: None,
            pending_world_is_instance: false,
            username: None,
            user_id: None,
            world: Some(WorldSnapshot {
                world_name: Some("TEST".to_string()),
                width,
                height,
                spawn_map_x: None,
                spawn_map_y: None,
                spawn_world_x: None,
                spawn_world_y: None,
                collectables_count: 0,
                world_items_count: 0,
                tile_counts: Vec::new(),
            }),
            world_foreground_tiles: foreground_tiles,
            world_background_tiles: background_tiles,
            world_water_tiles: Vec::new(),
            world_wiring_tiles: Vec::new(),
            current_outbound_tx: None,
            growing_tiles: HashMap::new(),
            player_position: PlayerPosition {
                map_x: None,
                map_y: None,
                world_x: None,
                world_y: None,
            },
            other_players: HashMap::new(),
            inventory: Vec::new(),
            collectables: HashMap::new(),
            last_error: None,
            awaiting_ready: false,
            tutorial_spawn_pod_confirmed: false,
            tutorial_automation_running: false,
            fishing: FishingAutomationState::default(),
            ping_ms: None,
        }
    }

    fn batch_ids(batch: &[Document]) -> Vec<String> {
        batch.iter()
            .map(|doc| doc.get_str("ID").unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn applies_block_placement_to_foreground_tiles() {
        let mut state = test_state(3, 2, vec![0, 0, 0, 0, 0, 0], vec![0, 0, 0, 0, 0, 0]);

        let changed = apply_foreground_block_change(&mut state, 1, 1, 2735);

        assert!(changed);
        assert_eq!(state.world_foreground_tiles[4], 2735);
        let world = state.world.unwrap();
        assert_eq!(
            world
                .tile_counts
                .iter()
                .find(|entry| entry.tile_id == 2735)
                .unwrap()
                .count,
            1
        );
        assert_eq!(
            world
                .tile_counts
                .iter()
                .find(|entry| entry.tile_id == 0)
                .unwrap()
                .count,
            5
        );
    }

    #[test]
    fn applies_block_removal_to_foreground_tiles() {
        let mut state = test_state(2, 2, vec![9, 0, 0, 0], vec![0, 0, 0, 0]);

        let changed = apply_foreground_block_change(&mut state, 0, 0, 0);

        assert!(changed);
        assert_eq!(state.world_foreground_tiles[0], 0);
        let world = state.world.unwrap();
        assert_eq!(world.tile_counts.len(), 1);
        assert_eq!(world.tile_counts[0].tile_id, 0);
        assert_eq!(world.tile_counts[0].count, 4);
    }

    #[test]
    fn destroy_clears_background_when_foreground_is_already_empty() {
        let mut state = test_state(2, 2, vec![0, 0, 0, 0], vec![7, 0, 0, 0]);

        let changed = apply_destroy_block_change(&mut state, 0, 0);

        assert!(changed);
        assert_eq!(state.world_foreground_tiles[0], 0);
        assert_eq!(state.world_background_tiles[0], 0);
    }

    #[test]
    fn destroy_prefers_foreground_before_background() {
        let mut state = test_state(2, 2, vec![9, 0, 0, 0], vec![7, 0, 0, 0]);

        let changed = apply_destroy_block_change(&mut state, 0, 0);

        assert!(changed);
        assert_eq!(state.world_foreground_tiles[0], 0);
        assert_eq!(state.world_background_tiles[0], 7);
        let world = state.world.unwrap();
        assert_eq!(world.tile_counts.len(), 1);
        assert_eq!(world.tile_counts[0].tile_id, 0);
        assert_eq!(world.tile_counts[0].count, 4);
    }

    #[test]
    fn growing_tile_reports_ready_only_after_growth_end_time() {
        let mut state = test_state(2, 2, vec![0, 0, 0, 0], vec![0, 0, 0, 0]);
        state.growing_tiles.insert(
            (1, 1),
            GrowingTileState {
                block_id: 2,
                growth_end_time: 1_000,
                growth_duration_secs: 31,
                mixed: false,
                harvest_seeds: 0,
                harvest_blocks: 5,
                harvest_gems: 0,
                harvest_extra_blocks: 0,
            },
        );

        assert!(!is_tile_ready_to_harvest_at(&state, 1, 1, 999).unwrap());
        assert!(is_tile_ready_to_harvest_at(&state, 1, 1, 1_000).unwrap());
        assert!(is_tile_ready_to_harvest_at(&state, 1, 1, 1_001).unwrap());
    }

    #[test]
    fn growing_tile_query_is_false_when_tile_is_not_tracked() {
        let state = test_state(2, 2, vec![0, 0, 0, 0], vec![0, 0, 0, 0]);

        assert!(!is_tile_ready_to_harvest_at(&state, 1, 1, 1_000).unwrap());
    }

    #[test]
    fn update_player_position_from_message_sets_world_and_map_coordinates() {
        let mut position = PlayerPosition {
            map_x: None,
            map_y: None,
            world_x: None,
            world_y: None,
        };

        let changed = update_player_position_from_message(&doc! { "x": 20.8, "y": 14.88 }, &mut position);
        let (expected_map_x, expected_map_y) = protocol::world_to_map(20.8, 14.88);

        assert!(changed);
        assert_eq!(position.world_x, Some(20.8));
        assert_eq!(position.world_y, Some(14.88));
        assert_eq!(position.map_x, Some(expected_map_x));
        assert_eq!(position.map_y, Some(expected_map_y));
    }

    #[test]
    fn update_player_position_from_message_reports_no_change_for_same_coordinates() {
        let (map_x, map_y) = protocol::world_to_map(20.8, 14.88);
        let mut position = PlayerPosition {
            map_x: Some(map_x),
            map_y: Some(map_y),
            world_x: Some(20.8),
            world_y: Some(14.88),
        };

        let changed = update_player_position_from_message(&doc! { "x": 20.8, "y": 14.88 }, &mut position);

        assert!(!changed);
    }

    #[tokio::test]
    async fn player_leave_removes_tracked_remote_player() {
        let session = BotSession::new(
            "test-session".to_string(),
            crate::models::AuthInput::AndroidDevice {
                device_id: Some("device".to_string()),
            },
            Logger::new(Arc::new(EventHub::new(16))),
        )
        .await;

        {
            let mut state = session.state.write().await;
            state.user_id = Some("local-user".to_string());
            state.other_players.insert(
                "remote-user".to_string(),
                PlayerPosition {
                    map_x: Some(1.0),
                    map_y: Some(2.0),
                    world_x: Some(10.0),
                    world_y: Some(20.0),
                },
            );
        }

        session
            .remove_other_player(&doc! { "U": "remote-user" })
            .await;

        assert!(!session
            .state
            .read()
            .await
            .other_players
            .contains_key("remote-user"));
    }

    #[test]
    fn scheduler_menu_idle_emits_empty_batch_when_idle() {
        let mut scheduler = SchedulerState::new();
        scheduler.set_phase(SchedulerPhase::MenuIdle);
        scheduler.st_due = false;

        let batch = scheduler.take_slot_batch().unwrap();

        assert!(batch.is_empty());
    }

    #[test]
    fn scheduler_mergeable_packets_replace_menu_empty_batch() {
        let mut scheduler = SchedulerState::new();
        scheduler.set_phase(SchedulerPhase::MenuIdle);
        scheduler.st_due = false;
        scheduler.enqueue_packets(
            vec![doc! { "ID": "HB" }],
            SendMode::Mergeable,
            QueuePriority::AfterGenerated,
        );

        let batch = scheduler.take_slot_batch().unwrap();

        assert_eq!(batch_ids(&batch), vec!["HB".to_string()]);
    }

    #[test]
    fn scheduler_world_batch_keeps_pre_and_post_generated_order() {
        let mut scheduler = SchedulerState::new();
        scheduler.set_phase(SchedulerPhase::WorldIdle);
        scheduler.st_due = false;
        scheduler.update_movement(12.8, 9.44, true, movement::ANIM_WALK, movement::DIR_RIGHT);
        scheduler.enqueue_packets(
            vec![protocol::make_map_point(41, 30)],
            SendMode::Mergeable,
            QueuePriority::BeforeGenerated,
        );
        scheduler.enqueue_packets(
            vec![doc! { "ID": "HB" }],
            SendMode::Mergeable,
            QueuePriority::AfterGenerated,
        );

        let batch = scheduler.take_slot_batch().unwrap();

        assert_eq!(
            batch_ids(&batch),
            vec![
                ids::PACKET_ID_MAP_POINT.to_string(),
                ids::PACKET_ID_MOVEMENT.to_string(),
                "HB".to_string(),
            ]
        );
    }

    #[test]
    fn scheduler_world_idle_emits_empty_movement_packet() {
        let mut scheduler = SchedulerState::new();
        scheduler.set_phase(SchedulerPhase::WorldIdle);
        scheduler.st_due = false;

        let batch = scheduler.take_slot_batch().unwrap();

        assert_eq!(batch_ids(&batch), vec![ids::PACKET_ID_MOVEMENT.to_string()]);
    }

    #[test]
    fn scheduler_exclusive_batch_stays_isolated() {
        let mut scheduler = SchedulerState::new();
        scheduler.set_phase(SchedulerPhase::MenuIdle);
        scheduler.st_due = false;
        scheduler.enqueue_packets(
            vec![doc! { "ID": "A" }],
            SendMode::Mergeable,
            QueuePriority::AfterGenerated,
        );
        scheduler.enqueue_packets(
            vec![doc! { "ID": "X" }, doc! { "ID": "Y" }],
            SendMode::ExclusiveBatch,
            QueuePriority::AfterGenerated,
        );

        let first = scheduler.take_slot_batch().unwrap();
        let second = scheduler.take_slot_batch().unwrap();

        assert_eq!(batch_ids(&first), vec!["A".to_string()]);
        assert_eq!(batch_ids(&second), vec!["X".to_string(), "Y".to_string()]);
    }

    #[test]
    fn scheduler_immediate_exclusive_preempts_slot_batches() {
        let mut scheduler = SchedulerState::new();
        scheduler.set_phase(SchedulerPhase::MenuIdle);
        scheduler.st_due = false;
        scheduler.enqueue_packets(
            vec![doc! { "ID": "queued" }],
            SendMode::Mergeable,
            QueuePriority::AfterGenerated,
        );
        scheduler.enqueue_packets(
            vec![doc! { "ID": "now" }],
            SendMode::ImmediateExclusive,
            QueuePriority::AfterGenerated,
        );

        let immediate = scheduler.take_immediate_batch().unwrap();
        let later = scheduler.take_slot_batch().unwrap();

        assert_eq!(batch_ids(&immediate), vec!["now".to_string()]);
        assert_eq!(batch_ids(&later), vec!["queued".to_string()]);
    }

    #[test]
    fn scheduler_menu_keepalive_and_st_share_the_same_slot() {
        let mut scheduler = SchedulerState::new();
        scheduler.set_phase(SchedulerPhase::MenuIdle);
        scheduler.menu_keepalive_due = true;
        scheduler.st_due = true;

        let batch = scheduler.take_slot_batch().unwrap();

        assert_eq!(
            batch_ids(&batch),
            vec![
                ids::PACKET_ID_KEEPALIVE.to_string(),
                ids::PACKET_ID_ST.to_string(),
            ]
        );
    }

    #[test]
    fn scheduler_st_response_keeps_burst_active_until_sample_window_is_full() {
        let mut scheduler = SchedulerState::new();
        scheduler.set_phase(SchedulerPhase::MenuStBurst);
        scheduler.st_sync.sample_count = timing::ST_SAMPLE_COUNT - 1;
        scheduler.st_sync.last_sent_at = Some(std::time::Instant::now() - Duration::from_millis(25));
        scheduler.st_due = false;

        let next = scheduler.handle_st_response();

        assert!(next.is_some());
        assert_eq!(scheduler.phase, SchedulerPhase::MenuIdle);
        assert!(!scheduler.st_due);
    }
}

async fn inventory_key_for(
    state: &Arc<RwLock<SessionState>>,
    block_id: u16,
    inventory_type: Option<u16>,
    fallback: i32,
) -> i32 {
    let state = state.read().await;
    state
        .inventory
        .iter()
        .find(|entry| {
            entry.block_id == block_id
                && inventory_type
                    .map(|expected| entry.inventory_type == expected)
                    .unwrap_or(true)
                && entry.amount > 0
        })
        .map(|entry| entry.inventory_key)
        .unwrap_or(fallback)
}

async fn publish_state_snapshot(
    logger: &Logger,
    session_id: &str,
    state: &Arc<RwLock<SessionState>>,
) {
    let snapshot = {
        let state = state.read().await;
        SessionSnapshot {
            id: session_id.to_string(),
            status: state.status.clone(),
            device_id: state.device_id.clone(),
            current_host: state.current_host.clone(),
            current_port: state.current_port,
            current_world: state.current_world.clone(),
            pending_world: state.pending_world.clone(),
            username: state.username.clone(),
            user_id: state.user_id.clone(),
            world: state.world.clone(),
            player_position: state.player_position.clone(),
            inventory: state
                .inventory
                .iter()
                .map(|e| InventoryItem {
                    block_id: e.block_id,
                    inventory_type: e.inventory_type,
                    amount: e.amount,
                })
                .collect(),
            last_error: state.last_error.clone(),
            ping_ms: state.ping_ms,
        }
    };
    logger.session_snapshot(snapshot);
}

async fn wait_for_collectables(state: &Arc<RwLock<SessionState>>) -> Result<(), String> {
    let deadline = Instant::now() + tutorial::collectable_timeout();
    loop {
        if !state.read().await.collectables.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for tutorial collectables".to_string());
        }
        sleep(Duration::from_millis(200)).await;
    }
}

async fn set_local_map_position(
    logger: &Logger,
    session_id: &str,
    state: &Arc<RwLock<SessionState>>,
    map_x: i32,
    map_y: i32,
) {
    let (world_x, world_y) = protocol::map_to_world(map_x as f64, map_y as f64);
    {
        let mut state = state.write().await;
        state.player_position.map_x = Some(map_x as f64);
        state.player_position.map_y = Some(map_y as f64);
        state.player_position.world_x = Some(world_x);
        state.player_position.world_y = Some(world_y);
    }
    logger.state(
        Some(session_id),
        format!(
            "reflected spawn pot choice at map=({map_x}, {map_y}) world=({world_x:.2}, {world_y:.2})"
        ),
    );
    let snapshot = {
        let state = state.read().await;
        SessionSnapshot {
            id: session_id.to_string(),
            status: state.status.clone(),
            device_id: state.device_id.clone(),
            current_host: state.current_host.clone(),
            current_port: state.current_port,
            current_world: state.current_world.clone(),
            pending_world: state.pending_world.clone(),
            username: state.username.clone(),
            user_id: state.user_id.clone(),
            world: state.world.clone(),
            player_position: state.player_position.clone(),
            inventory: state
                .inventory
                .iter()
                .map(|e| InventoryItem {
                    block_id: e.block_id,
                    inventory_type: e.inventory_type,
                    amount: e.amount,
                })
                .collect(),
            last_error: state.last_error.clone(),
            ping_ms: state.ping_ms,
        }
    };
    logger.session_snapshot(snapshot);
}

async fn collect_all_visible_collectables(
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    collect_all_visible_collectables_cancellable(state, outbound_tx, &cancel).await
}

async fn collect_all_visible_collectables_cancellable(
    state: &Arc<RwLock<SessionState>>,
    outbound_tx: &OutboundHandle,
    cancel: &AtomicBool,
) -> Result<(), String> {
    ensure_not_cancelled(cancel)?;
    let collectables = {
        let mut items = state
            .read()
            .await
            .collectables
            .values()
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.pos_x
                .partial_cmp(&right.pos_x)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        items
    };

    for collectable in collectables {
        ensure_not_cancelled(cancel)?;
        walk_to_map_cancellable(
            state,
            outbound_tx,
            collectable.pos_x.round() as i32,
            collectable.pos_y.round() as i32,
            cancel,
        )
        .await?;
        send_docs(
            outbound_tx,
            vec![protocol::make_collectable_request(
                collectable.collectable_id,
            )],
        )
        .await?;
        sleep(Duration::from_millis(250)).await;
    }

    Ok(())
}
