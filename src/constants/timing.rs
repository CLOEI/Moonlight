use std::time::Duration;

pub const ST_INTERVAL_MS: u64 = 1_000;
pub const KEEPALIVE_INTERVAL_MS: u64 = 5_000;
pub const FLUSH_INTERVAL_MS: u64 = 50;
pub const HTTP_TIMEOUT_SECS: u64 = 15;

pub fn st_interval() -> Duration {
    Duration::from_millis(ST_INTERVAL_MS)
}

pub fn keepalive_interval() -> Duration {
    Duration::from_millis(KEEPALIVE_INTERVAL_MS)
}

pub fn flush_interval() -> Duration {
    Duration::from_millis(FLUSH_INTERVAL_MS)
}

pub fn http_timeout() -> Duration {
    Duration::from_secs(HTTP_TIMEOUT_SECS)
}
