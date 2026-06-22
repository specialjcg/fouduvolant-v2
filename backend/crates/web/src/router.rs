//! Route table and static-asset serving.

use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use app::App;

use crate::handlers::board::*;
use crate::handlers::bracket::*;
use crate::handlers::matches::*;
use crate::handlers::pool::*;
use crate::handlers::team::*;
use crate::handlers::tournament::*;

pub(crate) fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/tournaments", get(list_tournaments).post(create_tournament))
        .route(
            "/tournaments/{id}",
            get(get_tournament).delete(delete_tournament),
        )
        .route("/tournaments/{id}/teams", post(register_team))
        .route("/tournaments/{id}/teams/import", post(import_teams))
        .route("/tournaments/{id}/teams/{team_id}", axum::routing::delete(remove_team))
        .route("/tournaments/{id}/teams/{team_id}/forfeit", post(forfeit_team))
        .route("/tournaments/{id}/pools", post(generate_pools))
        .route("/tournaments/{id}/pools/{pool_id}/matches", post(generate_pool_matches))
        .route("/tournaments/{id}/pools/{pool_id}/court", post(assign_pool_court))
        .route("/tournaments/{id}/courts", post(configure_courts))
        .route("/tournaments/{id}/reset", post(reset_tournament))
        .route("/tournaments/{id}/redo-pools", post(redo_pools))
        .route("/tournaments/{id}/start-pools", post(start_pools))
        .route("/tournaments/{id}/start-bracket", post(start_bracket))
        .route("/tournaments/{id}/matches", post(schedule_match))
        .route("/tournaments/{id}/dispatch", post(dispatch))
        .route("/tournaments/{id}/board", get(board))
        .route("/tournaments/{id}/standings", get(standings))
        .route("/tournaments/{id}/schedule", get(schedule))
        .route("/tournaments/{id}/bracket", get(get_bracket).post(generate_bracket))
        .route("/tournaments/{id}/bracket/advance", post(advance_bracket))
        .route("/tournaments/{id}/bracket/reset", post(reset_bracket))
        .route("/tournaments/{id}/bracket-format", post(set_bracket_format))
        .route("/tournaments/{id}/bracket-round-format", post(set_bracket_round_format))
        .route("/matches/{id}/start", post(start_match))
        .route("/matches/{id}/sets", post(record_set))
        .route("/matches/{id}/rescore", post(rescore))
        .route("/matches/{id}/reset", post(reset_match))
        .route("/matches/{id}/unstart", post(unstart_match))
        .route("/matches/{id}/concede", post(concede_match))
        // Serve the built frontend (index.html, elm.js) for any non-API path.
        .fallback_service(ServeDir::new(static_dir()))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(app)
}

/// Directory of static frontend assets, overridable via `STATIC_DIR`.
fn static_dir() -> String {
    std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string())
}

/// Serve `index.html` with `no-cache` so the browser always revalidates the HTML
/// and picks up the current cache-busted `elm.js?v=…` (avoids stale UI).
async fn serve_index() -> Response {
    let path = format!("{}/index.html", static_dir());
    match tokio::fs::read_to_string(&path).await {
        Ok(html) => (
            [
                (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL, "no-cache"),
            ],
            html,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
