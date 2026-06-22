use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use app::App;
use domain::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
use domain::matches::MatchCommand;

use crate::dto::*;
use crate::error::ApiError;


pub(crate) async fn schedule_match(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ScheduleMatch>,
) -> Result<Response, ApiError> {
    let match_id = MatchId::new();
    app.match_cmd(
        match_id,
        MatchCommand::Schedule {
            match_id,
            tournament_id: TournamentId(id),
            format: body.format,
            team_a: TeamId(body.team_a),
            team_b: TeamId(body.team_b),
            pool_id: body.pool_id.map(PoolId),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(IdResponse { id: match_id.0 })).into_response())
}


pub(crate) async fn start_match(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<StartMatch>,
) -> Result<Response, ApiError> {
    app.start_match(MatchId(id), CourtId(body.court_id)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn record_set(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<RecordSetBody>,
) -> Result<Response, ApiError> {
    app.record_set(MatchId(id), body.a, body.b).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn rescore(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<RecordSetBody>,
) -> Result<Response, ApiError> {
    app.rescore_match(MatchId(id), body.a, body.b).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn reset_match(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.reset_bracket_match(MatchId(id)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn unstart_match(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.unstart_match(MatchId(id)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn concede_match(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ConcedeBody>,
) -> Result<Response, ApiError> {
    app.concede_match(MatchId(id), TeamId(body.winner)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn dispatch(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let started = app.dispatch_courts(TournamentId(id)).await?;
    Ok(Json(DispatchResponse { started }).into_response())
}


