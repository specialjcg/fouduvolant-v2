//! Tournament feature: command dispatch, listing, reset and hard-delete. Use
//! cases on [`App`]; the read folds (`tournament_view`, …) stay on [`crate`].

use cqrs_es::AggregateError;
use domain::ids::TournamentId;
use domain::tournament::{TournamentCommand, TournamentError};

use crate::infrastructure::EventStore;
use crate::{App, AppError, TournamentSummary};

impl App {
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
}
