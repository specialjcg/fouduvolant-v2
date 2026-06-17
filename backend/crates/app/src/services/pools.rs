//! Pool feature: schedule round-robin matches, redo the pools live, and compute
//! ranked standings. Use cases on [`App`]; shared read helpers stay on [`crate`].

use std::collections::HashSet;

use domain::generation::round_robin_pairs;
use domain::ids::{MatchId, PoolId, TeamId, TournamentId};
use domain::matches::MatchCommand;
use domain::scheduling::MatchView;
use domain::standings::{pool_standings, MatchResult};

use crate::{App, AppError, PoolStandingsView, StandingRow};

impl App {
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
        let existing: HashSet<(TeamId, TeamId)> = self
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
}
