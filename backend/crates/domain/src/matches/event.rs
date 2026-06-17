use super::*;

/// Events emitted by the [`Match`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchEvent {
    /// The match was created.
    Scheduled {
        /// Identity of the match.
        match_id: MatchId,
        /// Tournament this match belongs to.
        tournament_id: TournamentId,
        /// Match format.
        format: MatchFormat,
        /// First side.
        team_a: TeamId,
        /// Second side.
        team_b: TeamId,
        /// Owning pool, if any.
        pool_id: Option<PoolId>,
    },
    /// The match was placed on a court and play began.
    MatchStarted {
        /// Court the match is played on.
        court_id: CourtId,
    },
    /// A finished set was recorded.
    SetRecorded {
        /// The validated set score.
        set: SetScore,
    },
    /// Enough sets were won; the match is decided.
    Completed {
        /// Winning team.
        winner: TeamId,
    },
    /// The score was corrected after the fact (replaces the set, sets the winner).
    Rescored {
        /// The corrected set.
        set: SetScore,
        /// Recomputed winner.
        winner: TeamId,
    },
    /// The match ended by forfeit / retirement; `winner` takes it.
    Conceded {
        /// The team that wins.
        winner: TeamId,
    },
}

impl DomainEvent for MatchEvent {
    fn event_type(&self) -> String {
        match self {
            MatchEvent::Scheduled { .. } => "MatchScheduled",
            MatchEvent::MatchStarted { .. } => "MatchStarted",
            MatchEvent::SetRecorded { .. } => "SetRecorded",
            MatchEvent::Completed { .. } => "MatchCompleted",
            MatchEvent::Rescored { .. } => "ScoreCorrected",
            MatchEvent::Conceded { .. } => "Conceded",
        }
        .to_string()
    }

    fn event_version(&self) -> String {
        "1.0".to_string()
    }
}

/// Errors returned when a [`MatchCommand`] is rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MatchError {
    /// `Schedule` on a match that already exists.
    #[error("match already scheduled")]
    AlreadyScheduled,
    /// Any command other than `Schedule` before the match exists.
    #[error("match has not been scheduled yet")]
    NotScheduled,
    /// Scoring before the match was started (placed on a court).
    #[error("match has not been started yet")]
    NotStarted,
    /// `Start` on a match that is already in progress.
    #[error("match is already in progress")]
    AlreadyStarted,
    /// Any command after completion.
    #[error("match is already completed")]
    AlreadyCompleted,
    /// `Schedule` with identical teams.
    #[error("a team cannot play against itself")]
    SameTeam,
    /// `Concede` naming a team that is not in this match.
    #[error("the conceding winner is not a team of this match")]
    UnknownWinner,
    /// The proposed set score is not a valid finished set.
    #[error(transparent)]
    Score(#[from] ScoreError),
}
