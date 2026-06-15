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
use postgres_es::{default_postgress_pool, postgres_cqrs, PostgresCqrs};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres, Row};

use domain::generation::round_robin_pairs;
use domain::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
use domain::matches::{Match, MatchCommand, MatchError, MatchEvent};
use domain::projections::MatchProjection;
use domain::scheduling::{plan, CourtPlan, MatchView};
use domain::score::MatchFormat;
use domain::tournament::{
    Phase, Pool as DomainPool, Tournament, TournamentCommand, TournamentError, TournamentEvent,
};

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
    /// Pool match format.
    pub pool_format: MatchFormat,
    /// Bracket match format.
    pub bracket_format: MatchFormat,
}

/// The live board for a tournament: court plans plus the underlying matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardView {
    /// Per-court plan (current / next / previews).
    pub courts: Vec<CourtPlan>,
    /// All match views for the tournament.
    pub matches: Vec<MatchView>,
}

/// Idempotent event-store schema, applied by [`App::run_migrations`].
const MIGRATION_SQL: &str = include_str!("../../../db/init.sql");

/// Top-level application: one event store, one CQRS framework per aggregate.
pub struct App {
    pool: Pool<Postgres>,
    tournaments: PostgresCqrs<Tournament>,
    matches: PostgresCqrs<Match>,
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
        let tournaments = postgres_cqrs(pool.clone(), vec![], ());
        let matches = postgres_cqrs(pool.clone(), vec![], ());
        Self {
            pool,
            tournaments,
            matches,
        }
    }

    /// Create the event-store tables if they do not already exist.
    ///
    /// # Errors
    /// Returns any `sqlx` error from executing the schema statements.
    pub async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        // The schema file holds multiple statements; `execute` on a raw string
        // runs them as a single batch.
        sqlx::raw_sql(MIGRATION_SQL).execute(&self.pool).await?;
        Ok(())
    }

    /// Access the underlying pool (e.g. to build read-model repositories).
    #[must_use]
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    /// Execute a command against a tournament aggregate.
    ///
    /// # Errors
    /// Returns [`AggregateError`] if the command is rejected or persistence fails.
    pub async fn tournament(
        &self,
        id: TournamentId,
        command: TournamentCommand,
    ) -> Result<(), AggregateError<TournamentError>> {
        self.tournaments.execute(&id.to_string(), command).await
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

    /// Scheduling process manager (pull-based).
    ///
    /// Replays the event store to build the current match view, runs the
    /// [`plan`](domain::scheduling::plan) over this tournament's courts, and
    /// starts (`Start{court}`) the suggested next match on every free court.
    ///
    /// Back-to-back-forced suggestions (`needs_rest`) are *not* auto-started —
    /// they are left for a human to confirm (hybrid dispatch).
    ///
    /// Returns the matches that were started. Idempotent in effect: re-running
    /// with no freed court starts nothing.
    ///
    /// # Errors
    /// Returns [`AppError`] on a database, deserialization or command failure.
    pub async fn dispatch_courts(
        &self,
        tournament_id: TournamentId,
    ) -> Result<Vec<MatchId>, AppError> {
        let courts = self.tournament_courts(tournament_id).await?;
        if courts.is_empty() {
            return Ok(Vec::new());
        }
        let projection = self.match_projection().await?;
        let views: Vec<_> = projection
            .views()
            .into_iter()
            .filter(|v| v.tournament == tournament_id)
            .collect();

        let plans = plan(&views, &courts, &std::collections::HashMap::new());

        let mut started = Vec::new();
        for cp in plans {
            if cp.current.is_some() {
                continue; // court busy
            }
            let Some(next) = cp.next else { continue };
            if next.needs_rest {
                continue; // leave forced back-to-back for manual confirmation
            }
            self.match_cmd(next.match_id, MatchCommand::Start { court_id: cp.court })
                .await
                .map_err(|e| AppError::Command(e.to_string()))?;
            started.push(next.match_id);
        }
        Ok(started)
    }

    /// Schedule the single round-robin matches for a pool (every team plays
    /// every other once). Idempotent: pairs already scheduled for the pool are
    /// skipped, so re-running only fills gaps. Returns the matches created.
    ///
    /// # Errors
    /// Returns [`AppError`] if the tournament or pool is unknown, or on a
    /// database / command failure.
    pub async fn generate_pool_matches(
        &self,
        tournament_id: TournamentId,
        pool_id: PoolId,
    ) -> Result<Vec<MatchId>, AppError> {
        let view = self
            .tournament_view(tournament_id)
            .await?
            .ok_or(AppError::NotFound("tournament"))?;
        let pool = view
            .pools
            .iter()
            .find(|p| p.id == pool_id)
            .ok_or(AppError::NotFound("pool"))?;
        let format = view.pool_format;

        let unordered = |a: TeamId, b: TeamId| if a <= b { (a, b) } else { (b, a) };
        let existing: std::collections::HashSet<(TeamId, TeamId)> = self
            .match_projection()
            .await?
            .views()
            .into_iter()
            .filter(|v| v.tournament == tournament_id && v.pool == Some(pool_id))
            .map(|v| unordered(v.team_a, v.team_b))
            .collect();

        let mut created = Vec::new();
        for (a, b) in round_robin_pairs(&pool.teams) {
            if existing.contains(&unordered(a, b)) {
                continue;
            }
            let match_id = MatchId::new();
            self.match_cmd(
                match_id,
                MatchCommand::Schedule {
                    match_id,
                    tournament_id,
                    format,
                    team_a: a,
                    team_b: b,
                    pool_id: Some(pool_id),
                },
            )
            .await
            .map_err(|e| AppError::Command(e.to_string()))?;
            created.push(match_id);
        }
        Ok(created)
    }

    /// List every tournament with its current phase.
    ///
    /// # Errors
    /// Returns [`AppError`] on a database or deserialization failure.
    pub async fn list_tournaments(&self) -> Result<Vec<TournamentSummary>, AppError> {
        let rows = sqlx::query(
            "SELECT aggregate_id FROM events \
             WHERE aggregate_type = 'Tournament' AND event_type = 'TournamentCreated' \
             ORDER BY global_seq",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::new();
        for row in rows {
            let id_str: String = row.try_get("aggregate_id")?;
            let id = TournamentId(id_str.parse().map_err(AppError::BadId)?);
            if let Some(view) = self.tournament_view(id).await? {
                out.push(TournamentSummary {
                    id,
                    name: view.name,
                    phase: view.phase,
                });
            }
        }
        Ok(out)
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
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf3,
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
                TournamentEvent::TeamRegistered { team_id, name } => {
                    view.teams.push(TeamView { id: team_id, name });
                }
                TournamentEvent::TeamRemoved { team_id } => {
                    view.teams.retain(|t| t.id != team_id);
                }
                TournamentEvent::PoolsGenerated { pools } => view.pools = pools,
                TournamentEvent::CourtsConfigured { courts } => view.courts = courts,
                TournamentEvent::PoolPhaseStarted => view.phase = Phase::PoolPhase,
                TournamentEvent::BracketPhaseStarted => view.phase = Phase::BracketPhase,
            }
        }
        Ok(Some(view))
    }

    /// Build the live board (court plans + match views) for a tournament.
    ///
    /// # Errors
    /// Returns [`AppError`] on a database or deserialization failure.
    pub async fn board(&self, tournament_id: TournamentId) -> Result<BoardView, AppError> {
        let courts = self.tournament_courts(tournament_id).await?;
        let matches: Vec<MatchView> = self
            .match_projection()
            .await?
            .views()
            .into_iter()
            .filter(|v| v.tournament == tournament_id)
            .collect();
        let plans = plan(&matches, &courts, &std::collections::HashMap::new());
        Ok(BoardView {
            courts: plans,
            matches,
        })
    }

    /// Replay a tournament's events in sequence order.
    async fn tournament_events(
        &self,
        id: TournamentId,
    ) -> Result<Vec<TournamentEvent>, AppError> {
        let rows = sqlx::query(
            "SELECT payload FROM events \
             WHERE aggregate_type = 'Tournament' AND aggregate_id = $1 \
             ORDER BY sequence",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let payload: serde_json::Value = row.try_get("payload")?;
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
        let rows = sqlx::query(
            "SELECT aggregate_id, payload FROM events \
             WHERE aggregate_type = 'Match' ORDER BY global_seq",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut projection = MatchProjection::new();
        for row in rows {
            let id_str: String = row.try_get("aggregate_id")?;
            let id = MatchId(id_str.parse().map_err(AppError::BadId)?);
            let payload: serde_json::Value = row.try_get("payload")?;
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
