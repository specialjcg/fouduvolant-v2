//! Match feature: start on a court, dispatch, score, correct, forfeit. Use cases
//! on [`App`]; the `match_cmd` dispatcher and shared read helpers stay on [`crate`].

use domain::ids::{CourtId, MatchId, TeamId, TournamentId};
use domain::matches::MatchCommand;
use domain::scheduling::{plan, SchedStatus};

use crate::{App, AppError};

impl App {
    /// Start a match on a court, refusing if another match is already in progress
    /// on that court (avoids two live matches on one terrain).
    ///
    /// # Errors
    /// Returns [`AppError`] if the court is busy, or on a command/store failure.
    pub async fn start_match(&self, match_id: MatchId, court_id: CourtId) -> Result<(), AppError> {
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

        let mut started = Vec::new();
        for cp in plans {
            if cp.current.is_some() {
                continue;
            }
            let Some(next) = cp.next else { continue };
            if next.needs_rest {
                continue;
            }
            self.match_cmd(next.match_id, MatchCommand::Start { court_id: cp.court })
                .await
                .map_err(|e| AppError::Command(e.to_string()))?;
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
