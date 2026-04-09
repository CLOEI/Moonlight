use std::time::Duration;

pub const TUTORIAL_WORLD: &str = "TUTORIAL2";
pub const POST_TUTORIAL_WORLD: &str = "PIXELSTATION";

pub const TUTORIAL_GENDER: i32 = 0;
pub const TUTORIAL_COUNTRY: i32 = 999;
pub const TUTORIAL_SKIN_COLOR: i32 = 7;

pub const PRE_CHARACTER_POD_SELECTION: [i32; 2] = [2, 20];
pub const STARTER_FACE_BLOCK: i32 = 527;
pub const STARTER_HAIR_BLOCK: i32 = 515;
pub const POST_CHARACTER_POD_CONFIRMATION: [i32; 2] = [10, 5];

// Sleeping pod spawn: map (39, 44) = world (12.48, 13.92).
// The GWC WorldStartPoint for TUTORIAL2 is (40, 30) which is the generic
// visitor spawn; new accounts ignore it and always spawn here instead.
// Source: packets.bin rec 32 — mp pM=(39,44) + mP x=12.48 y=13.92 tp=true.
pub const TUTORIAL_SPAWN_MAP_X: i32 = 39;
pub const TUTORIAL_SPAWN_MAP_Y: i32 = 44;

// Pod selection tile the player walks to after character creation.
// Source: packets.bin rec 230 — mp pM=(42,44).
pub const SPAWN_POT_MAP_X: i32 = 42;
pub const SPAWN_POT_MAP_Y: i32 = 44;

pub const CLOTHES_PACK_ID: &str = "BasicClothes";
pub const CLOTHES_PACK_AE: i32 = 6;
pub const EQUIP_BLOCKS: [i32; 3] = [553, 349, 356];

pub const SOIL_BLOCK_ID: i32 = 2735;
pub const FERTILIZER_BLOCK_ID: i32 = 1070;
pub const SEED_INVENTORY_TYPE: u16 = 512;
pub const FERTILIZER_INVENTORY_TYPE: u16 = 512;

pub const BUILD_TARGETS: [(i32, i32); 4] = [(66, 39), (67, 39), (67, 40), (66, 40)];
pub const FARM_TARGET_X: i32 = 64;
pub const FARM_TARGET_Y: i32 = 39;

pub const PORTAL_APPROACH_X: i32 = 46;
pub const PORTAL_APPROACH_Y: i32 = 45;
pub const PORTAL_ENTRY_X: i32 = 65;
pub const PORTAL_ENTRY_Y: i32 = 47;
pub const TUTORIAL_LANDING_X: i32 = 65;
pub const TUTORIAL_LANDING_Y: i32 = 39;

pub const INTRO_PORTAL_WALK_PATH: &[(i32, i32)] = &[
    (40, 44),
    (41, 44),
    (42, 44),
    (43, 44),
    (43, 45),
    (44, 45),
    (44, 46),
    (45, 45),
    (46, 45),
];

pub fn short_pause() -> Duration {
    Duration::from_millis(350)
}

pub fn walk_step_pause() -> Duration {
    Duration::from_millis(180)
}

pub fn medium_pause() -> Duration {
    Duration::from_millis(750)
}

pub fn spawn_pod_confirm_timeout() -> Duration {
    Duration::from_secs(6)
}

pub fn spawn_pod_settle_pause() -> Duration {
    Duration::from_millis(2_500)
}

pub fn long_pause() -> Duration {
    Duration::from_millis(1_500)
}

pub fn world_join_timeout() -> Duration {
    Duration::from_secs(25)
}

pub fn collectable_timeout() -> Duration {
    Duration::from_secs(8)
}

pub fn portal_transition_timeout() -> Duration {
    Duration::from_secs(6)
}
