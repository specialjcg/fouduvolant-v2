use super::*;

/// Scheduling-relevant lifecycle of a match, decoupled from the `Match`
/// aggregate so the planner can run over any snapshot source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedStatus {
    /// Not yet played; eligible to be scheduled.
    Pending,
    /// Currently being played on a court.
    Playing,
    /// Finished.
    Done,
}

/// A read-only view of one match, the planner's unit of input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchView {
    /// Match identity.
    pub id: MatchId,
    /// Tournament this match belongs to (used to scope dispatch).
    pub tournament: TournamentId,
    /// Stable creation order; drives all deterministic tiebreaks.
    pub seq: u32,
    /// Owning pool, or `None` for bracket/finals matches.
    pub pool: Option<PoolId>,
    /// First side.
    pub team_a: TeamId,
    /// Second side.
    pub team_b: TeamId,
    /// Scheduling status.
    pub status: SchedStatus,
    /// Court the match is playing on / was played on, if any.
    pub court: Option<CourtId>,
    /// User override: pin this match to a specific court (the ▶ action).
    pub manual_court: Option<CourtId>,
    /// Completion order, used to find the most recent finish per court.
    pub done_order: Option<u32>,
    /// Winner once the match is decided.
    pub winner: Option<TeamId>,
    /// Points scored by side A across all recorded sets.
    pub points_a: u16,
    /// Points scored by side B across all recorded sets.
    pub points_b: u16,
    /// Each recorded set as `(a, b)` in play order — lets the UI show
    /// "21-15 21-10" instead of a summed 42 for best-of-3 matches.
    pub sets: Vec<(u16, u16)>,
    /// True when the match was ended by forfeit / retirement.
    pub conceded: bool,
}

impl MatchView {
    pub(super) fn teams(&self) -> [TeamId; 2] {
        [self.team_a, self.team_b]
    }
}

/// A single proposed match for a court.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// The proposed match.
    pub match_id: MatchId,
    /// True when the proposal forces a team to play back-to-back because no
    /// rested alternative exists (the UI flags "needs rest").
    pub needs_rest: bool,
}

/// The plan for one court: what is playing now, what is next, and a short
/// look-ahead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourtPlan {
    /// Court identity.
    pub court: CourtId,
    /// Match currently being played on this court, if any.
    pub current: Option<MatchId>,
    /// Proposed next match, if one fits.
    pub next: Option<Suggestion>,
    /// Up to [`PREVIEW_DEPTH`] further matches after `next`.
    pub previews: Vec<Suggestion>,
}
