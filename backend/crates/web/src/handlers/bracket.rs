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


pub(crate) async fn start_bracket(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.tournament(TournamentId(id), TournamentCommand::StartBracketPhase)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn get_bracket(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    Ok(Json(app.bracket_view(TournamentId(id)).await?).into_response())
}


pub(crate) async fn generate_bracket(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<GenerateBracket>,
) -> Result<Response, ApiError> {
    let created = app
        .generate_bracket(TournamentId(id), body.per_pool)
        .await?;
    Ok((StatusCode::CREATED, Json(CreatedResponse { created })).into_response())
}


pub(crate) async fn advance_bracket(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let created = app.advance_bracket(TournamentId(id)).await?;
    Ok(Json(CreatedResponse { created }).into_response())
}


pub(crate) async fn reset_bracket(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.reset_bracket(TournamentId(id)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn set_bracket_format(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetFormatBody>,
) -> Result<Response, ApiError> {
    app.tournament(
        TournamentId(id),
        TournamentCommand::SetBracketFormat { format: body.format },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn set_bracket_round_format(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetRoundFormatBody>,
) -> Result<Response, ApiError> {
    app.tournament(
        TournamentId(id),
        TournamentCommand::SetBracketRoundFormat { round_size: body.round_size, format: body.format },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


