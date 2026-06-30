use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use app::App;
use domain::ids::TournamentId;

use crate::error::ApiError;


pub(crate) async fn board(State(app): State<Arc<App>>, Path(id): Path<Uuid>) -> Result<Response, ApiError> {
    Ok(Json(app.board(TournamentId(id)).await?).into_response())
}


pub(crate) async fn standings(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    Ok(Json(app.standings(TournamentId(id)).await?).into_response())
}


pub(crate) async fn schedule(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    Ok(Json(app.schedule(TournamentId(id)).await?).into_response())
}

/// SVG QR code pointing at the public read-only view of this tournament.
/// No state needed — it just encodes the public URL derived from the request host.
pub(crate) async fn qr(Path(id): Path<Uuid>, headers: HeaderMap) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:3000");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("https");
    let url = format!("{proto}://{host}/?public={id}");

    match qrcode::QrCode::new(url.as_bytes()) {
        Ok(code) => {
            let svg = code
                .render::<qrcode::render::svg::Color>()
                .min_dimensions(220, 220)
                .quiet_zone(true)
                .build();
            ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}


