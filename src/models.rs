use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthInput {
    Jwt {
        jwt: String,
        device_id: Option<String>,
    },
    EmailPassword {
        email: String,
        password: String,
        device_id: Option<String>,
    },
    AndroidDevice {
        device_id: Option<String>,
    },
}

impl AuthInput {
    pub fn device_id(&self) -> String {
        match self {
            Self::Jwt { device_id, .. } => device_id.clone().unwrap_or_default(),
            Self::EmailPassword { device_id, .. } => device_id.clone().unwrap_or_default(),
            Self::AndroidDevice { device_id } => device_id.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Idle,
    Connecting,
    Authenticating,
    MenuReady,
    JoiningWorld,
    LoadingWorld,
    AwaitingReady,
    InWorld,
    Redirecting,
    Disconnected,
    Error,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Authenticating => "authenticating",
            Self::MenuReady => "menu_ready",
            Self::JoiningWorld => "joining_world",
            Self::LoadingWorld => "loading_world",
            Self::AwaitingReady => "awaiting_ready",
            Self::InWorld => "in_world",
            Self::Redirecting => "redirecting",
            Self::Disconnected => "disconnected",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileCount {
    pub tile_id: u16,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub world_name: Option<String>,
    pub width: u32,
    pub height: u32,
    pub spawn_map_x: Option<f64>,
    pub spawn_map_y: Option<f64>,
    pub spawn_world_x: Option<f64>,
    pub spawn_world_y: Option<f64>,
    pub collectables_count: usize,
    pub world_items_count: usize,
    pub tile_counts: Vec<TileCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerPosition {
    pub map_x: Option<f64>,
    pub map_y: Option<f64>,
    pub world_x: Option<f64>,
    pub world_y: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePlayerSnapshot {
    pub user_id: String,
    pub position: PlayerPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimapSnapshot {
    pub width: u32,
    pub height: u32,
    pub foreground_tiles: Vec<u16>,
    pub background_tiles: Vec<u16>,
    pub water_tiles: Vec<u16>,
    pub wiring_tiles: Vec<u16>,
    pub player_position: PlayerPosition,
    pub other_players: Vec<RemotePlayerSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaCollectableSnapshot {
    pub id: i32,
    pub block_type: i32,
    pub amount: i32,
    pub inventory_type: i32,
    pub pos_x: f64,
    pub pos_y: f64,
    pub is_gem: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaGrowingTileSnapshot {
    pub x: i32,
    pub y: i32,
    pub block_id: u16,
    pub growth_end_time: i64,
    pub growth_duration_secs: i32,
    pub mixed: bool,
    pub harvest_seeds: i32,
    pub harvest_blocks: i32,
    pub harvest_gems: i32,
    pub harvest_extra_blocks: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaTileSnapshot {
    pub foreground: u16,
    pub background: u16,
    pub water: u16,
    pub wiring: u16,
    pub ready_to_harvest: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaWorldSpawnSnapshot {
    pub map_x: Option<f64>,
    pub map_y: Option<f64>,
    pub world_x: Option<f64>,
    pub world_y: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaWorldTilesSnapshot {
    pub foreground: Vec<u16>,
    pub background: Vec<u16>,
    pub water: Vec<u16>,
    pub wiring: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaWorldObjectsSnapshot {
    pub collectables: Vec<LuaCollectableSnapshot>,
    pub growing_tiles: Vec<LuaGrowingTileSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaWorldSnapshot {
    pub name: Option<String>,
    pub width: u32,
    pub height: u32,
    pub spawn: LuaWorldSpawnSnapshot,
    pub tiles: LuaWorldTilesSnapshot,
    pub objects: LuaWorldObjectsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: String,
    pub status: SessionStatus,
    pub device_id: String,
    pub current_host: String,
    pub current_port: u16,
    pub current_world: Option<String>,
    pub pending_world: Option<String>,
    pub username: Option<String>,
    pub user_id: Option<String>,
    pub world: Option<WorldSnapshot>,
    pub player_position: PlayerPosition,
    pub inventory: Vec<InventoryItem>,
    pub last_error: Option<String>,
    pub ping_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryItem {
    pub block_id: u16,
    pub inventory_type: u16,
    pub amount: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMessage {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Log { event: LogEvent },
    Session { snapshot: SessionSnapshot },
    TutorialCompleted { event: TutorialCompletedEvent },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub timestamp_ms: u128,
    pub level: String,
    pub transport: Option<String>,
    pub direction: Option<String>,
    pub scope: String,
    pub session_id: Option<String>,
    pub message: String,
    pub formatted: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TutorialCompletedEvent {
    pub timestamp_ms: u128,
    pub session_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub auth: AuthInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinWorldRequest {
    pub world: String,
    #[serde(default)]
    pub instance: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveDirectionRequest {
    pub direction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WearItemRequest {
    pub block_id: i32,
    pub equip: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PunchRequest {
    pub offset_x: i32,
    pub offset_y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceRequest {
    pub offset_x: i32,
    pub offset_y: i32,
    pub block_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FishingStartRequest {
    pub direction: String,
    pub bait: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TalkRequest {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpamStartRequest {
    pub message: String,
    pub delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaScriptStartRequest {
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropItemRequest {
    pub block_id: i32,
    pub amount: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaScriptStatusSnapshot {
    pub running: bool,
    pub started_at: Option<u128>,
    pub finished_at: Option<u128>,
    pub last_error: Option<String>,
    pub last_result_message: Option<String>,
}
