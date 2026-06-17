use super::*;

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
    pub(super) status: MatchStatus,
    tournament: Option<TournamentId>,
    format: Option<MatchFormat>,
    team_a: Option<TeamId>,
    team_b: Option<TeamId>,
    pool_id: Option<PoolId>,
    court: Option<CourtId>,
    sets: Vec<SetScore>,
    pub(super) winner: Option<TeamId>,
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
