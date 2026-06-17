use super::*;


/// The bracket aggregate. Holds the seeded draw for one tournament; the tree is
/// derived elsewhere via [`build_bracket`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Bracket {
    drawn: bool,
    main_seeds: Vec<TeamId>,
    consolation_seeds: Vec<TeamId>,
}

/// Commands for the [`Bracket`] aggregate.
#[derive(Debug, Clone)]
pub enum BracketCommand {
    /// Fix the seeded draw (once).
    Draw {
        /// Qualified teams in seed order (best first).
        main_seeds: Vec<TeamId>,
        /// Non-qualified teams for the consolation draw (best first).
        consolation_seeds: Vec<TeamId>,
    },
}

/// Events for the [`Bracket`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BracketEvent {
    /// The draw was fixed.
    Drawn {
        /// Qualified teams in seed order.
        main_seeds: Vec<TeamId>,
        /// Non-qualified teams for the consolation draw.
        consolation_seeds: Vec<TeamId>,
    },
}

impl DomainEvent for BracketEvent {
    fn event_type(&self) -> String {
        "BracketDrawn".to_string()
    }
    fn event_version(&self) -> String {
        "1.0".to_string()
    }
}

/// Errors from the [`Bracket`] aggregate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BracketError {
    /// The draw was already made.
    #[error("bracket already drawn")]
    AlreadyDrawn,
    /// Fewer than two qualified teams.
    #[error("need at least two qualified teams to draw a bracket")]
    TooFew,
}

impl Aggregate for Bracket {
    const TYPE: &'static str = "Bracket";
    type Command = BracketCommand;
    type Event = BracketEvent;
    type Error = BracketError;
    type Services = ();

    async fn handle(
        &mut self,
        command: Self::Command,
        _services: &Self::Services,
        sink: &EventSink<Self>,
    ) -> Result<(), Self::Error> {
        match command {
            BracketCommand::Draw {
                main_seeds,
                consolation_seeds,
            } => {
                if self.drawn {
                    return Err(BracketError::AlreadyDrawn);
                }
                if main_seeds.len() < 2 {
                    return Err(BracketError::TooFew);
                }
                sink.write(
                    BracketEvent::Drawn {
                        main_seeds,
                        consolation_seeds,
                    },
                    self,
                )
                .await;
            }
        }
        Ok(())
    }

    fn apply(&mut self, event: Self::Event) {
        match event {
            BracketEvent::Drawn {
                main_seeds,
                consolation_seeds,
            } => {
                self.drawn = true;
                self.main_seeds = main_seeds;
                self.consolation_seeds = consolation_seeds;
            }
        }
    }
}
