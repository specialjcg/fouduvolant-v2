//! Board feature: the live court board and the per-court forecast. Use cases on
//! [`App`]; the read folds (`tournament_courts`, …) stay on [`crate`].

use std::collections::HashMap;

use domain::ids::{MatchId, TeamId, TournamentId};
use domain::scheduling::{forecast, plan, MatchView};

use crate::{App, AppError, BoardView, ForecastCourt, ForecastMatch, MATCH_MINUTES};

impl App {
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
        let by_id: HashMap<MatchId, MatchView> =
            views.iter().map(|v| (v.id, v.clone())).collect();

        let (team_names, pool_names) = match self.tournament_view(tournament_id).await? {
            Some(view) => (
                view.teams
                    .iter()
                    .map(|t| (t.id, t.name.clone()))
                    .collect::<HashMap<_, _>>(),
                view.pools
                    .iter()
                    .map(|p| (p.id, p.name.clone()))
                    .collect::<HashMap<_, _>>(),
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
}
