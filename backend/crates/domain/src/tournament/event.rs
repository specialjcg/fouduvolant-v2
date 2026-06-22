use super::*;

/// Events emitted by the [`Tournament`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TournamentEvent {
    /// The tournament was created.
    Created {
        /// Identity of the tournament.
        tournament_id: TournamentId,
        /// Display name.
        name: String,
        /// Format for pool matches.
        pool_format: MatchFormat,
        /// Format for bracket matches.
        bracket_format: MatchFormat,
    },
    /// A team was registered.
    TeamRegistered {
        /// Team identity.
        team_id: TeamId,
        /// Team display name.
        name: String,
        /// First player.
        player1: String,
        /// Second player.
        player2: String,
    },
    /// A team was removed.
    TeamRemoved {
        /// Team that was removed.
        team_id: TeamId,
    },
    /// A team forfeited (withdrew / no-show) after the draft.
    TeamForfeited {
        /// Team that forfeited.
        team_id: TeamId,
    },
    /// The pool composition was set.
    PoolsGenerated {
        /// Full set of pools.
        pools: Vec<Pool>,
    },
    /// The available courts were declared.
    CourtsConfigured {
        /// Court identities.
        courts: Vec<CourtId>,
    },
    /// A pool was pinned to a court.
    PoolCourtAssigned {
        /// Pool assigned.
        pool_id: PoolId,
        /// Court it plays on.
        court_id: CourtId,
    },
    /// The pool stage began.
    PoolPhaseStarted,
    /// The bracket stage began.
    BracketPhaseStarted,
    /// The tournament was reset to draft.
    DraftReopened,
    /// The bracket match format was changed.
    BracketFormatSet {
        /// New format for bracket matches.
        format: MatchFormat,
    },
    /// The format for one bracket round (by team count) was set.
    BracketRoundFormatSet {
        /// Number of teams in the round.
        round_size: u16,
        /// Format for that round.
        format: MatchFormat,
    },
}

impl DomainEvent for TournamentEvent {
    fn event_type(&self) -> String {
        match self {
            TournamentEvent::Created { .. } => "TournamentCreated",
            TournamentEvent::TeamRegistered { .. } => "TeamRegistered",
            TournamentEvent::TeamRemoved { .. } => "TeamRemoved",
            TournamentEvent::TeamForfeited { .. } => "TeamForfeited",
            TournamentEvent::PoolsGenerated { .. } => "PoolsGenerated",
            TournamentEvent::CourtsConfigured { .. } => "CourtsConfigured",
            TournamentEvent::PoolCourtAssigned { .. } => "PoolCourtAssigned",
            TournamentEvent::PoolPhaseStarted => "PoolPhaseStarted",
            TournamentEvent::BracketPhaseStarted => "BracketPhaseStarted",
            TournamentEvent::DraftReopened => "DraftReopened",
            TournamentEvent::BracketFormatSet { .. } => "BracketFormatSet",
            TournamentEvent::BracketRoundFormatSet { .. } => "BracketRoundFormatSet",
        }
        .to_string()
    }

    fn event_version(&self) -> String {
        "1.0".to_string()
    }
}

/// Errors returned when a [`TournamentCommand`] is rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TournamentError {
    /// `Create` on a tournament that already exists.
    #[error("tournament already created")]
    AlreadyCreated,
    /// A setup command issued outside the `Draft` phase.
    #[error("tournament is not in draft (current phase forbids setup changes)")]
    NotInDraft,
    /// A team id was registered twice.
    #[error("team already registered")]
    DuplicateTeam,
    /// A referenced team is not registered.
    #[error("unknown team")]
    UnknownTeam,
    /// A forfeit was requested during draft (remove the team instead).
    #[error("cannot forfeit a team during draft (remove it instead)")]
    CannotForfeitInDraft,
    /// Pool composition is invalid.
    #[error("invalid pools: {0}")]
    InvalidPools(&'static str),
    /// Court list is invalid.
    #[error("invalid courts: {0}")]
    InvalidCourts(&'static str),
    /// Pool→court assignment is invalid.
    #[error("cannot assign pool to court: {0}")]
    CannotAssign(&'static str),
    /// Cannot start the pool phase yet.
    #[error("cannot start pool phase: {0}")]
    CannotStartPoolPhase(&'static str),
    /// Cannot start the bracket phase from the current phase.
    #[error("cannot start bracket phase: pool phase must be in progress")]
    CannotStartBracketPhase,
}
