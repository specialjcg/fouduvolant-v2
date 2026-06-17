use std::sync::Arc;

use axum::{
    extract::{Path, State},
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


