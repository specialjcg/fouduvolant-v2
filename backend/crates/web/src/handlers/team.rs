use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;

use app::App;
use domain::ids::{TeamId, TournamentId};
use domain::tournament::TournamentCommand;

use crate::dto::*;
use crate::error::ApiError;


pub(crate) async fn register_team(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<TeamBody>,
) -> Result<Response, ApiError> {
    let team_id = TeamId::new();
    app.tournament(
        TournamentId(id),
        TournamentCommand::RegisterTeam {
            team_id,
            name: body.name,
            player1: body.player1,
            player2: body.player2,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(IdResponse { id: team_id.0 })).into_response())
}


pub(crate) async fn import_teams(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ImportTeams>,
) -> Result<Response, ApiError> {
    let mut created = 0;
    for raw in body.names {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        app.tournament(
            TournamentId(id),
            TournamentCommand::RegisterTeam {
                team_id: TeamId::new(),
                name: name.to_string(),
                player1: String::new(),
                player2: String::new(),
            },
        )
        .await?;
        created += 1;
    }
    Ok((StatusCode::CREATED, Json(ImportResult { created })).into_response())
}


pub(crate) async fn remove_team(
    State(app): State<Arc<App>>,
    Path((id, team_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    app.tournament(
        TournamentId(id),
        TournamentCommand::RemoveTeam {
            team_id: TeamId(team_id),
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}


