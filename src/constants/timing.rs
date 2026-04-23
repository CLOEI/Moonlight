use std::time::Duration;

pub const SEND_SLOT_MS: u64 = 250;
pub const MENU_KEEPALIVE_MS: u64 = 5_000;
pub const HTTP_TIMEOUT_SECS: u64 = 15;

// Adaptive time-sync constants (mirrors KukouriTime SetTimeOffset algorithm)
pub const ST_INTERVAL_INIT_SECS: i32 = 5;
pub const ST_INTERVAL_MIN_SECS: i32 = 5;
pub const ST_INTERVAL_MAX_SECS: i32 = 300;
pub const ST_INTERVAL_STEP_SECS: i32 = 20;
pub const ST_SAMPLE_COUNT: usize = 15;
pub const ST_SUCCESS_THRESHOLD: i32 = 15;

pub fn send_slot_interval() -> Duration {
    Duration::from_millis(SEND_SLOT_MS)
}

pub fn menu_keepalive_interval() -> Duration {
    Duration::from_millis(MENU_KEEPALIVE_MS)
}

pub fn http_timeout() -> Duration {
    Duration::from_secs(HTTP_TIMEOUT_SECS)
}
