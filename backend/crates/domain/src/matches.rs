//! The `Match` aggregate: a single match between two teams.
//!
//! Lifecycle: `Schedule` → `Start{court}` → `RecordSet`+ → auto-`Completed`.
//! Scoring rules live in [`crate::score`]; this aggregate enforces sequencing
//! (can't start before scheduling, can't score before starting, can't score a
//! finished match) and decides when enough sets have been won to finish.
//!
//! The court here is where the match is actually *played*. Scheduling hints
//! (suggested / manually-pinned court) are a read-side concern and live outside
//! this aggregate — see [`crate::scheduling`].

use cqrs_es::event_sink::EventSink;
use cqrs_es::{Aggregate, DomainEvent};
use serde::{Deserialize, Serialize};

use crate::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
use crate::score::{MatchFormat, ScoreError, SetOutcome, SetScore};

/// Status of a match within its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MatchStatus {
    /// Aggregate has no events yet.
    #[default]
    NotStarted,
    /// Created but not yet placed on a court.
    Scheduled,
    /// Placed on a court and accepting scores.
    InProgress,
    /// Decided; no further scoring allowed.
    Completed,
}

/// The match aggregate state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Match {
    status: MatchStatus,
    tournament: Option<TournamentId>,
    format: Option<MatchFormat>,
    team_a: Option<TeamId>,
    team_b: Option<TeamId>,
    pool_id: Option<PoolId>,
    court: Option<CourtId>,
    sets: Vec<SetScore>,
    winner: Option<TeamId>,
}

impl Match {
    /// Count set wins per side from the recorded sets.
    fn set_wins(&self) -> (u8, u8) {
        let mut a = 0u8;
        let mut b = 0u8;
        for set in &self.sets {
            match set.winner() {
                SetOutcome::SideA => a += 1,
                SetOutcome::SideB => b += 1,
            }
        }
        (a, b)
    }
}

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

impl Aggregate for Match {
    const TYPE: &'static str = "Match";
    type Command = MatchCommand;
    type Event = MatchEvent;
    type Error = MatchError;
    type Services = ();

    async fn handle(
        &mut self,
        command: Self::Command,
        _services: &Self::Services,
        sink: &EventSink<Self>,
    ) -> Result<(), Self::Error> {
        match command {
            MatchCommand::Schedule {
                match_id,
                tournament_id,
                format,
                team_a,
                team_b,
                pool_id,
            } => {
                if self.status != MatchStatus::NotStarted {
                    return Err(MatchError::AlreadyScheduled);
                }
                if team_a == team_b {
                    return Err(MatchError::SameTeam);
                }
                sink.write(
                    MatchEvent::Scheduled {
                        match_id,
                        tournament_id,
                        format,
                        team_a,
                        team_b,
                        pool_id,
                    },
                    self,
                )
                .await;
            }

            MatchCommand::Start { court_id } => match self.status {
                MatchStatus::NotStarted => return Err(MatchError::NotScheduled),
                MatchStatus::InProgress => return Err(MatchError::AlreadyStarted),
                MatchStatus::Completed => return Err(MatchError::AlreadyCompleted),
                MatchStatus::Scheduled => {
                    sink.write(MatchEvent::MatchStarted { court_id }, self).await;
                }
            },

            MatchCommand::RecordSet { a, b } => {
                match self.status {
                    MatchStatus::NotStarted => return Err(MatchError::NotScheduled),
                    MatchStatus::Scheduled => return Err(MatchError::NotStarted),
                    MatchStatus::Completed => return Err(MatchError::AlreadyCompleted),
                    MatchStatus::InProgress => {}
                }
                let format = self.format.expect("scheduled match has a format");

                // Defense in depth: never record a set into an already-decided
                // match, even if the status still reads InProgress (guards a
                // malformed stream). The status check above covers the normal
                // path; this covers the impossible-by-construction one.
                let (wins_a, wins_b) = self.set_wins();
                let needed = format.sets_to_win();
                if wins_a >= needed || wins_b >= needed {
                    return Err(MatchError::AlreadyCompleted);
                }

                let set = SetScore::new(a, b)?;

                // `write` applies the event immediately, so `set_wins` below
                // already reflects this set.
                sink.write(MatchEvent::SetRecorded { set }, self).await;

                let (wins_a, wins_b) = self.set_wins();
                let needed = format.sets_to_win();
                if wins_a >= needed || wins_b >= needed {
                    let winner = if wins_a > wins_b {
                        self.team_a.expect("scheduled match has team_a")
                    } else {
                        self.team_b.expect("scheduled match has team_b")
                    };
                    sink.write(MatchEvent::Completed { winner }, self).await;
                }
            }

            MatchCommand::Rescore { a, b } => {
                match self.status {
                    MatchStatus::NotStarted => return Err(MatchError::NotScheduled),
                    MatchStatus::Scheduled => return Err(MatchError::NotStarted),
                    MatchStatus::InProgress | MatchStatus::Completed => {}
                }
                let set = SetScore::new(a, b)?;
                let winner = match set.winner() {
                    SetOutcome::SideA => self.team_a.expect("started match has team_a"),
                    SetOutcome::SideB => self.team_b.expect("started match has team_b"),
                };
                sink.write(MatchEvent::Rescored { set, winner }, self).await;
            }

            MatchCommand::Concede { winner } => {
                match self.status {
                    MatchStatus::NotStarted => return Err(MatchError::NotScheduled),
                    MatchStatus::Completed => return Err(MatchError::AlreadyCompleted),
                    MatchStatus::Scheduled | MatchStatus::InProgress => {}
                }
                if Some(winner) != self.team_a && Some(winner) != self.team_b {
                    return Err(MatchError::UnknownWinner);
                }
                sink.write(MatchEvent::Conceded { winner }, self).await;
            }
        }
        Ok(())
    }

    fn apply(&mut self, event: Self::Event) {
        match event {
            MatchEvent::Scheduled {
                tournament_id,
                format,
                team_a,
                team_b,
                pool_id,
                ..
            } => {
                self.status = MatchStatus::Scheduled;
                self.tournament = Some(tournament_id);
                self.format = Some(format);
                self.team_a = Some(team_a);
                self.team_b = Some(team_b);
                self.pool_id = pool_id;
            }
            MatchEvent::MatchStarted { court_id } => {
                self.status = MatchStatus::InProgress;
                self.court = Some(court_id);
            }
            MatchEvent::SetRecorded { set } => {
                self.sets.push(set);
            }
            MatchEvent::Completed { winner } => {
                self.status = MatchStatus::Completed;
                self.winner = Some(winner);
            }
            MatchEvent::Rescored { set, winner } => {
                self.sets = vec![set];
                self.winner = Some(winner);
                self.status = MatchStatus::Completed;
            }
            MatchEvent::Conceded { winner } => {
                self.winner = Some(winner);
                self.status = MatchStatus::Completed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a command, returning the events it emitted. The aggregate is mutated
    /// in place (events are applied as they are written).
    async fn exec(m: &mut Match, cmd: MatchCommand) -> Result<Vec<MatchEvent>, MatchError> {
        let sink = EventSink::default();
        m.handle(cmd, &(), &sink).await?;
        Ok(sink.collect().await)
    }

    fn scheduled(format: MatchFormat) -> (Match, TeamId, TeamId) {
        let (a, b) = (TeamId::new(), TeamId::new());
        let mut m = Match::default();
        m.apply(MatchEvent::Scheduled {
            match_id: MatchId::new(),
            tournament_id: TournamentId::new(),
            format,
            team_a: a,
            team_b: b,
            pool_id: None,
        });
        (m, a, b)
    }

    fn started(format: MatchFormat) -> (Match, TeamId, TeamId) {
        let (mut m, a, b) = scheduled(format);
        m.apply(MatchEvent::MatchStarted {
            court_id: CourtId::new(),
        });
        (m, a, b)
    }

    #[tokio::test]
    async fn concede_completes_with_named_winner() {
        // No-show before start: conceding a scheduled match is allowed.
        let (mut m, a, b) = scheduled(MatchFormat::BestOf1);
        let events = exec(&mut m, MatchCommand::Concede { winner: a }).await.unwrap();
        assert_eq!(events, vec![MatchEvent::Conceded { winner: a }]);
        assert_eq!(m.status, MatchStatus::Completed);
        assert_eq!(m.winner, Some(a));

        // A non-participant cannot be the winner.
        let (mut m2, _, _) = started(MatchFormat::BestOf3);
        let outsider = TeamId::new();
        assert!(matches!(
            exec(&mut m2, MatchCommand::Concede { winner: outsider }).await,
            Err(MatchError::UnknownWinner)
        ));
        let _ = b;
    }

    #[tokio::test]
    async fn cannot_concede_a_completed_match() {
        let (mut m, a, _b) = started(MatchFormat::BestOf1);
        exec(&mut m, MatchCommand::RecordSet { a: 21, b: 0 }).await.unwrap();
        assert!(matches!(
            exec(&mut m, MatchCommand::Concede { winner: a }).await,
            Err(MatchError::AlreadyCompleted)
        ));
    }

    #[tokio::test]
    async fn cannot_schedule_twice() {
        let (mut m, a, b) = scheduled(MatchFormat::BestOf1);
        let err = exec(
            &mut m,
            MatchCommand::Schedule {
                match_id: MatchId::new(),
                tournament_id: TournamentId::new(),
                format: MatchFormat::BestOf1,
                team_a: a,
                team_b: b,
                pool_id: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, MatchError::AlreadyScheduled);
    }

    #[tokio::test]
    async fn cannot_score_before_schedule() {
        let mut m = Match::default();
        let err = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 10 })
            .await
            .unwrap_err();
        assert_eq!(err, MatchError::NotScheduled);
    }

    #[tokio::test]
    async fn cannot_score_before_start() {
        let (mut m, _a, _b) = scheduled(MatchFormat::BestOf1);
        let err = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 10 })
            .await
            .unwrap_err();
        assert_eq!(err, MatchError::NotStarted);
    }

    #[tokio::test]
    async fn start_then_cannot_start_again() {
        let (mut m, _a, _b) = scheduled(MatchFormat::BestOf1);
        let ev = exec(
            &mut m,
            MatchCommand::Start {
                court_id: CourtId::new(),
            },
        )
        .await
        .unwrap();
        assert_eq!(ev.len(), 1);
        let err = exec(
            &mut m,
            MatchCommand::Start {
                court_id: CourtId::new(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, MatchError::AlreadyStarted);
    }

    #[tokio::test]
    async fn bo1_completes_after_one_set() {
        let (mut m, a, _b) = started(MatchFormat::BestOf1);
        let events = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 15 })
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1], MatchEvent::Completed { winner: a });
    }

    #[tokio::test]
    async fn bo3_needs_two_sets() {
        let (mut m, a, _b) = started(MatchFormat::BestOf3);
        let ev = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 18 })
            .await
            .unwrap();
        assert_eq!(ev.len(), 1);
        let ev = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 12 })
            .await
            .unwrap();
        assert_eq!(ev.last(), Some(&MatchEvent::Completed { winner: a }));
    }

    #[tokio::test]
    async fn cannot_score_completed_match() {
        let (mut m, _a, _b) = started(MatchFormat::BestOf1);
        exec(&mut m, MatchCommand::RecordSet { a: 21, b: 0 })
            .await
            .unwrap();
        let err = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 0 })
            .await
            .unwrap_err();
        assert_eq!(err, MatchError::AlreadyCompleted);
    }
}
