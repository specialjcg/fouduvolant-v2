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
use domain::generation::round_robin_pairs;
use domain::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
use domain::matches::{Match, MatchCommand, MatchError, MatchEvent};
use domain::projections::MatchProjection;
use domain::scheduling::{forecast, plan, CourtPlan, MatchView, SchedStatus};
use domain::score::MatchFormat;
use domain::standings::{pool_standings, MatchResult};
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
    /// First player.
    pub player1: String,
    /// Second player.
    pub player2: String,
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
const MATCH_MINUTES: u32 = 15;

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
        let mut out = Vec::new();
        for id_str in self.db.created_tournament_ids().await? {
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

    /// Reset a tournament for a fresh launch: deletes all its matches and the
    /// bracket draw, and reopens the draft (teams / pools / courts kept).
    ///
    /// # Errors
    /// Returns [`AppError`] on a database or command failure.
    /// Redo the pools live after a no-show: only allowed while no pool match has
    /// been played. Wipes the (unplayed) matches and reopens the draft, keeping
    /// teams and pools so the absent team can be removed and the pools redrawn.
    ///
    /// # Errors
    /// Returns [`AppError`] if a pool match already has a result, or on failure.
    pub async fn redo_pools(&self, tournament_id: TournamentId) -> Result<(), AppError> {
        let played = self
            .match_projection()
            .await?
            .views()
            .into_iter()
            .any(|v| v.tournament == tournament_id && v.pool.is_some() && v.winner.is_some());
        if played {
            return Err(AppError::Command(
                "des matchs de poule ont déjà été joués — refaire les poules est impossible".into(),
            ));
        }
        self.reset_tournament(tournament_id).await
    }

    pub async fn reset_tournament(&self, tournament_id: TournamentId) -> Result<(), AppError> {
        let tid = tournament_id.to_string();
        self.db.delete_tournament_matches(&tid).await?;
        self.db.delete_aggregate("Bracket", &tid).await?;
        self.tournament(tournament_id, TournamentCommand::ReopenDraft)
            .await
            .map_err(|e| AppError::Command(e.to_string()))?;
        Ok(())
    }

    /// Hard-delete a tournament: removes its event streams (Tournament +
    /// Bracket, both keyed by the tournament id) and all its matches' streams.
    ///
    /// # Errors
    /// Returns [`AppError`] on a database failure.
    pub async fn delete_tournament(&self, tournament_id: TournamentId) -> Result<(), AppError> {
        let tid = tournament_id.to_string();
        self.db.delete_tournament_matches(&tid).await?;
        // Tournament + Bracket aggregates share the tournament id.
        self.db.delete_by_id(&tid).await?;
        Ok(())
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
                    });
                }
                TournamentEvent::TeamRemoved { team_id } => {
                    view.teams.retain(|t| t.id != team_id);
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
        let map = self.pool_court_map(tournament_id).await?;
        let plans = plan(&matches, &courts, &map);
        Ok(BoardView {
            courts: plans,
            matches,
        })
    }

    /// Full per-court forecast (prévisionnel) with names and estimated times.
    ///
    /// # Errors
    /// Returns [`AppError`] on a store or deserialization failure.
    pub async fn schedule(
        &self,
        tournament_id: TournamentId,
    ) -> Result<Vec<ForecastCourt>, AppError> {
        let courts = self.tournament_courts(tournament_id).await?;
        let map = self.pool_court_map(tournament_id).await?;
        let views: Vec<MatchView> = self
            .match_projection()
            .await?
            .views()
            .into_iter()
            .filter(|v| v.tournament == tournament_id)
            .collect();
        let by_id: std::collections::HashMap<MatchId, MatchView> =
            views.iter().map(|v| (v.id, v.clone())).collect();

        let (team_names, pool_names) = match self.tournament_view(tournament_id).await? {
            Some(view) => (
                view.teams
                    .iter()
                    .map(|t| (t.id, t.name.clone()))
                    .collect::<std::collections::HashMap<_, _>>(),
                view.pools
                    .iter()
                    .map(|p| (p.id, p.name.clone()))
                    .collect::<std::collections::HashMap<_, _>>(),
            ),
            None => Default::default(),
        };
        let name = |id: TeamId| team_names.get(&id).cloned().unwrap_or_default();

        Ok(forecast(&views, &courts, &map)
            .into_iter()
            .map(|(court, ids)| ForecastCourt {
                court,
                matches: ids
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, id)| {
                        by_id.get(&id).map(|v| ForecastMatch {
                            id,
                            team_a: name(v.team_a),
                            team_b: name(v.team_b),
                            pool: v.pool.and_then(|p| pool_names.get(&p).cloned()),
                            status: v.status,
                            points_a: v.points_a,
                            points_b: v.points_b,
                            eta_min: i as u32 * MATCH_MINUTES,
                        })
                    })
                    .collect(),
            })
            .collect())
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

    /// Compute ranked standings for every pool of a tournament.
    ///
    /// # Errors
    /// Returns [`AppError`] on a database or deserialization failure.
    pub async fn standings(
        &self,
        tournament_id: TournamentId,
    ) -> Result<Vec<PoolStandingsView>, AppError> {
        let Some(view) = self.tournament_view(tournament_id).await? else {
            return Ok(Vec::new());
        };
        let names: std::collections::HashMap<TeamId, String> = view
            .teams
            .iter()
            .map(|t| (t.id, t.name.clone()))
            .collect();

        let matches = self.match_projection().await?;
        let done: Vec<MatchView> = matches
            .views()
            .into_iter()
            .filter(|v| v.tournament == tournament_id && v.winner.is_some())
            .collect();

        let mut out = Vec::with_capacity(view.pools.len());
        for pool in &view.pools {
            let results: Vec<MatchResult> = done
                .iter()
                .filter(|v| v.pool == Some(pool.id))
                .map(|v| MatchResult {
                    team_a: v.team_a,
                    team_b: v.team_b,
                    winner: v.winner.expect("filtered to winners"),
                    points_a: u32::from(v.points_a),
                    points_b: u32::from(v.points_b),
                })
                .collect();

            let rows = pool_standings(&pool.teams, &results)
                .into_iter()
                .enumerate()
                .map(|(i, s)| StandingRow {
                    team: s.team,
                    name: names.get(&s.team).cloned().unwrap_or_default(),
                    rank: i as u32 + 1,
                    played: s.played,
                    wins: s.wins,
                    points_for: s.points_for,
                    points_against: s.points_against,
                    diff: s.diff(),
                })
                .collect();

            out.push(PoolStandingsView {
                pool_id: pool.id,
                name: pool.name.clone(),
                rows,
            });
        }
        Ok(out)
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
