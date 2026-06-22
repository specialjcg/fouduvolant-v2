use super::*;

/// Lifecycle phase of a tournament.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Phase {
    /// No `Create` event applied yet.
    #[default]
    NotCreated,
    /// Setup: registering teams, forming pools, declaring courts.
    Draft,
    /// Pool (group) stage in progress.
    PoolPhase,
    /// Elimination bracket stage in progress.
    BracketPhase,
    /// Tournament finished.
    Done,
}

/// A pool (group-stage group) and its members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pool {
    /// Pool identity.
    pub id: PoolId,
    /// Display name (e.g. "Poule 1").
    pub name: String,
    /// Teams assigned to this pool.
    pub teams: Vec<TeamId>,
}

/// The tournament aggregate state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Tournament {
    pub(super) phase: Phase,
    name: String,
    teams: Vec<TeamId>,
    /// Teams that forfeited after the draft (kept in `teams`; badged out).
    forfeited: Vec<TeamId>,
    pools: Vec<Pool>,
    courts: Vec<CourtId>,
    pool_courts: Vec<(PoolId, CourtId)>,
    pool_format: MatchFormat,
    pub(super) bracket_format: MatchFormat,
    /// Per-round bracket format override, keyed by the number of teams in the
    /// round (2 = final, 4 = semis, 8 = quarters, 16 = round of 16, …). Rounds
    /// not present fall back to `bracket_format`.
    bracket_round_formats: std::collections::HashMap<u16, MatchFormat>,
}

impl Aggregate for Tournament {
    const TYPE: &'static str = "Tournament";
    type Command = TournamentCommand;
    type Event = TournamentEvent;
    type Error = TournamentError;
    type Services = ();

    async fn handle(
        &mut self,
        command: Self::Command,
        _services: &Self::Services,
        sink: &EventSink<Self>,
    ) -> Result<(), Self::Error> {
        use TournamentCommand as C;
        use TournamentError as E;

        match command {
            C::Create {
                tournament_id,
                name,
                pool_format,
                bracket_format,
            } => {
                if self.phase != Phase::NotCreated {
                    return Err(E::AlreadyCreated);
                }
                sink.write(
                    TournamentEvent::Created {
                        tournament_id,
                        name,
                        pool_format,
                        bracket_format,
                    },
                    self,
                )
                .await;
            }

            C::RegisterTeam {
                team_id,
                name,
                player1,
                player2,
            } => {
                self.require_draft()?;
                if self.teams.contains(&team_id) {
                    return Err(E::DuplicateTeam);
                }
                sink.write(
                    TournamentEvent::TeamRegistered {
                        team_id,
                        name,
                        player1,
                        player2,
                    },
                    self,
                )
                .await;
            }

            C::RemoveTeam { team_id } => {
                self.require_draft()?;
                if !self.teams.contains(&team_id) {
                    return Err(E::UnknownTeam);
                }
                sink.write(TournamentEvent::TeamRemoved { team_id }, self)
                    .await;
            }

            C::ForfeitTeam { team_id } => {
                if self.phase == Phase::Draft {
                    return Err(E::CannotForfeitInDraft);
                }
                if !self.teams.contains(&team_id) {
                    return Err(E::UnknownTeam);
                }
                // Idempotent: a second forfeit is a no-op.
                if !self.forfeited.contains(&team_id) {
                    sink.write(TournamentEvent::TeamForfeited { team_id }, self)
                        .await;
                }
            }

            C::GeneratePools { pools } => {
                self.require_draft()?;
                self.validate_pools(&pools)?;
                sink.write(TournamentEvent::PoolsGenerated { pools }, self)
                    .await;
            }

            C::SetBracketFormat { format } => {
                sink.write(TournamentEvent::BracketFormatSet { format }, self)
                    .await;
            }

            C::SetBracketRoundFormat { round_size, format } => {
                sink.write(
                    TournamentEvent::BracketRoundFormatSet { round_size, format },
                    self,
                )
                .await;
            }

            C::ConfigureCourts { courts } => {
                self.require_draft()?;
                if courts.is_empty() {
                    return Err(E::InvalidCourts("at least one court is required"));
                }
                let mut seen = courts.clone();
                seen.sort();
                seen.dedup();
                if seen.len() != courts.len() {
                    return Err(E::InvalidCourts("duplicate court"));
                }
                sink.write(TournamentEvent::CourtsConfigured { courts }, self)
                    .await;
            }

            C::AssignPoolCourt { pool_id, court_id } => {
                if self.phase != Phase::Draft && self.phase != Phase::PoolPhase {
                    return Err(E::CannotAssign("only during draft or pool phase"));
                }
                if !self.pools.iter().any(|p| p.id == pool_id) {
                    return Err(E::CannotAssign("unknown pool"));
                }
                if !self.courts.contains(&court_id) {
                    return Err(E::CannotAssign("unknown court"));
                }
                sink.write(
                    TournamentEvent::PoolCourtAssigned { pool_id, court_id },
                    self,
                )
                .await;
            }

            C::StartPoolPhase => {
                self.require_draft()?;
                if self.pools.is_empty() {
                    return Err(E::CannotStartPoolPhase("no pools generated"));
                }
                if self.courts.is_empty() {
                    return Err(E::CannotStartPoolPhase("no courts configured"));
                }
                sink.write(TournamentEvent::PoolPhaseStarted, self).await;
            }

            C::StartBracketPhase => {
                if self.phase != Phase::PoolPhase {
                    return Err(E::CannotStartBracketPhase);
                }
                sink.write(TournamentEvent::BracketPhaseStarted, self).await;
            }

            C::ReopenDraft => {
                if self.phase != Phase::NotCreated {
                    sink.write(TournamentEvent::DraftReopened, self).await;
                }
            }
        }
        Ok(())
    }

    fn apply(&mut self, event: Self::Event) {
        match event {
            TournamentEvent::Created {
                name,
                pool_format,
                bracket_format,
                ..
            } => {
                self.phase = Phase::Draft;
                self.name = name;
                self.pool_format = pool_format;
                self.bracket_format = bracket_format;
            }
            TournamentEvent::TeamRegistered { team_id, .. } => {
                self.teams.push(team_id);
            }
            TournamentEvent::TeamRemoved { team_id } => {
                self.teams.retain(|t| *t != team_id);
            }
            TournamentEvent::TeamForfeited { team_id } => {
                if !self.forfeited.contains(&team_id) {
                    self.forfeited.push(team_id);
                }
            }
            TournamentEvent::PoolsGenerated { pools } => {
                self.pools = pools;
            }
            TournamentEvent::CourtsConfigured { courts } => {
                self.courts = courts;
            }
            TournamentEvent::PoolCourtAssigned { pool_id, court_id } => {
                self.pool_courts.retain(|(p, _)| *p != pool_id);
                self.pool_courts.push((pool_id, court_id));
            }
            TournamentEvent::PoolPhaseStarted => {
                self.phase = Phase::PoolPhase;
            }
            TournamentEvent::BracketPhaseStarted => {
                self.phase = Phase::BracketPhase;
            }
            TournamentEvent::DraftReopened => {
                self.phase = Phase::Draft;
            }
            TournamentEvent::BracketFormatSet { format } => {
                self.bracket_format = format;
            }
            TournamentEvent::BracketRoundFormatSet { round_size, format } => {
                self.bracket_round_formats.insert(round_size, format);
            }
        }
    }
}

impl Tournament {
    fn require_draft(&self) -> Result<(), TournamentError> {
        if self.phase == Phase::Draft {
            Ok(())
        } else {
            Err(TournamentError::NotInDraft)
        }
    }

    fn validate_pools(&self, pools: &[Pool]) -> Result<(), TournamentError> {
        use TournamentError::InvalidPools;
        if pools.is_empty() {
            return Err(InvalidPools("at least one pool is required"));
        }
        let mut seen: Vec<TeamId> = Vec::new();
        for pool in pools {
            if pool.teams.is_empty() {
                return Err(InvalidPools("a pool has no teams"));
            }
            for team in &pool.teams {
                if !self.teams.contains(team) {
                    return Err(InvalidPools("pool references an unregistered team"));
                }
                if seen.contains(team) {
                    return Err(InvalidPools("a team appears in more than one pool"));
                }
                seen.push(*team);
            }
        }
        Ok(())
    }
}
