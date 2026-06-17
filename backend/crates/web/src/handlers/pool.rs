use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use app::App;
use domain::ids::{CourtId, PoolId, TeamId, TournamentId};
use domain::tournament::{Pool, TournamentCommand};

use crate::dto::*;
use crate::error::ApiError;


pub(crate) async fn generate_pools(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<GeneratePools>,
) -> Result<Response, ApiError> {
    let pools = body
        .pools
        .into_iter()
        .map(|p| Pool {
            id: PoolId::new(),
            name: p.name,
            teams: p.teams.into_iter().map(TeamId).collect(),
        })
        .collect();
    app.tournament(TournamentId(id), TournamentCommand::GeneratePools { pools })
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn generate_pool_matches(
    State(app): State<Arc<App>>,
    Path((id, pool_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let created = app
        .generate_pool_matches(TournamentId(id), PoolId(pool_id))
        .await?;
    Ok((StatusCode::CREATED, Json(CreatedResponse { created })).into_response())
}


pub(crate) async fn assign_pool_court(
    State(app): State<Arc<App>>,
    Path((id, pool_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<AssignCourt>,
) -> Result<Response, ApiError> {
    app.tournament(
        TournamentId(id),
        TournamentCommand::AssignPoolCourt {
            pool_id: PoolId(pool_id),
            court_id: CourtId(body.court_id),
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn configure_courts(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ConfigureCourts>,
) -> Result<Response, ApiError> {
    let courts: Vec<CourtId> = (0..body.count).map(|_| CourtId::new()).collect();
    app.tournament(
        TournamentId(id),
        TournamentCommand::ConfigureCourts {
            courts: courts.clone(),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(CourtsResponse { courts })).into_response())
}


pub(crate) async fn redo_pools(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.redo_pools(TournamentId(id)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


pub(crate) async fn start_pools(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.tournament(TournamentId(id), TournamentCommand::StartPoolPhase)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


