use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use tokio::sync::{broadcast, RwLock};
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
};

use crate::{
    dashboard_auth::DashboardAuthManager,
    logging::{Direction, EventHub, Logger, TransportKind},
    models::{
        ApiMessage, CreateSessionRequest, FishingStartRequest, JoinWorldRequest,
        LuaScriptStartRequest, MoveDirectionRequest, PlaceRequest, PunchRequest, ServerEvent,
        SpamStartRequest, TalkRequest, WearItemRequest,
    },
    session::SessionManager,
};

#[derive(Clone)]
struct MovementCooldownState {
    last_move_time: Option<Instant>,
    blocked_attempt_count: u32,
    blocked_attempts_reset_time: Option<Instant>,
}

impl MovementCooldownState {
    fn new() -> Self {
        Self {
            last_move_time: None,
            blocked_attempt_count: 0,
            blocked_attempts_reset_time: None,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub session_manager: SessionManager,
    pub logger: Logger,
    pub event_hub: Arc<EventHub>,
    pub dashboard_auth: DashboardAuthManager,
    pub movement_cooldowns: Arc<RwLock<HashMap<String, MovementCooldownState>>>,
}

impl AppState {
    pub fn new(
        session_manager: SessionManager,
        logger: Logger,
        event_hub: Arc<EventHub>,
        dashboard_auth: DashboardAuthManager,
    ) -> Self {
        Self {
            session_manager,
            logger,
            event_hub,
            dashboard_auth,
            movement_cooldowns: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dist_dir = project_root.join("dist");
    let dist_index = dist_dir.join("index.html");
    let block_types_file = project_root.join("block_types.json");

    Router::new()
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/register", post(auth_register))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/connect", post(connect_with_auth))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}", get(get_session))
        .route("/api/sessions/{id}/connect", post(connect_session))
        .route("/api/sessions/{id}/join", post(join_world))
        .route("/api/sessions/{id}/leave", post(leave_world))
        .route("/api/sessions/{id}/disconnect", post(disconnect_session))
        .route("/api/sessions/{id}/reconnect", post(reconnect_session))
        .route("/api/sessions/{id}/move", post(move_session))
        .route("/api/sessions/{id}/punch", post(punch_session))
        .route("/api/sessions/{id}/place", post(place_session))
        .route("/api/sessions/{id}/wear", post(wear_item))
        .route(
            "/api/sessions/{id}/tutorial/automate",
            post(automate_tutorial),
        )
        .route("/api/sessions/{id}/fishing/start", post(start_fishing))
        .route("/api/sessions/{id}/fishing/stop", post(stop_fishing))
        .route("/api/sessions/{id}/talk", post(talk))
        .route("/api/sessions/{id}/spam/start", post(start_spam))
        .route("/api/sessions/{id}/spam/stop", post(stop_spam))
        .route("/api/sessions/{id}/lua/start", post(start_lua_script))
        .route("/api/sessions/{id}/lua/stop", post(stop_lua_script))
        .route("/api/sessions/{id}/lua/status", get(get_lua_status))
        .route("/api/sessions/{id}/minimap", get(get_minimap))
        .route("/ws", get(websocket_handler))
        .route_service("/block_types.json", ServeFile::new(block_types_file))
        .fallback_service(ServeDir::new(dist_dir).not_found_service(ServeFile::new(dist_index)))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers(Any),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            http_log_middleware,
        ))
        .with_state(state)
}

async fn http_log_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    state.logger.transport(
        TransportKind::Http,
        Direction::Incoming,
        "http_server",
        None,
        format!("{method} {path}"),
    );

    let response = next.run(request).await;
    state.logger.transport(
        TransportKind::Http,
        Direction::Outgoing,
        "http_server",
        None,
        format!("{method} {path} -> {}", response.status()),
    );
    response
}

async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    let is_auth_path = path.starts_with("/api/auth/");
    let is_api_path = path.starts_with("/api/");
    let is_ws_path = path == "/ws";

    if !is_api_path && !is_ws_path {
        return next.run(request).await;
    }

    if is_auth_path {
        return next.run(request).await;
    }

    let token = if is_ws_path {
        token_from_query(request.uri().query())
    } else {
        token_from_headers(request.headers())
    };

    if state.dashboard_auth.is_authorized(token.as_deref()).await {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "message": "unauthorized"
            })),
        )
            .into_response()
    }
}

fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("authorization")?.to_str().ok()?;
    let value = value.trim();
    if let Some(raw) = value.strip_prefix("Bearer ") {
        let token = raw.trim();
        if token.is_empty() {
            None
        } else {
            Some(token.to_string())
        }
    } else {
        None
    }
}

fn token_from_query(query: Option<&str>) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next()?;
        if key == "token" {
            let value = parts.next().unwrap_or_default();
            if value.is_empty() {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

async fn auth_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let token = token_from_headers(&headers);
    let status = state.dashboard_auth.status(token.as_deref()).await;
    Json(json!({
        "registered": status.registered,
        "authenticated": status.authenticated,
    }))
}

#[derive(serde::Deserialize)]
struct PasswordRequest {
    password: String,
}

async fn auth_register(
    State(state): State<AppState>,
    Json(request): Json<PasswordRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let token = state
        .dashboard_auth
        .register(request.password)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "ok": true,
        "message": "dashboard password created",
        "token": token,
    })))
}

async fn auth_login(
    State(state): State<AppState>,
    Json(request): Json<PasswordRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let token = state
        .dashboard_auth
        .login(request.password)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "ok": true,
        "message": "dashboard unlocked",
        "token": token,
    })))
}

async fn auth_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Some(token) = token_from_headers(&headers) {
        state.dashboard_auth.logout(&token).await;
    }
    Ok(Json(json!({
        "ok": true,
        "message": "logged out",
    })))
}

async fn connect_with_auth(
    State(state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state.session_manager.create_session(request.auth).await;
    session.connect().await.map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message: "session created and connect queued".to_string() },
        "session": session.snapshot().await
    })))
}

async fn list_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "sessions": state.session_manager.list_snapshots().await }))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    Ok(Json(json!({ "session": session.snapshot().await })))
}

async fn connect_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    session.connect().await.map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message: "connect queued".to_string() },
        "session": session.snapshot().await
    })))
}

async fn join_world(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<JoinWorldRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    session
        .join_world(request.world, request.instance)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message: "join queued".to_string() },
        "session": session.snapshot().await
    })))
}

async fn leave_world(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    session.leave_world().await.map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message: "leave queued".to_string() },
        "session": session.snapshot().await
    })))
}

async fn disconnect_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    session.disconnect().await.map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message: "disconnect queued".to_string() },
        "session": session.snapshot().await
    })))
}

async fn reconnect_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    session.reconnect().await.map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message: "reconnect queued".to_string() },
        "session": session.snapshot().await
    })))
}

async fn move_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<MoveDirectionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let now = Instant::now();
    let mut cooldowns = state.movement_cooldowns.write().await;
    let cooldown_state = cooldowns.entry(id.clone()).or_insert_with(MovementCooldownState::new);

    if let Some(reset_time) = cooldown_state.blocked_attempts_reset_time {
        if now.duration_since(reset_time) > Duration::from_secs(20) {
            cooldown_state.blocked_attempt_count = 0;
            cooldown_state.blocked_attempts_reset_time = None;
        }
    }

    if cooldown_state.blocked_attempt_count >= 2 {
        return Err(ApiError::bad_request(format!(
            "too many blocked attempts recently ({})",
            cooldown_state.blocked_attempt_count
        )));
    }

    if let Some(last_time) = cooldown_state.last_move_time {
        let elapsed = now.duration_since(last_time);
        if elapsed < Duration::from_millis(1500) {
            return Err(ApiError::bad_request(format!(
                "movement cooldown active ({:?} elapsed)",
                elapsed
            )));
        }
    }

    cooldown_state.last_move_time = Some(now);
    drop(cooldowns);

    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .queue_move_direction(&request.direction)
        .await
        .map_err(ApiError::bad_request)?;

    if message.contains("cannot move") {
        let mut cooldowns = state.movement_cooldowns.write().await;
        if let Some(cooldown_state) = cooldowns.get_mut(&id) {
            cooldown_state.blocked_attempt_count += 1;
            if cooldown_state.blocked_attempts_reset_time.is_none() {
                cooldown_state.blocked_attempts_reset_time = Some(now);
            }
        }
    }

    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn wear_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<WearItemRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .queue_wear_item(request.block_id, request.equip)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn punch_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<PunchRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .queue_punch(request.offset_x, request.offset_y)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn place_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<PlaceRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .queue_place(request.offset_x, request.offset_y, request.block_id)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn automate_tutorial(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .automate_tutorial()
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn get_minimap(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let minimap = session
        .minimap_snapshot()
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({ "minimap": minimap })))
}

async fn start_fishing(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<FishingStartRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .queue_start_fishing(&request.direction, &request.bait)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn stop_fishing(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .queue_stop_fishing()
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn talk(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<TalkRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .queue_talk(&request.message)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn start_spam(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<SpamStartRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .queue_start_spam(&request.message, request.delay_ms)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn stop_spam(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .queue_stop_spam()
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
}

async fn start_lua_script(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<LuaScriptStartRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let status = state
        .session_manager
        .start_lua_script(&id, request.source)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message: "lua script started".to_string() },
        "status": status
    })))
}

async fn stop_lua_script(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let status = state
        .session_manager
        .stop_lua_script(&id)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message: "lua stop requested".to_string() },
        "status": status
    })))
}

async fn get_lua_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let status = state
        .session_manager
        .lua_script_status(&id)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({ "status": status })))
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| websocket_session(socket, state.event_hub.subscribe()))
}

async fn websocket_session(mut socket: WebSocket, mut rx: broadcast::Receiver<ServerEvent>) {
    while let Ok(event) = rx.recv().await {
        let payload = match serde_json::to_string(&event) {
            Ok(payload) => payload,
            Err(_) => continue,
        };

        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }
}

#[derive(Debug, Clone)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn not_found(message: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "ok": false,
                "message": self.message,
            })),
        )
            .into_response()
    }
}
