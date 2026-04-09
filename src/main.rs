mod auth;
mod constants;
mod logging;
mod models;
mod net;
mod pathfinding;
mod protocol;
mod session;
mod web;
mod world;

use std::sync::Arc;

use logging::{EventHub, Logger};
use session::SessionManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_hub = Arc::new(EventHub::new(2048));
    let logger = Logger::new(event_hub.clone());
    let session_manager = SessionManager::new(logger.clone());
    let app_state = web::AppState::new(session_manager, logger.clone(), event_hub);
    let app = web::router(app_state.clone());

    let bind_addr = constants::network::dashboard_bind_addr();
    logger.state(None, format!("dashboard listening on http://{bind_addr}"));

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
