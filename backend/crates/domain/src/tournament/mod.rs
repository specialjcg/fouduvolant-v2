//! The `Tournament` aggregate: setup lifecycle before and around play.
//!
//! Owns the consistency boundary for tournament configuration — registering
//! teams, forming pools, declaring courts and advancing phases. It does *not*
//! own match scoring; that is the [`crate::matches::Match`] aggregate.
//!
//! Phases advance one way: `Draft` → `PoolPhase` → `BracketPhase` → `Done`.

pub(crate) use cqrs_es::event_sink::EventSink;
pub(crate) use cqrs_es::{Aggregate, DomainEvent};
pub(crate) use serde::{Deserialize, Serialize};

pub(crate) use crate::ids::{CourtId, PoolId, TeamId, TournamentId};
pub(crate) use crate::score::MatchFormat;

mod aggregate;
mod command;
mod event;

pub use aggregate::*;
pub use command::*;
pub use event::*;

#[cfg(test)]
mod tests;
