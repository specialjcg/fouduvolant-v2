//! The `Tournament` aggregate: setup lifecycle before and around play.
//!
//! Owns the consistency boundary for tournament configuration — registering
//! teams, forming pools, declaring courts and advancing phases. It does *not*
//! own match scoring; that is the [`crate::matches::Match`] aggregate.
//!
//! Phases advance one way: `Draft` → `PoolPhase` → `BracketPhase` → `Done`.

use cqrs_es::event_sink::EventSink;
use cqrs_es::{Aggregate, DomainEvent};
use serde::{Deserialize, Serialize};

use crate::ids::{CourtId, PoolId, TeamId, TournamentId};
use crate::score::MatchFormat;

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
    phase: Phase,
    name: String,
    teams: Vec<TeamId>,
    pools: Vec<Pool>,
    courts: Vec<CourtId>,
    pool_format: MatchFormat,
    bracket_format: MatchFormat,
}

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
    /// Lock setup and begin the pool stage.
    StartPoolPhase,
    /// Begin the elimination bracket stage.
    StartBracketPhase,
}

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
    },
    /// A team was removed.
    TeamRemoved {
        /// Team that was removed.
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
    /// The pool stage began.
    PoolPhaseStarted,
    /// The bracket stage began.
    BracketPhaseStarted,
}

impl DomainEvent for TournamentEvent {
    fn event_type(&self) -> String {
        match self {
            TournamentEvent::Created { .. } => "TournamentCreated",
            TournamentEvent::TeamRegistered { .. } => "TeamRegistered",
            TournamentEvent::TeamRemoved { .. } => "TeamRemoved",
            TournamentEvent::PoolsGenerated { .. } => "PoolsGenerated",
            TournamentEvent::CourtsConfigured { .. } => "CourtsConfigured",
            TournamentEvent::PoolPhaseStarted => "PoolPhaseStarted",
            TournamentEvent::BracketPhaseStarted => "BracketPhaseStarted",
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
    /// Pool composition is invalid.
    #[error("invalid pools: {0}")]
    InvalidPools(&'static str),
    /// Court list is invalid.
    #[error("invalid courts: {0}")]
    InvalidCourts(&'static str),
    /// Cannot start the pool phase yet.
    #[error("cannot start pool phase: {0}")]
    CannotStartPoolPhase(&'static str),
    /// Cannot start the bracket phase from the current phase.
    #[error("cannot start bracket phase: pool phase must be in progress")]
    CannotStartBracketPhase,
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

            C::RegisterTeam { team_id, name } => {
                self.require_draft()?;
                if self.teams.contains(&team_id) {
                    return Err(E::DuplicateTeam);
                }
                sink.write(TournamentEvent::TeamRegistered { team_id, name }, self)
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

            C::GeneratePools { pools } => {
                self.require_draft()?;
                self.validate_pools(&pools)?;
                sink.write(TournamentEvent::PoolsGenerated { pools }, self)
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
            TournamentEvent::PoolsGenerated { pools } => {
                self.pools = pools;
            }
            TournamentEvent::CourtsConfigured { courts } => {
                self.courts = courts;
            }
            TournamentEvent::PoolPhaseStarted => {
                self.phase = Phase::PoolPhase;
            }
            TournamentEvent::BracketPhaseStarted => {
                self.phase = Phase::BracketPhase;
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn exec(
        t: &mut Tournament,
        cmd: TournamentCommand,
    ) -> Result<Vec<TournamentEvent>, TournamentError> {
        let sink = EventSink::default();
        t.handle(cmd, &(), &sink).await?;
        Ok(sink.collect().await)
    }

    fn created() -> Tournament {
        let mut t = Tournament::default();
        t.apply(TournamentEvent::Created {
            tournament_id: TournamentId::new(),
            name: "Open".to_string(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf3,
        });
        t
    }

    #[tokio::test]
    async fn cannot_create_twice() {
        let mut t = created();
        let err = exec(
            &mut t,
            TournamentCommand::Create {
                tournament_id: TournamentId::new(),
                name: "X".into(),
                pool_format: MatchFormat::BestOf1,
                bracket_format: MatchFormat::BestOf3,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, TournamentError::AlreadyCreated);
    }

    #[tokio::test]
    async fn rejects_duplicate_team() {
        let mut t = created();
        let id = TeamId::new();
        exec(
            &mut t,
            TournamentCommand::RegisterTeam {
                team_id: id,
                name: "A".into(),
            },
        )
        .await
        .unwrap();
        let err = exec(
            &mut t,
            TournamentCommand::RegisterTeam {
                team_id: id,
                name: "A".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, TournamentError::DuplicateTeam);
    }

    #[tokio::test]
    async fn pools_must_reference_registered_teams() {
        let mut t = created();
        let err = exec(
            &mut t,
            TournamentCommand::GeneratePools {
                pools: vec![Pool {
                    id: PoolId::new(),
                    name: "P1".into(),
                    teams: vec![TeamId::new()],
                }],
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, TournamentError::InvalidPools(_)));
    }

    #[tokio::test]
    async fn start_pool_phase_requires_pools_and_courts() {
        let mut t = created();
        let err = exec(&mut t, TournamentCommand::StartPoolPhase)
            .await
            .unwrap_err();
        assert!(matches!(err, TournamentError::CannotStartPoolPhase(_)));
    }

    #[tokio::test]
    async fn full_setup_flow() {
        let mut t = created();
        let (a, b) = (TeamId::new(), TeamId::new());
        for (id, name) in [(a, "A"), (b, "B")] {
            exec(
                &mut t,
                TournamentCommand::RegisterTeam {
                    team_id: id,
                    name: name.into(),
                },
            )
            .await
            .unwrap();
        }
        exec(
            &mut t,
            TournamentCommand::GeneratePools {
                pools: vec![Pool {
                    id: PoolId::new(),
                    name: "P1".into(),
                    teams: vec![a, b],
                }],
            },
        )
        .await
        .unwrap();
        exec(
            &mut t,
            TournamentCommand::ConfigureCourts {
                courts: vec![CourtId::new()],
            },
        )
        .await
        .unwrap();
        exec(&mut t, TournamentCommand::StartPoolPhase)
            .await
            .unwrap();
        assert_eq!(t.phase, Phase::PoolPhase);
    }
}
