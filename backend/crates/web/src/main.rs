//! HTTP API for fouduvolant — a thin axum layer over the [`app`] command and
//! read methods. Each handler deserialises a request, issues a domain command or
//! read, and maps the result to JSON. Identifiers for new aggregates are
//! generated server-side and returned in the response.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use app::{AggregateError, App, AppError};
use domain::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
use domain::matches::MatchCommand;
use domain::score::MatchFormat;
use domain::tournament::{Pool, TournamentCommand};

const DEFAULT_DATABASE_URL: &str =
    "postgresql://fouduvolant:fouduvolant@localhost:5432/fouduvolant";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .init();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
    let app = App::connect(&database_url).await;
    app.run_migrations()
        .await
        .expect("event-store migrations failed");

    let router = router(Arc::new(app));

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".into());
    let host = std::env::var("BIND_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    tracing::info!("listening on {addr}");
    axum::serve(listener, router).await.expect("server error");
}

fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/tournaments", get(list_tournaments).post(create_tournament))
        .route("/tournaments/{id}", get(get_tournament))
        .route("/tournaments/{id}/teams", post(register_team))
        .route("/tournaments/{id}/teams/{team_id}", axum::routing::delete(remove_team))
        .route("/tournaments/{id}/pools", post(generate_pools))
        .route("/tournaments/{id}/pools/{pool_id}/matches", post(generate_pool_matches))
        .route("/tournaments/{id}/pools/{pool_id}/court", post(assign_pool_court))
        .route("/tournaments/{id}/courts", post(configure_courts))
        .route("/tournaments/{id}/start-pools", post(start_pools))
        .route("/tournaments/{id}/start-bracket", post(start_bracket))
        .route("/tournaments/{id}/matches", post(schedule_match))
        .route("/tournaments/{id}/dispatch", post(dispatch))
        .route("/tournaments/{id}/board", get(board))
        .route("/tournaments/{id}/standings", get(standings))
        .route("/tournaments/{id}/bracket", get(get_bracket).post(generate_bracket))
        .route("/tournaments/{id}/bracket/advance", post(advance_bracket))
        .route("/matches/{id}/start", post(start_match))
        .route("/matches/{id}/sets", post(record_set))
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

// ---- Requests / responses ----

#[derive(Deserialize)]
struct CreateTournament {
    name: String,
    pool_format: MatchFormat,
    bracket_format: MatchFormat,
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

#[derive(Deserialize)]
struct PoolInput {
    name: String,
    teams: Vec<Uuid>,
}

#[derive(Deserialize)]
struct GeneratePools {
    pools: Vec<PoolInput>,
}

#[derive(Deserialize)]
struct ConfigureCourts {
    count: usize,
}

#[derive(Deserialize)]
struct AssignCourt {
    court_id: Uuid,
}

#[derive(Deserialize)]
struct ScheduleMatch {
    format: MatchFormat,
    team_a: Uuid,
    team_b: Uuid,
    pool_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct StartMatch {
    court_id: Uuid,
}

#[derive(Deserialize)]
struct RecordSetBody {
    a: u8,
    b: u8,
}

#[derive(Serialize)]
struct IdResponse {
    id: Uuid,
}

#[derive(Serialize)]
struct CourtsResponse {
    courts: Vec<CourtId>,
}

#[derive(Serialize)]
struct DispatchResponse {
    started: Vec<MatchId>,
}

#[derive(Serialize)]
struct CreatedResponse {
    created: Vec<MatchId>,
}

#[derive(Deserialize)]
struct GenerateBracket {
    per_pool: usize,
}

// ---- Handlers ----

async fn list_tournaments(State(app): State<Arc<App>>) -> Result<Response, ApiError> {
    Ok(Json(app.list_tournaments().await?).into_response())
}

async fn create_tournament(
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

async fn get_tournament(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    match app.tournament_view(TournamentId(id)).await? {
        Some(view) => Ok(Json(view).into_response()),
        None => Err(ApiError::not_found("tournament")),
    }
}

async fn register_team(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<NameBody>,
) -> Result<Response, ApiError> {
    let team_id = TeamId::new();
    app.tournament(
        TournamentId(id),
        TournamentCommand::RegisterTeam {
            team_id,
            name: body.name,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(IdResponse { id: team_id.0 })).into_response())
}

async fn remove_team(
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

async fn generate_pools(
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

async fn generate_pool_matches(
    State(app): State<Arc<App>>,
    Path((id, pool_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let created = app
        .generate_pool_matches(TournamentId(id), PoolId(pool_id))
        .await?;
    Ok((StatusCode::CREATED, Json(CreatedResponse { created })).into_response())
}

async fn assign_pool_court(
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

async fn configure_courts(
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

async fn start_pools(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.tournament(TournamentId(id), TournamentCommand::StartPoolPhase)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn start_bracket(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    app.tournament(TournamentId(id), TournamentCommand::StartBracketPhase)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn schedule_match(
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

async fn start_match(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<StartMatch>,
) -> Result<Response, ApiError> {
    app.match_cmd(
        MatchId(id),
        MatchCommand::Start {
            court_id: CourtId(body.court_id),
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn record_set(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<RecordSetBody>,
) -> Result<Response, ApiError> {
    app.match_cmd(MatchId(id), MatchCommand::RecordSet { a: body.a, b: body.b })
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn dispatch(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let started = app.dispatch_courts(TournamentId(id)).await?;
    Ok(Json(DispatchResponse { started }).into_response())
}

async fn board(State(app): State<Arc<App>>, Path(id): Path<Uuid>) -> Result<Response, ApiError> {
    Ok(Json(app.board(TournamentId(id)).await?).into_response())
}

async fn standings(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    Ok(Json(app.standings(TournamentId(id)).await?).into_response())
}

async fn get_bracket(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    Ok(Json(app.bracket_view(TournamentId(id)).await?).into_response())
}

async fn generate_bracket(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
    Json(body): Json<GenerateBracket>,
) -> Result<Response, ApiError> {
    let created = app
        .generate_bracket(TournamentId(id), body.per_pool)
        .await?;
    Ok((StatusCode::CREATED, Json(CreatedResponse { created })).into_response())
}

async fn advance_bracket(
    State(app): State<Arc<App>>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let created = app.advance_bracket(TournamentId(id)).await?;
    Ok(Json(CreatedResponse { created }).into_response())
}

// ---- Error mapping ----

/// An HTTP error with a status and a user-facing message.
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn not_found(what: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: format!("{what} not found"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

impl<E: std::error::Error> From<AggregateError<E>> for ApiError {
    fn from(e: AggregateError<E>) -> Self {
        let status = match &e {
            AggregateError::UserError(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AggregateError::AggregateConflict => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: e.to_string(),
        }
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        let status = match e {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Command(_) => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: e.to_string(),
        }
    }
}
