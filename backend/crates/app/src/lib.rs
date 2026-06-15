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

use domain::bracket::{build_bracket, Bracket, BracketCommand, BracketError, BracketKind};
use domain::generation::round_robin_pairs;
use domain::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
use domain::matches::{Match, MatchCommand, MatchError, MatchEvent};
use domain::projections::MatchProjection;
use domain::scheduling::{plan, CourtPlan, MatchView};
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
}

/// Idempotent event-store schema, applied by [`App::run_migrations`].
const MIGRATION_SQL: &str = include_str!("../../../db/init.sql");

/// Top-level application: one event store, one CQRS framework per aggregate.
pub struct App {
    pool: Pool<Postgres>,
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
        let tournaments = postgres_cqrs(pool.clone(), vec![], ());
        let matches = postgres_cqrs(pool.clone(), vec![], ());
        let brackets = postgres_cqrs(pool.clone(), vec![], ());
        Self {
            pool,
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

    /// Draw the bracket from current pool standings: the top `per_pool` of each
    /// pool seed the main draw (rank-major, pools interleaved); the rest seed the
    /// consolation draw. Then schedules the first playable matches.
    ///
    /// Idempotent: re-running after the draw exists just advances.
    ///
    /// # Errors
    /// Returns [`AppError`] on too few qualified teams or a store/command failure.
    pub async fn generate_bracket(
        &self,
        tournament_id: TournamentId,
        per_pool: usize,
    ) -> Result<Vec<MatchId>, AppError> {
        let standings = self.standings(tournament_id).await?;
        let mut main: Vec<(u32, usize, TeamId)> = Vec::new();
        let mut cons: Vec<(u32, usize, TeamId)> = Vec::new();
        for (pool_idx, ps) in standings.iter().enumerate() {
            for row in &ps.rows {
                let entry = (row.rank, pool_idx, row.team);
                if (row.rank as usize) <= per_pool {
                    main.push(entry);
                } else {
                    cons.push(entry);
                }
            }
        }
        // Rank-major ordering keeps pools apart in the seeding.
        main.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        cons.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let main_seeds: Vec<TeamId> = main.into_iter().map(|(_, _, t)| t).collect();
        let consolation_seeds: Vec<TeamId> = cons.into_iter().map(|(_, _, t)| t).collect();

        if main_seeds.len() < 2 {
            return Err(AppError::Command(
                "at least two qualified teams are required".into(),
            ));
        }

        match self
            .bracket(
                tournament_id,
                BracketCommand::Draw {
                    main_seeds,
                    consolation_seeds,
                },
            )
            .await
        {
            Ok(()) => {}
            // Already drawn → fall through and just advance.
            Err(AggregateError::UserError(BracketError::AlreadyDrawn)) => {}
            Err(e) => return Err(AppError::Command(e.to_string())),
        }

        self.advance_bracket(tournament_id).await
    }

    /// Schedule every bracket match whose teams are now known and not yet
    /// scheduled. Pull-based, idempotent — safe to call after each result.
    ///
    /// # Errors
    /// Returns [`AppError`] on a store or command failure.
    pub async fn advance_bracket(
        &self,
        tournament_id: TournamentId,
    ) -> Result<Vec<MatchId>, AppError> {
        let Some((main_seeds, consolation_seeds)) = self.bracket_seeds(tournament_id).await?
        else {
            return Ok(Vec::new());
        };
        let Some(view) = self.tournament_view(tournament_id).await? else {
            return Ok(Vec::new());
        };
        let format = view.bracket_format;

        let bracket_matches: Vec<MatchView> = self
            .match_projection()
            .await?
            .views()
            .into_iter()
            .filter(|v| v.tournament == tournament_id && v.pool.is_none())
            .collect();

        let unordered = |a: TeamId, b: TeamId| if a <= b { (a, b) } else { (b, a) };
        let results: Vec<(TeamId, TeamId, TeamId)> = bracket_matches
            .iter()
            .filter_map(|v| v.winner.map(|w| (v.team_a, v.team_b, w)))
            .collect();
        let mut existing: std::collections::HashSet<(TeamId, TeamId)> = bracket_matches
            .iter()
            .map(|v| unordered(v.team_a, v.team_b))
            .collect();

        let mut created = Vec::new();
        for node in build_bracket(&main_seeds, &consolation_seeds, &results) {
            if !node.is_playable() {
                continue;
            }
            let (a, b) = (
                node.team_a.expect("playable has team_a"),
                node.team_b.expect("playable has team_b"),
            );
            if !existing.insert(unordered(a, b)) {
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
                    pool_id: None,
                },
            )
            .await
            .map_err(|e| AppError::Command(e.to_string()))?;
            created.push(match_id);
        }
        Ok(created)
    }

    /// The bracket tree (main + consolation) with team names resolved.
    ///
    /// # Errors
    /// Returns [`AppError`] on a store or deserialization failure.
    pub async fn bracket_view(
        &self,
        tournament_id: TournamentId,
    ) -> Result<Vec<BracketNodeView>, AppError> {
        let Some((main_seeds, consolation_seeds)) = self.bracket_seeds(tournament_id).await?
        else {
            return Ok(Vec::new());
        };
        let names: std::collections::HashMap<TeamId, String> = self
            .tournament_view(tournament_id)
            .await?
            .map(|v| v.teams.into_iter().map(|t| (t.id, t.name)).collect())
            .unwrap_or_default();
        let name = |t: Option<TeamId>| t.and_then(|id| names.get(&id).cloned());

        let results: Vec<(TeamId, TeamId, TeamId)> = self
            .match_projection()
            .await?
            .views()
            .into_iter()
            .filter(|v| v.tournament == tournament_id && v.pool.is_none())
            .filter_map(|v| v.winner.map(|w| (v.team_a, v.team_b, w)))
            .collect();

        Ok(build_bracket(&main_seeds, &consolation_seeds, &results)
            .into_iter()
            .map(|n| BracketNodeView {
                kind: n.kind,
                round: n.round,
                index: n.index,
                team_a: name(n.team_a),
                team_b: name(n.team_b),
                winner: name(n.winner),
            })
            .collect())
    }

    /// Replay the bracket aggregate's draw, if any.
    async fn bracket_seeds(
        &self,
        tournament_id: TournamentId,
    ) -> Result<Option<(Vec<TeamId>, Vec<TeamId>)>, AppError> {
        let rows = sqlx::query(
            "SELECT payload FROM events \
             WHERE aggregate_type = 'Bracket' AND aggregate_id = $1 \
             ORDER BY sequence",
        )
        .bind(tournament_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mut seeds = None;
        for row in rows {
            let payload: serde_json::Value = row.try_get("payload")?;
            let domain::bracket::BracketEvent::Drawn {
                main_seeds,
                consolation_seeds,
            } = serde_json::from_value(payload)?;
            seeds = Some((main_seeds, consolation_seeds));
        }
        Ok(seeds)
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
