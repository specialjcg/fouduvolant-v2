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

pub(crate) use cqrs_es::event_sink::EventSink;
pub(crate) use cqrs_es::{Aggregate, DomainEvent};
pub(crate) use serde::{Deserialize, Serialize};

pub(crate) use crate::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
pub(crate) use crate::score::{MatchFormat, ScoreError, SetOutcome, SetScore};

mod aggregate;
mod command;
mod event;

pub use aggregate::*;
pub use command::*;
pub use event::*;

#[cfg(test)]
mod tests;
