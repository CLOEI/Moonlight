use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::broadcast;

use crate::models::{LogEvent, ServerEvent, SessionSnapshot, TutorialCompletedEvent};

#[derive(Debug, Clone)]
pub struct EventHub {
    sender: broadcast::Sender<ServerEvent>,
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn emit(&self, event: ServerEvent) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.sender.subscribe()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    State,
}

#[derive(Debug, Clone, Copy)]
pub enum TransportKind {
    Http,
    Tcp,
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone)]
pub struct Logger {
    hub: Arc<EventHub>,
}

impl Logger {
    pub fn new(hub: Arc<EventHub>) -> Self {
        Self { hub }
    }

    pub fn info(&self, scope: &str, session_id: Option<&str>, message: impl Into<String>) {
        self.log(LogLevel::Info, None, None, scope, session_id, message.into());
    }

    pub fn warn(&self, scope: &str, session_id: Option<&str>, message: impl Into<String>) {
        self.log(LogLevel::Warn, None, None, scope, session_id, message.into());
    }

    pub fn error(&self, scope: &str, session_id: Option<&str>, message: impl Into<String>) {
        self.log(LogLevel::Error, None, None, scope, session_id, message.into());
    }

    pub fn state(&self, session_id: Option<&str>, message: impl Into<String>) {
        self.log(LogLevel::State, None, None, "session", session_id, message.into());
    }

    pub fn transport(
        &self,
        transport: TransportKind,
        direction: Direction,
        scope: &str,
        session_id: Option<&str>,
        message: impl Into<String>,
    ) {
        self.log(
            LogLevel::Info,
            Some(transport),
            Some(direction),
            scope,
            session_id,
            message.into(),
        );
    }

    pub fn session_snapshot(&self, snapshot: SessionSnapshot) {
        self.hub.emit(ServerEvent::Session { snapshot });
    }

    pub fn tutorial_completed(&self, session_id: impl Into<String>) {
        let session_id = session_id.into();
        self.hub.emit(ServerEvent::TutorialCompleted {
            event: TutorialCompletedEvent {
                timestamp_ms: now_millis(),
                message: format!("Tutorial for session {session_id} finished."),
                session_id,
            },
        });
    }

    fn log(
        &self,
        level: LogLevel,
        transport: Option<TransportKind>,
        direction: Option<Direction>,
        scope: &str,
        session_id: Option<&str>,
        message: String,
    ) {
        let formatted = format_log_line(level, transport, direction, scope, session_id, &message);
        println!("{formatted}");

        let event = LogEvent {
            timestamp_ms: now_millis(),
            level: level.as_str().to_string(),
            transport: transport.map(|value| value.as_str().to_string()),
            direction: direction.map(|value| value.as_str().to_string()),
            scope: scope.to_string(),
            session_id: session_id.map(ToOwned::to_owned),
            message,
            formatted,
        };
        self.hub.emit(ServerEvent::Log { event });
    }
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::State => "state",
        }
    }
}

impl TransportKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Tcp => "tcp",
        }
    }
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
        }
    }
}

pub fn format_log_line(
    level: LogLevel,
    transport: Option<TransportKind>,
    direction: Option<Direction>,
    scope: &str,
    session_id: Option<&str>,
    message: &str,
) -> String {
    let timestamp = format_timestamp();
    let prefix = format_prefix(level, direction);
    let session = session_id
        .map(|id| format!(" session={id}"))
        .unwrap_or_default();
    let transport = transport
        .map(|value| format!(" {}", value.as_str().to_uppercase()))
        .unwrap_or_default();

    format!("{timestamp} {prefix} [{scope}{transport}]{session} {message}")
}

fn format_prefix(level: LogLevel, direction: Option<Direction>) -> String {
    match direction {
        Some(Direction::Incoming) => colorize("[>]", 32),
        Some(Direction::Outgoing) => colorize("[<]", 36),
        None => match level {
            LogLevel::Info => colorize("[i]", 37),
            LogLevel::Warn => colorize("[!]", 33),
            LogLevel::Error => colorize("[x]", 31),
            LogLevel::State => colorize("[*]", 35),
        },
    }
}

fn colorize(value: &str, color: u8) -> String {
    format!("\x1b[{color}m{value}\x1b[0m")
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

fn format_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = now.as_secs() as i64;
    let millis = now.subsec_millis();

    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;

    format!(
        "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03} UTC"
    )
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };

    (year as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::{format_log_line, Direction, LogLevel, TransportKind};

    #[test]
    fn incoming_uses_requested_prefix() {
        let line = format_log_line(
            LogLevel::Info,
            Some(TransportKind::Tcp),
            Some(Direction::Incoming),
            "tcp",
            Some("s1"),
            "hello",
        );
        assert!(line.contains("[>]"));
    }

    #[test]
    fn outgoing_uses_requested_prefix() {
        let line = format_log_line(
            LogLevel::Info,
            Some(TransportKind::Http),
            Some(Direction::Outgoing),
            "http",
            None,
            "hello",
        );
        assert!(line.contains("[<]"));
    }

    #[test]
    fn prefixes_are_colored() {
        let line = format_log_line(
            LogLevel::Warn,
            None,
            Some(Direction::Incoming),
            "scope",
            None,
            "hello",
        );
        assert!(line.contains("\x1b["));
    }
}
