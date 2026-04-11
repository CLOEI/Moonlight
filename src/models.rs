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
pub struct MinimapSnapshot {
    pub width: u32,
    pub height: u32,
    pub foreground_tiles: Vec<u16>,
    pub background_tiles: Vec<u16>,
    pub water_tiles: Vec<u16>,
    pub wiring_tiles: Vec<u16>,
    pub player_position: PlayerPosition,
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
