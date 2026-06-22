//! Match feature: start on a court, dispatch, score, correct, forfeit. Use cases
//! on [`App`]; the `match_cmd` dispatcher and shared read helpers stay on [`crate`].

use domain::ids::{CourtId, MatchId, TeamId, TournamentId};
use domain::matches::MatchCommand;
use domain::scheduling::{plan, SchedStatus};
use domain::tournament::TournamentCommand;

use crate::{App, AppError};

impl App {
    /// Start a match on a court, refusing if another match is already in progress
    /// on that court (avoids two live matches on one terrain).
    ///
    /// # Errors
    /// Returns [`AppError`] if the court is busy, or on a command/store failure.
    pub async fn start_match(&self, match_id: MatchId, court_id: CourtId) -> Result<(), AppError> {
        let projection = self.match_projection().await?;
        let views = projection.views();
        if views
            .iter()
            .any(|v| v.court == Some(court_id) && v.status == SchedStatus::Playing && v.id != match_id)
        {
            return Err(AppError::Command("terrain déjà occupé".into()));
        }
        // A team cannot play two matches at once: refuse if either side is
        // already on court in another live match.
        if let Some(me) = views.iter().find(|v| v.id == match_id) {
            let busy = views.iter().any(|v| {
                v.id != match_id
                    && v.status == SchedStatus::Playing
                    && (v.team_a == me.team_a
                        || v.team_a == me.team_b
                        || v.team_b == me.team_a
                        || v.team_b == me.team_b)
            });
            if busy {
                return Err(AppError::Command(
                    "une équipe joue déjà sur un autre terrain".into(),
                ));
            }
        }
        self.match_cmd(match_id, MatchCommand::Start { court_id })
            .await
            .map_err(|e| AppError::Command(e.to_string()))
    }

    /// Undo a mistaken start: a live match goes back to the pending queue and
    /// releases its court (e.g. it was dispatched onto a team already playing).
    ///
    /// # Errors
    /// Returns [`AppError`] if the match is not in progress, or on a store failure.
    pub async fn unstart_match(&self, match_id: MatchId) -> Result<(), AppError> {
        self.match_cmd(match_id, MatchCommand::Unstart)
            .await
            .map_err(|e| AppError::Command(e.to_string()))
    }

    /// Scheduling process manager (pull-based): plan this tournament's courts and
    /// start the suggested next match on every free court. Forced back-to-back
    /// suggestions are left for manual confirmation.
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

        // Teams already on court; a team must never be started into a second
        // simultaneous match, even if two plans land on it in the same pass.
        let mut playing: std::collections::HashSet<TeamId> = views
            .iter()
            .filter(|v| v.status == SchedStatus::Playing)
            .flat_map(|v| [v.team_a, v.team_b])
            .collect();

        let mut started = Vec::new();
        for cp in plans {
            if cp.current.is_some() {
                continue;
            }
            let Some(next) = cp.next else { continue };
            if next.needs_rest {
                continue;
            }
            let Some(nv) = views.iter().find(|v| v.id == next.match_id) else {
                continue;
            };
            if playing.contains(&nv.team_a) || playing.contains(&nv.team_b) {
                continue;
            }
            self.match_cmd(next.match_id, MatchCommand::Start { court_id: cp.court })
                .await
                .map_err(|e| AppError::Command(e.to_string()))?;
            playing.insert(nv.team_a);
            playing.insert(nv.team_b);
            started.push(next.match_id);
        }
        Ok(started)
    }

    /// Record a set, then — for a bracket match — advance the draw so any match
    /// whose two teams are now known is scheduled immediately.
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

    /// Forfeit (withdraw) a whole team after the draft: record the forfeit badge
    /// on the tournament, then concede every not-yet-finished match the team is
    /// in (the opponent wins, keeping any sets already played) and advance the
    /// bracket so the draw moves on.
    ///
    /// # Errors
    /// Returns [`AppError`] if the tournament is still in draft, the team is
    /// unknown, or on a command / store failure.
    pub async fn forfeit_team(
        &self,
        tournament_id: TournamentId,
        team_id: TeamId,
    ) -> Result<(), AppError> {
        // Record the badge first — this validates phase + team membership.
        self.tournament(tournament_id, TournamentCommand::ForfeitTeam { team_id })
            .await
            .map_err(|e| AppError::Command(e.to_string()))?;

        // Concede each pending / in-progress match the team is part of.
        let to_concede: Vec<_> = self
            .match_projection()
            .await?
            .views()
            .into_iter()
            .filter(|v| {
                v.tournament == tournament_id
                    && v.status != SchedStatus::Done
                    && (v.team_a == team_id || v.team_b == team_id)
            })
            .collect();

        let mut touched_bracket = false;
        for v in to_concede {
            let winner = if v.team_a == team_id {
                v.team_b
            } else {
                v.team_a
            };
            self.match_cmd(v.id, MatchCommand::Concede { winner })
                .await
                .map_err(|e| AppError::Command(e.to_string()))?;
            if v.pool.is_none() {
                touched_bracket = true;
            }
        }
        if touched_bracket {
            self.advance_bracket(tournament_id).await?;
        }
        Ok(())
    }

    /// Correct a match's score, then — if it is a bracket match — reconcile the
    /// downstream bracket.
    ///
    /// # Errors
    /// Returns [`AppError`] on a command or database failure.
    pub async fn rescore_match(&self, match_id: MatchId, a: u8, b: u8) -> Result<(), AppError> {
        self.match_cmd(match_id, MatchCommand::Rescore { a, b })
            .await
            .map_err(|e| AppError::Command(e.to_string()))?;
        if let Some(view) = self.match_projection().await?.get(match_id) {
            if view.pool.is_none() {
                self.reconcile_bracket(view.tournament).await?;
            }
        }
        Ok(())
    }
}
