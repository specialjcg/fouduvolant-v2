//! Read-side projections built by folding aggregate event streams.
//!
//! [`MatchProjection`] turns the `Match` aggregates' events into the
//! [`MatchView`]s the scheduler consumes. It is the bridge between the write
//! model ([`crate::matches`]) and the planner ([`crate::scheduling`]).
//!
//! Two pieces of ordering metadata that no single `Match` stream carries are
//! synthesised here from the *global* order events are applied:
//!   - `seq` — creation order, assigned on first `Scheduled`.
//!   - `done_order` — completion order, assigned on `Completed`.
//!
//! `manual_court` (the ▶ override) is a scheduling concern, not a `Match` event;
//! it is left `None` and set via [`MatchProjection::set_manual_court`] when wired
//! to the scheduling command side.

use std::collections::HashMap;

use crate::ids::{CourtId, MatchId};
use crate::matches::MatchEvent;
use crate::scheduling::{MatchView, SchedStatus};

/// Folds `Match` events into [`MatchView`]s, keyed by match id.
///
/// Apply every `Match` event in global commit order via [`Self::apply`]; read
/// the result with [`Self::views`] to feed [`crate::scheduling::plan`].
#[derive(Debug, Default)]
pub struct MatchProjection {
    views: HashMap<MatchId, MatchView>,
    next_seq: u32,
    next_done: u32,
}

impl MatchProjection {
    /// An empty projection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one event for the match aggregate identified by `id`.
    pub fn apply(&mut self, id: MatchId, event: &MatchEvent) {
        match event {
            MatchEvent::Scheduled {
                tournament_id,
                team_a,
                team_b,
                pool_id,
                ..
            } => {
                let seq = self.next_seq;
                self.next_seq += 1;
                self.views.insert(
                    id,
                    MatchView {
                        id,
                        tournament: *tournament_id,
                        seq,
                        pool: *pool_id,
                        team_a: *team_a,
                        team_b: *team_b,
                        status: SchedStatus::Pending,
                        court: None,
                        manual_court: None,
                        done_order: None,
                        winner: None,
                        points_a: 0,
                        points_b: 0,
                    },
                );
            }
            MatchEvent::MatchStarted { court_id } => {
                if let Some(v) = self.views.get_mut(&id) {
                    v.status = SchedStatus::Playing;
                    v.court = Some(*court_id);
                }
            }
            MatchEvent::SetRecorded { set } => {
                if let Some(v) = self.views.get_mut(&id) {
                    v.points_a += u16::from(set.a());
                    v.points_b += u16::from(set.b());
                }
            }
            MatchEvent::Completed { winner } => {
                if let Some(v) = self.views.get_mut(&id) {
                    v.status = SchedStatus::Done;
                    v.winner = Some(*winner);
                    v.done_order = Some(self.next_done);
                    self.next_done += 1;
                }
            }
        }
    }

    /// Set (or clear) the manual court override for a match — the ▶ action.
    pub fn set_manual_court(&mut self, id: MatchId, court: Option<CourtId>) {
        if let Some(v) = self.views.get_mut(&id) {
            v.manual_court = court;
        }
    }

    /// All match views, ordered by creation sequence (deterministic).
    #[must_use]
    pub fn views(&self) -> Vec<MatchView> {
        let mut v: Vec<MatchView> = self.views.values().cloned().collect();
        v.sort_by_key(|m| m.seq);
        v
    }

    /// Look up a single view.
    #[must_use]
    pub fn get(&self, id: MatchId) -> Option<&MatchView> {
        self.views.get(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{PoolId, TeamId, TournamentId};
    use crate::score::{MatchFormat, SetScore};
    use crate::scheduling::{plan, SchedStatus};
    use std::collections::HashMap;

    fn scheduled(id: MatchId, pool: PoolId, a: TeamId, b: TeamId) -> MatchEvent {
        MatchEvent::Scheduled {
            match_id: id,
            tournament_id: TournamentId::new(),
            format: MatchFormat::BestOf1,
            team_a: a,
            team_b: b,
            pool_id: Some(pool),
        }
    }

    #[test]
    fn folds_lifecycle_into_view() {
        let mut proj = MatchProjection::new();
        let (id, pool) = (MatchId::new(), PoolId::new());
        let (a, b) = (TeamId::new(), TeamId::new());
        let court = CourtId::new();

        proj.apply(id, &scheduled(id, pool, a, b));
        assert_eq!(proj.get(id).unwrap().status, SchedStatus::Pending);

        proj.apply(id, &MatchEvent::MatchStarted { court_id: court });
        let v = proj.get(id).unwrap();
        assert_eq!(v.status, SchedStatus::Playing);
        assert_eq!(v.court, Some(court));

        proj.apply(
            id,
            &MatchEvent::SetRecorded {
                set: SetScore::new(21, 10).unwrap(),
            },
        );
        proj.apply(id, &MatchEvent::Completed { winner: a });
        let v = proj.get(id).unwrap();
        assert_eq!(v.status, SchedStatus::Done);
        assert_eq!(v.done_order, Some(0));
    }

    #[test]
    fn seq_and_done_order_follow_global_order() {
        let mut proj = MatchProjection::new();
        let pool = PoolId::new();
        let (m1, m2) = (MatchId::new(), MatchId::new());
        proj.apply(m1, &scheduled(m1, pool, TeamId::new(), TeamId::new()));
        proj.apply(m2, &scheduled(m2, pool, TeamId::new(), TeamId::new()));
        assert_eq!(proj.get(m1).unwrap().seq, 0);
        assert_eq!(proj.get(m2).unwrap().seq, 1);

        // Complete m2 first, then m1: done_order reflects completion order.
        proj.apply(m2, &MatchEvent::Completed { winner: TeamId::new() });
        proj.apply(m1, &MatchEvent::Completed { winner: TeamId::new() });
        assert_eq!(proj.get(m2).unwrap().done_order, Some(0));
        assert_eq!(proj.get(m1).unwrap().done_order, Some(1));
    }

    #[test]
    fn views_feed_the_planner() {
        let mut proj = MatchProjection::new();
        let pool = PoolId::new();
        let court = CourtId::new();
        for _ in 0..2 {
            let id = MatchId::new();
            proj.apply(id, &scheduled(id, pool, TeamId::new(), TeamId::new()));
        }
        let plans = plan(&proj.views(), &[court], &HashMap::new());
        assert_eq!(plans.len(), 1);
        assert!(plans[0].next.is_some(), "planner suggests from projected views");
    }
}
