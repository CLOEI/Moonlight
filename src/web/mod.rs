use std::{path::PathBuf, sync::Arc};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use tokio::sync::broadcast;
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
};

use crate::{
    logging::{Direction, EventHub, Logger, TransportKind},
    models::{
        ApiMessage, CreateSessionRequest, FishingStartRequest, JoinWorldRequest, MoveDirectionRequest,
        PlaceRequest, PunchRequest, ServerEvent, SpamStartRequest, TalkRequest, WearItemRequest,
    },
    session::SessionManager,
};

#[derive(Clone)]
pub struct AppState {
    pub session_manager: SessionManager,
    pub logger: Logger,
    pub event_hub: Arc<EventHub>,
}

impl AppState {
    pub fn new(
        session_manager: SessionManager,
        logger: Logger,
        event_hub: Arc<EventHub>,
    ) -> Self {
        Self {
            session_manager,
            logger,
            event_hub,
        }
    }
}

pub fn router(state: AppState) -> Router {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dist_dir = project_root.join("dist");
    let dist_index = dist_dir.join("index.html");
    let block_types_file = project_root.join("block_types.json");

    Router::new()
        .route("/api/connect", post(connect_with_auth))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{id}", get(get_session))
        .route("/api/sessions/{id}/connect", post(connect_session))
        .route("/api/sessions/{id}/join", post(join_world))
        .route("/api/sessions/{id}/leave", post(leave_world))
        .route("/api/sessions/{id}/disconnect", post(disconnect_session))
        .route("/api/sessions/{id}/move", post(move_session))
        .route("/api/sessions/{id}/punch", post(punch_session))
        .route("/api/sessions/{id}/place", post(place_session))
        .route("/api/sessions/{id}/wear", post(wear_item))
        .route("/api/sessions/{id}/tutorial/automate", post(automate_tutorial))
        .route("/api/sessions/{id}/fishing/start", post(start_fishing))
        .route("/api/sessions/{id}/fishing/stop", post(stop_fishing))
        .route("/api/sessions/{id}/talk", post(talk))
        .route("/api/sessions/{id}/spam/start", post(start_spam))
        .route("/api/sessions/{id}/spam/stop", post(stop_spam))
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
        .layer(middleware::from_fn_with_state(state.clone(), http_log_middleware))
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
        .join_world(request.world)
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
    session
        .disconnect()
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message: "disconnect queued".to_string() },
        "session": session.snapshot().await
    })))
}

async fn move_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<MoveDirectionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let message = session
        .move_direction(&request.direction)
        .await
        .map_err(ApiError::bad_request)?;
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
        .wear_item(request.block_id, request.equip)
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
        .punch(request.offset_x, request.offset_y)
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
        .place(request.offset_x, request.offset_y, request.block_id)
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
    let minimap = session.minimap_snapshot().await.map_err(ApiError::bad_request)?;
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
        .start_fishing(&request.direction, &request.bait)
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
    let message = session.stop_fishing().await.map_err(ApiError::bad_request)?;
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
    let message = session.talk(&request.message).await.map_err(ApiError::bad_request)?;
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
        .start_spam(&request.message, request.delay_ms)
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
    let message = session.stop_spam().await.map_err(ApiError::bad_request)?;
    Ok(Json(json!({
        "result": ApiMessage { ok: true, message },
        "session": session.snapshot().await
    })))
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
