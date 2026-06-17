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
use sqlx::{Pool, Postgres, Row};

use domain::bracket::{
    build_bracket, reseed_pool_separation, Bracket, BracketCommand, BracketError, BracketKind,
    BracketNode,
};
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
        // Snapshot store: rehydrate from a periodic snapshot + the events since,
        // instead of replaying the whole stream on every command.
        let tournaments = postgres_snapshot_cqrs(pool.clone(), vec![], SNAPSHOT_EVERY, ());
        let matches = postgres_snapshot_cqrs(pool.clone(), vec![], SNAPSHOT_EVERY, ());
        let brackets = postgres_snapshot_cqrs(pool.clone(), vec![], SNAPSHOT_EVERY, ());
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

    /// Start a match on a court, refusing if another match is already in
    /// progress on that court (avoids two live matches on one terrain).
    ///
    /// # Errors
    /// Returns [`AppError`] if the court is busy, or on a command/store failure.
    pub async fn start_match(
        &self,
        match_id: MatchId,
        court_id: CourtId,
    ) -> Result<(), AppError> {
        let busy = self.match_projection().await?.views().iter().any(|v| {
            v.court == Some(court_id) && v.status == SchedStatus::Playing && v.id != match_id
        });
        if busy {
            return Err(AppError::Command("terrain déjà occupé".into()));
        }
        self.match_cmd(match_id, MatchCommand::Start { court_id })
            .await
            .map_err(|e| AppError::Command(e.to_string()))
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

        let map = self.pool_court_map(tournament_id).await?;
        let plans = plan(&views, &courts, &map);

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
        let match_subquery = "SELECT aggregate_id FROM events \
             WHERE aggregate_type = 'Match' AND event_type = 'MatchScheduled' \
             AND payload->'Scheduled'->>'tournament_id' = $1";
        sqlx::query(&format!(
            "DELETE FROM snapshots WHERE aggregate_type = 'Match' AND aggregate_id IN ({match_subquery})"
        ))
        .bind(&tid)
        .execute(&self.pool)
        .await?;
        sqlx::query(&format!(
            "DELETE FROM events WHERE aggregate_type = 'Match' AND aggregate_id IN ({match_subquery})"
        ))
        .bind(&tid)
        .execute(&self.pool)
        .await?;
        // Bracket aggregate shares the tournament id.
        sqlx::query("DELETE FROM events WHERE aggregate_type = 'Bracket' AND aggregate_id = $1")
            .bind(&tid)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM snapshots WHERE aggregate_type = 'Bracket' AND aggregate_id = $1")
            .bind(&tid)
            .execute(&self.pool)
            .await?;
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
        // Match aggregate ids belonging to this tournament (via the Scheduled payload).
        let match_subquery = "SELECT aggregate_id FROM events \
             WHERE aggregate_type = 'Match' AND event_type = 'MatchScheduled' \
             AND payload->'Scheduled'->>'tournament_id' = $1";
        // Snapshots first (their selection depends on the events still existing).
        sqlx::query(&format!(
            "DELETE FROM snapshots WHERE aggregate_type = 'Match' AND aggregate_id IN ({match_subquery})"
        ))
        .bind(&tid)
        .execute(&self.pool)
        .await?;
        sqlx::query(&format!(
            "DELETE FROM events WHERE aggregate_type = 'Match' AND aggregate_id IN ({match_subquery})"
        ))
        .bind(&tid)
        .execute(&self.pool)
        .await?;
        // Tournament + Bracket aggregates share the tournament id.
        sqlx::query("DELETE FROM events WHERE aggregate_id = $1")
            .bind(&tid)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM snapshots WHERE aggregate_id = $1")
            .bind(&tid)
            .execute(&self.pool)
            .await?;
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
        let mut pool_of: std::collections::HashMap<TeamId, usize> = std::collections::HashMap::new();
        for (pool_idx, ps) in standings.iter().enumerate() {
            for row in &ps.rows {
                pool_of.insert(row.team, pool_idx + 1);
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
        let mut main_seeds: Vec<TeamId> = main.into_iter().map(|(_, _, t)| t).collect();
        let mut consolation_seeds: Vec<TeamId> = cons.into_iter().map(|(_, _, t)| t).collect();
        // Avoid first-round same-pool matchups where possible.
        reseed_pool_separation(&mut main_seeds, &pool_of);
        reseed_pool_separation(&mut consolation_seeds, &pool_of);

        if main_seeds.len() < 2 {
            return Err(AppError::Command(
                "at least two qualified teams are required".into(),
            ));
        }

        // A draw may already exist. Compare the freshly-computed seeds with the
        // stored ones:
        //   - identical → the bracket is still valid (e.g. re-clicking Générer);
        //     leave its matches and results untouched, just advance.
        //   - different → the stored draw came from stale standings (the classic
        //     "drawn before the pools were scored" case). Every match it produced
        //     is invalid, so drop them all — including any already scored, which
        //     are themselves garbage — then redraw from the correct seeds.
        if let Some((old_main, old_cons)) = self.bracket_seeds(tournament_id).await? {
            if old_main == main_seeds && old_cons == consolation_seeds {
                return self.advance_bracket(tournament_id).await;
            }
            let finals: Vec<MatchView> = self
                .match_projection()
                .await?
                .views()
                .into_iter()
                .filter(|v| v.tournament == tournament_id && v.pool.is_none())
                .collect();
            for v in &finals {
                self.delete_match(v.id).await?;
            }
            self.delete_bracket(tournament_id).await?;
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
            // Already drawn (race) → fall through and just advance.
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

        let tree = build_bracket(&main_seeds, &consolation_seeds, &results);
        // Largest round number per bracket (final), to convert a round index into
        // the team count of that round (2 = final, 4 = semis, …).
        let max_round_of = |kind: BracketKind| {
            tree.iter()
                .filter(|n| n.kind == kind && n.round != 0 && n.round != 255)
                .map(|n| n.round)
                .max()
                .unwrap_or(1)
        };
        let (max_main, max_cons) =
            (max_round_of(BracketKind::Main), max_round_of(BracketKind::Consolation));
        let format_of = |node: &BracketNode| -> MatchFormat {
            let max_r = if node.kind == BracketKind::Main { max_main } else { max_cons };
            let size: u16 = match node.round {
                0 => 1u16 << max_r,                 // barrage feeds the first round
                255 => 2,                           // 3rd-place plays like a final
                r => 1u16 << (max_r - r + 1),
            };
            view.bracket_round_formats
                .get(&size)
                .copied()
                .unwrap_or(view.bracket_format)
        };

        let mut created = Vec::new();
        for node in &tree {
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
                    format: format_of(node),
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

    /// Record a set, then — for a bracket match — advance the draw so any match
    /// whose two teams are now known is scheduled immediately, without waiting
    /// for a manual "Avancer".
    ///
    /// # Errors
    /// Returns [`AppError`] on a command or database failure.
    pub async fn record_set(&self, match_id: MatchId, a: u8, b: u8) -> Result<(), AppError> {
        self.match_cmd(match_id, MatchCommand::RecordSet { a, b })
            .await
            .map_err(|e| AppError::Command(e.to_string()))?;

        if let Some(view) = self.match_projection().await?.get(match_id) {
            if view.pool.is_none() {
                self.advance_bracket(view.tournament).await?;
            }
        }
        Ok(())
    }

    /// End a match by forfeit / retirement (`winner` takes it), then advance the
    /// bracket if it was a bracket match.
    ///
    /// # Errors
    /// Returns [`AppError`] on a command or store failure.
    pub async fn concede_match(&self, match_id: MatchId, winner: TeamId) -> Result<(), AppError> {
        self.match_cmd(match_id, MatchCommand::Concede { winner })
            .await
            .map_err(|e| AppError::Command(e.to_string()))?;
        if let Some(view) = self.match_projection().await?.get(match_id) {
            if view.pool.is_none() {
                self.advance_bracket(view.tournament).await?;
            }
        }
        Ok(())
    }

    /// Reset the whole bracket: drop every finals match (main + consolation) and
    /// the draw itself, returning to the "not drawn" state so "Générer" can
    /// re-seed from scratch. Pools and their results are untouched.
    ///
    /// # Errors
    /// Returns [`AppError`] on a store failure.
    pub async fn reset_bracket(&self, tournament_id: TournamentId) -> Result<(), AppError> {
        let finals: Vec<MatchView> = self
            .match_projection()
            .await?
            .views()
            .into_iter()
            .filter(|v| v.tournament == tournament_id && v.pool.is_none())
            .collect();
        for v in &finals {
            self.delete_match(v.id).await?;
        }
        self.delete_bracket(tournament_id).await?;
        Ok(())
    }

    /// Reset a single bracket match so it can be replayed: drop its event stream,
    /// then reconcile the bracket. The match is re-created fresh (its two teams
    /// are still known from the upstream results) and any later-round match that
    /// depended on its now-removed result is dropped. Pool matches are rejected.
    ///
    /// # Errors
    /// Returns [`AppError`] if the match is unknown, is a pool match, or on a
    /// store/command failure.
    pub async fn reset_bracket_match(&self, match_id: MatchId) -> Result<(), AppError> {
        let Some(view) = self.match_projection().await?.get(match_id).cloned() else {
            return Err(AppError::NotFound("match"));
        };
        if view.pool.is_some() {
            return Err(AppError::Command(
                "seul un match de bracket peut être réinitialisé".into(),
            ));
        }
        self.delete_match(match_id).await?;
        self.reconcile_bracket(view.tournament).await?;
        Ok(())
    }

    /// Correct a match's score, then — if it is a bracket match whose winner
    /// changed — reconcile the downstream bracket (delete now-invalid matches of
    /// later rounds and re-create the correct ones).
    ///
    /// # Errors
    /// Returns [`AppError`] on a command or database failure.
    pub async fn rescore_match(
        &self,
        match_id: MatchId,
        a: u8,
        b: u8,
    ) -> Result<(), AppError> {
        self.match_cmd(match_id, MatchCommand::Rescore { a, b })
            .await
            .map_err(|e| AppError::Command(e.to_string()))?;

        // Bracket match? Reconcile downstream rounds.
        if let Some(view) = self.match_projection().await?.get(match_id) {
            if view.pool.is_none() {
                self.reconcile_bracket(view.tournament).await?;
            }
        }
        Ok(())
    }

    /// Drop bracket matches whose pairing is no longer part of the recomputed
    /// tree (e.g. after a re-score flipped an earlier-round winner), then
    /// re-advance to schedule the correct matches.
    async fn reconcile_bracket(&self, tournament_id: TournamentId) -> Result<(), AppError> {
        let Some((main_seeds, consolation_seeds)) = self.bracket_seeds(tournament_id).await?
        else {
            return Ok(());
        };
        let unordered = |x: TeamId, y: TeamId| if x <= y { (x, y) } else { (y, x) };

        let bracket_matches: Vec<MatchView> = self
            .match_projection()
            .await?
            .views()
            .into_iter()
            .filter(|v| v.tournament == tournament_id && v.pool.is_none())
            .collect();
        let results: Vec<(TeamId, TeamId, TeamId)> = bracket_matches
            .iter()
            .filter_map(|v| v.winner.map(|w| (v.team_a, v.team_b, w)))
            .collect();

        // Legitimate matchups in the current tree (both teams known).
        let valid: std::collections::HashSet<(TeamId, TeamId)> =
            build_bracket(&main_seeds, &consolation_seeds, &results)
                .into_iter()
                .filter_map(|n| match (n.team_a, n.team_b) {
                    (Some(x), Some(y)) => Some(unordered(x, y)),
                    _ => None,
                })
                .collect();

        for v in &bracket_matches {
            if !valid.contains(&unordered(v.team_a, v.team_b)) {
                self.delete_match(v.id).await?;
            }
        }
        self.advance_bracket(tournament_id).await?;
        Ok(())
    }

    /// Hard-delete a single match's event stream.
    async fn delete_match(&self, match_id: MatchId) -> Result<(), AppError> {
        let id = match_id.to_string();
        sqlx::query("DELETE FROM events WHERE aggregate_type = 'Match' AND aggregate_id = $1")
            .bind(&id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM snapshots WHERE aggregate_type = 'Match' AND aggregate_id = $1")
            .bind(&id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Hard-delete a tournament's bracket draw so it can be re-seeded. The
    /// Bracket aggregate is keyed by the tournament id.
    async fn delete_bracket(&self, tournament_id: TournamentId) -> Result<(), AppError> {
        let id = tournament_id.to_string();
        sqlx::query("DELETE FROM events WHERE aggregate_type = 'Bracket' AND aggregate_id = $1")
            .bind(&id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM snapshots WHERE aggregate_type = 'Bracket' AND aggregate_id = $1")
            .bind(&id)
            .execute(&self.pool)
            .await?;
        Ok(())
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
                feeds: n.feeds,
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
