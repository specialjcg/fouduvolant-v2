use super::*;

/// Commands accepted by the [`Tournament`] aggregate.
#[derive(Debug, Clone)]
pub enum TournamentCommand {
    /// Create the tournament.
    Create {
        /// Identity of the tournament.
        tournament_id: TournamentId,
        /// Display name.
        name: String,
        /// Format for pool matches.
        pool_format: MatchFormat,
        /// Format for bracket matches.
        bracket_format: MatchFormat,
    },
    /// Register a team during draft.
    RegisterTeam {
        /// Team identity.
        team_id: TeamId,
        /// Team display name.
        name: String,
        /// First player.
        player1: String,
        /// Second player.
        player2: String,
    },
    /// Remove a previously registered team during draft.
    RemoveTeam {
        /// Team to remove.
        team_id: TeamId,
    },
    /// Replace the pool composition (computed by an application service).
    GeneratePools {
        /// Full set of pools.
        pools: Vec<Pool>,
    },
    /// Declare the available courts.
    ConfigureCourts {
        /// Court identities.
        courts: Vec<CourtId>,
    },
    /// Pin a pool to a specific court (manual scheduling).
    AssignPoolCourt {
        /// Pool to assign.
        pool_id: PoolId,
        /// Court it should play on.
        court_id: CourtId,
    },
    /// Lock setup and begin the pool stage.
    StartPoolPhase,
    /// Begin the elimination bracket stage.
    StartBracketPhase,
    /// Reopen the draft (after a reset) to allow editing teams/pools again.
    ReopenDraft,
    /// Change the bracket (finals) match format — e.g. switch finals between a
    /// single set and best-of-3. Takes effect on the next bracket draw.
    SetBracketFormat {
        /// New format for bracket matches.
        format: MatchFormat,
    },
    /// Set the format for one bracket round, by team count (2 = final, 4 = semis,
    /// 8 = quarters, …). Takes effect on the next draw.
    SetBracketRoundFormat {
        /// Number of teams in the round.
        round_size: u16,
        /// Format for that round.
        format: MatchFormat,
    },
}
