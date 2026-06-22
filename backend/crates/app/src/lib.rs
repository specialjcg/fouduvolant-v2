//! Application layer: wires the pure [`domain`] aggregates to a PostgreSQL event
//! store via `postgres-es`.
//!
//! The CQRS framework is generic over a single aggregate, so each aggregate gets
//! its own `PostgresCqrs`; they share one connection pool. [`App`] is the façade
//! that owns both and exposes typed command entry points.
//!
//! Read-model persistence (projections as `cqrs_es::Query` backed by view tables)
//! is intentionally not wired here — see `docs/ARCHITECTURE.md`. The query vector
//! is empty for now.

pub use cqrs_es::AggregateError;
use postgres_es::{default_postgress_pool, postgres_snapshot_cqrs, PostgresCqrs};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};

pub mod infrastructure;
use infrastructure::{Db, EventStore};

mod services;

use domain::bracket::{Bracket, BracketCommand, BracketError, BracketKind};
use domain::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
use domain::matches::{Match, MatchCommand, MatchError, MatchEvent};
use domain::projections::MatchProjection;
use domain::scheduling::{CourtPlan, MatchView, SchedStatus};
use domain::score::MatchFormat;
use domain::tournament::{Phase, Pool as DomainPool, Tournament, TournamentEvent};

/// A tournament list entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentSummary {
    /// Tournament id.
    pub id: TournamentId,
    /// Display name.
    pub name: String,
    /// Current phase.
    pub phase: Phase,
}

/// A team within a tournament view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamView {
    /// Team id.
    pub id: TeamId,
    /// Team display name.
    pub name: String,
    /// First player.
    pub player1: String,
    /// Second player.
    pub player2: String,
    /// Team forfeited (withdrew / no-show) after the draft.
    pub forfeited: bool,
}

/// A pool pinned to a court (manual scheduling).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolCourtView {
    /// Pool id.
    pub pool: PoolId,
    /// Court id.
    pub court: CourtId,
}

/// Full read view of a tournament, folded from its event stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentView {
    /// Tournament id.
    pub id: TournamentId,
    /// Display name.
    pub name: String,
    /// Current phase.
    pub phase: Phase,
    /// Registered teams.
    pub teams: Vec<TeamView>,
    /// Pools (empty until generated).
    pub pools: Vec<DomainPool>,
    /// Configured courts.
    pub courts: Vec<CourtId>,
    /// Manual pool→court assignments.
    pub pool_courts: Vec<PoolCourtView>,
    /// Pool match format.
    pub pool_format: MatchFormat,
    /// Bracket match format (default for rounds without an override).
    pub bracket_format: MatchFormat,
    /// Per-round bracket format override, keyed by team count (2 = final, …).
    pub bracket_round_formats: std::collections::HashMap<u16, MatchFormat>,
}

/// One row of a pool's ranked standings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandingRow {
    /// Team id.
    pub team: TeamId,
    /// Team display name.
    pub name: String,
    /// Final rank within the pool (1-based).
    pub rank: u32,
    /// Matches played.
    pub played: u32,
    /// Matches won.
    pub wins: u32,
    /// Points scored.
    pub points_for: u32,
    /// Points conceded.
    pub points_against: u32,
    /// Point difference.
    pub diff: i32,
}

/// A pool's ranked standings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolStandingsView {
    /// Pool id.
    pub pool_id: PoolId,
    /// Pool display name.
    pub name: String,
    /// Ranked rows.
    pub rows: Vec<StandingRow>,
}

/// One match in a court's forecast, with names + estimated start offset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastMatch {
    /// Match id.
    pub id: MatchId,
    /// First side name.
    pub team_a: String,
    /// Second side name.
    pub team_b: String,
    /// Pool name (None for bracket).
    pub pool: Option<String>,
    /// Scheduling status.
    pub status: SchedStatus,
    /// Points side A.
    pub points_a: u16,
    /// Points side B.
    pub points_b: u16,
    /// Estimated start, minutes from the start of this court's schedule.
    pub eta_min: u32,
}

/// A court's full forecast (prévisionnel).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastCourt {
    /// Court id.
    pub court: CourtId,
    /// Ordered matches.
    pub matches: Vec<ForecastMatch>,
}

/// Average match duration (minutes) used for the forecast.
pub(crate) const MATCH_MINUTES: u32 = 15;

/// The live board for a tournament: court plans plus the underlying matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardView {
    /// Per-court plan (current / next / previews).
    pub courts: Vec<CourtPlan>,
    /// All match views for the tournament.
    pub matches: Vec<MatchView>,
}

/// One node of a bracket tree, with team names resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BracketNodeView {
    /// Main or consolation.
    pub kind: BracketKind,
    /// Round, 1-based.
    pub round: u8,
    /// Index within the round, 0-based.
    pub index: u16,
    /// First side name (`None` = bye / unknown).
    pub team_a: Option<String>,
    /// Second side name.
    pub team_b: Option<String>,
    /// Winner name once decided.
    pub winner: Option<String>,
    /// For a preliminary (round 0) node: the round-1 match index it feeds.
    pub feeds: Option<u16>,
}

/// Idempotent event-store schema, applied by [`App::run_migrations`].
const MIGRATION_SQL: &str = include_str!("../../../db/init.sql");

/// Take an aggregate snapshot every N events (rehydration reads the snapshot
/// plus the events since, rather than the whole stream).
const SNAPSHOT_EVERY: usize = 20;

/// Top-level application: one event store, one CQRS framework per aggregate.
pub struct App {
    db: Db,
    tournaments: PostgresCqrs<Tournament>,
    matches: PostgresCqrs<Match>,
    brackets: PostgresCqrs<Bracket>,
}

impl App {
    /// Connect to PostgreSQL and build the framework. Does not create tables —
    /// call [`App::run_migrations`] once at startup.
    ///
    /// # Panics
    /// Panics if the connection pool cannot be established (mirrors
    /// `postgres-es`'s own `default_postgress_pool`).
    pub async fn connect(database_url: &str) -> Self {
        let pool = default_postgress_pool(database_url).await;
        Self::from_pool(pool)
    }

    /// Build the framework over an existing pool (useful for tests).
    #[must_use]
    pub fn from_pool(pool: Pool<Postgres>) -> Self {
        // Snapshot store: rehydrate from a periodic snapshot + the events since,
        // instead of replaying the whole stream on every command.
        let tournaments = postgres_snapshot_cqrs(pool.clone(), vec![], SNAPSHOT_EVERY, ());
        let matches = postgres_snapshot_cqrs(pool.clone(), vec![], SNAPSHOT_EVERY, ());
        let brackets = postgres_snapshot_cqrs(pool.clone(), vec![], SNAPSHOT_EVERY, ());
        Self {
            db: Db::new(pool),
            tournaments,
            matches,
            brackets,
        }
    }

    /// Create the event-store tables if they do not already exist.
    ///
    /// # Errors
    /// Returns any `sqlx` error from executing the schema statements.
    pub async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        self.db.run_migrations(MIGRATION_SQL).await
    }

    /// Execute a command against a match aggregate.
    ///
    /// # Errors
    /// Returns [`AggregateError`] if the command is rejected or persistence fails.
    pub async fn match_cmd(
        &self,
        id: MatchId,
        command: MatchCommand,
    ) -> Result<(), AggregateError<MatchError>> {
        self.matches.execute(&id.to_string(), command).await
    }

    /// Fold a tournament's event stream into a [`TournamentView`].
    /// Returns `None` if the tournament does not exist.
    ///
    /// # Errors
    /// Returns [`AppError`] on a database or deserialization failure.
    pub async fn tournament_view(
        &self,
        id: TournamentId,
    ) -> Result<Option<TournamentView>, AppError> {
        let events = self.tournament_events(id).await?;
        if events.is_empty() {
            return Ok(None);
        }
        let mut view = TournamentView {
            id,
            name: String::new(),
            phase: Phase::NotCreated,
            teams: Vec::new(),
            pools: Vec::new(),
            courts: Vec::new(),
            pool_courts: Vec::new(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf3,
            bracket_round_formats: std::collections::HashMap::new(),
        };
        for ev in events {
            match ev {
                TournamentEvent::Created {
                    name,
                    pool_format,
                    bracket_format,
                    ..
                } => {
                    view.name = name;
                    view.phase = Phase::Draft;
                    view.pool_format = pool_format;
                    view.bracket_format = bracket_format;
                }
                TournamentEvent::TeamRegistered {
                    team_id,
                    name,
                    player1,
                    player2,
                } => {
                    view.teams.push(TeamView {
                        id: team_id,
                        name,
                        player1,
                        player2,
                        forfeited: false,
                    });
                }
                TournamentEvent::TeamRemoved { team_id } => {
                    view.teams.retain(|t| t.id != team_id);
                }
                TournamentEvent::TeamForfeited { team_id } => {
                    if let Some(t) = view.teams.iter_mut().find(|t| t.id == team_id) {
                        t.forfeited = true;
                    }
                }
                TournamentEvent::PoolsGenerated { pools } => view.pools = pools,
                TournamentEvent::CourtsConfigured { courts } => view.courts = courts,
                TournamentEvent::PoolCourtAssigned { pool_id, court_id } => {
                    view.pool_courts.retain(|pc| pc.pool != pool_id);
                    view.pool_courts.push(PoolCourtView {
                        pool: pool_id,
                        court: court_id,
                    });
                }
                TournamentEvent::PoolPhaseStarted => view.phase = Phase::PoolPhase,
                TournamentEvent::BracketPhaseStarted => view.phase = Phase::BracketPhase,
                TournamentEvent::DraftReopened => view.phase = Phase::Draft,
                TournamentEvent::BracketFormatSet { format } => view.bracket_format = format,
                TournamentEvent::BracketRoundFormatSet { round_size, format } => {
                    view.bracket_round_formats.insert(round_size, format);
                }
            }
        }
        Ok(Some(view))
    }

    /// Manual pool→court assignments, folded from the tournament events.
    async fn pool_court_map(
        &self,
        tournament_id: TournamentId,
    ) -> Result<std::collections::HashMap<PoolId, CourtId>, AppError> {
        let mut map = std::collections::HashMap::new();
        for ev in self.tournament_events(tournament_id).await? {
            if let TournamentEvent::PoolCourtAssigned { pool_id, court_id } = ev {
                map.insert(pool_id, court_id);
            }
        }
        Ok(map)
    }

    /// Execute a command against a tournament's bracket aggregate.
    ///
    /// # Errors
    /// Returns [`AggregateError`] if the command is rejected or persistence fails.
    pub async fn bracket(
        &self,
        id: TournamentId,
        command: BracketCommand,
    ) -> Result<(), AggregateError<BracketError>> {
        self.brackets.execute(&id.to_string(), command).await
    }

    /// Hard-delete a single match's event stream.
    async fn delete_match(&self, match_id: MatchId) -> Result<(), AppError> {
        self.db.delete_aggregate("Match", &match_id.to_string()).await
    }

    /// Replay a tournament's events in sequence order.
    async fn tournament_events(
        &self,
        id: TournamentId,
    ) -> Result<Vec<TournamentEvent>, AppError> {
        let mut events = Vec::new();
        for payload in self.db.events_for("Tournament", &id.to_string()).await? {
            events.push(serde_json::from_value(payload)?);
        }
        Ok(events)
    }

    /// Fold a tournament's events to its currently configured courts.
    async fn tournament_courts(
        &self,
        tournament_id: TournamentId,
    ) -> Result<Vec<CourtId>, AppError> {
        let mut courts = Vec::new();
        for ev in self.tournament_events(tournament_id).await? {
            if let TournamentEvent::CourtsConfigured { courts: c } = ev {
                courts = c;
            }
        }
        Ok(courts)
    }

    /// Replay all `Match` events in global commit order into a projection.
    async fn match_projection(&self) -> Result<MatchProjection, AppError> {
        let mut projection = MatchProjection::new();
        for (id_str, payload) in self.db.match_events_global().await? {
            let id = MatchId(id_str.parse().map_err(AppError::BadId)?);
            let event: MatchEvent = serde_json::from_value(payload)?;
            projection.apply(id, &event);
        }
        Ok(projection)
    }
}

/// Errors from store replay, reads and the scheduling dispatcher.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// A database query failed.
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    /// An event payload could not be deserialized.
    #[error("deserialize event: {0}")]
    Json(#[from] serde_json::Error),
    /// An aggregate id in the store was not a valid UUID.
    #[error("invalid aggregate id: {0}")]
    BadId(uuid::Error),
    /// A referenced entity does not exist.
    #[error("{0} not found")]
    NotFound(&'static str),
    /// Issuing a command failed.
    #[error("command failed: {0}")]
    Command(String),
}
