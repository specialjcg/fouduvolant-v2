//! Bracket feature: draw, advance, reconcile and reset the elimination tree.
//! Use cases on [`App`]; shared read/store helpers live on the [`crate`] façade.

use std::collections::{HashMap, HashSet};

use cqrs_es::AggregateError;
use domain::bracket::{build_bracket, reseed_pool_separation, BracketCommand, BracketError, BracketKind, BracketNode};
use domain::ids::{MatchId, TeamId, TournamentId};
use domain::matches::MatchCommand;
use domain::scheduling::MatchView;
use domain::score::MatchFormat;

use crate::infrastructure::EventStore;
use crate::{App, AppError, BracketNodeView};

impl App {
    /// Draw the bracket from current pool standings: the top `per_pool` of each
    /// pool seed the main draw (rank-major, pools interleaved); the rest seed the
    /// consolation draw. Then schedules the first playable matches.
    ///
    /// Idempotent: re-running with unchanged seeds just advances; with changed
    /// seeds it wipes the stale draw and redraws.
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
        let mut pool_of: HashMap<TeamId, usize> = HashMap::new();
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

        // Compare freshly-computed seeds with the stored draw: identical → keep
        // (just advance); different → the stored draw is stale, wipe it (incl.
        // already-scored garbage) and redraw.
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
                BracketCommand::Draw { main_seeds, consolation_seeds },
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
        let Some((main_seeds, consolation_seeds)) = self.bracket_seeds(tournament_id).await? else {
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
        let mut existing: HashSet<(TeamId, TeamId)> = bracket_matches
            .iter()
            .map(|v| unordered(v.team_a, v.team_b))
            .collect();

        let tree = build_bracket(&main_seeds, &consolation_seeds, &results);
        // Largest round per bracket (final), to turn a round into a team count.
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
                0 => 1u16 << max_r,
                255 => 2,
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

    /// Reset the whole bracket: drop every finals match (main + consolation) and
    /// the draw itself, returning to the "not drawn" state. Pools untouched.
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

    /// Reset a single bracket match so it can be replayed: drop its stream then
    /// reconcile (it is re-created fresh; downstream matches that used its result
    /// are dropped). Pool matches are rejected.
    ///
    /// # Errors
    /// Returns [`AppError`] if the match is unknown, is a pool match, or fails.
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

    /// Drop bracket matches whose pairing is no longer part of the recomputed
    /// tree (e.g. after a re-score flipped an earlier-round winner), then
    /// re-advance to schedule the correct matches.
    pub(crate) async fn reconcile_bracket(&self, tournament_id: TournamentId) -> Result<(), AppError> {
        let Some((main_seeds, consolation_seeds)) = self.bracket_seeds(tournament_id).await? else {
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

        let valid: HashSet<(TeamId, TeamId)> =
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

    /// The bracket tree (main + consolation) with team names resolved.
    ///
    /// # Errors
    /// Returns [`AppError`] on a store or deserialization failure.
    pub async fn bracket_view(
        &self,
        tournament_id: TournamentId,
    ) -> Result<Vec<BracketNodeView>, AppError> {
        let Some((main_seeds, consolation_seeds)) = self.bracket_seeds(tournament_id).await? else {
            return Ok(Vec::new());
        };
        let names: HashMap<TeamId, String> = self
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
    pub(crate) async fn bracket_seeds(
        &self,
        tournament_id: TournamentId,
    ) -> Result<Option<(Vec<TeamId>, Vec<TeamId>)>, AppError> {
        let mut seeds = None;
        for payload in self.db.events_for("Bracket", &tournament_id.to_string()).await? {
            let domain::bracket::BracketEvent::Drawn { main_seeds, consolation_seeds } =
                serde_json::from_value(payload)?;
            seeds = Some((main_seeds, consolation_seeds));
        }
        Ok(seeds)
    }

    /// Hard-delete a tournament's bracket draw so it can be re-seeded.
    pub(crate) async fn delete_bracket(&self, tournament_id: TournamentId) -> Result<(), AppError> {
        self.db.delete_aggregate("Bracket", &tournament_id.to_string())
            .await
    }
}
