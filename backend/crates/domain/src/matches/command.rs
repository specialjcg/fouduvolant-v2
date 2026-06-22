use super::*;

/// Commands accepted by the [`Match`] aggregate.
#[derive(Debug, Clone)]
pub enum MatchCommand {
    /// Create the match.
    Schedule {
        /// Identity of the match being created.
        match_id: MatchId,
        /// Tournament this match belongs to.
        tournament_id: TournamentId,
        /// Best-of-1 (pool) or best-of-3 (bracket).
        format: MatchFormat,
        /// First side.
        team_a: TeamId,
        /// Second side.
        team_b: TeamId,
        /// Pool this match belongs to, if any (bracket matches have none).
        pool_id: Option<PoolId>,
    },
    /// Put the match on a court and begin play.
    Start {
        /// Court the match is played on.
        court_id: CourtId,
    },
    /// Undo a `Start`: a live match that was started by mistake goes back to
    /// scheduled and releases its court. Only valid while in progress.
    Unstart,
    /// Record one finished set, by raw points. The aggregate validates the
    /// badminton rules and auto-completes the match when decided.
    RecordSet {
        /// Points for side A.
        a: u8,
        /// Points for side B.
        b: u8,
    },
    /// Correct the score of a started or completed match (single decisive set).
    /// Replaces the recorded score and recomputes the winner.
    Rescore {
        /// Points for side A.
        a: u8,
        /// Points for side B.
        b: u8,
    },
    /// End the match by forfeit (no-show before start) or retirement (abandon
    /// during play): the named team wins, keeping any sets already played.
    Concede {
        /// The team that wins.
        winner: TeamId,
    },
}
