use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use app::App;
use domain::ids::TournamentId;
use domain::tournament::TournamentCommand;

use crate::dto::*;
use crate::error::ApiError;


pub(crate) async fn list_tournaments(State(app): State<Arc<App>>) -> Result<Response, ApiError> {
    Ok(Json(app.list_tournaments().await?).into_response())
}


pub(crate) async fn create_tournament(
    State(app): State<Arc<App>>,
    Json(body): Json<CreateTournament>,
) -> Result<Response, ApiError> {
    let id = TournamentId::new();
    app.tournament(
        id,
        TournamentCommand::Create {
            tournament_id: id,
            name: body.name,
            pool_format: body.pool_format,
            bracket_format: body.bracket_format,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(IdResponse { id: id.0 })).into_response())
}


pub(crate) async fn get_tournament(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    match app.tournament_view(TournamentId(id)).await? {
        Some(view) => Ok(Json(view).into_response()),
        None => Err(ApiError::not_found("tournament")),
    }
}


pub(crate) async fn delete_tournament(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.delete_tournament(TournamentId(id)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn reset_tournament(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.reset_tournament(TournamentId(id)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


