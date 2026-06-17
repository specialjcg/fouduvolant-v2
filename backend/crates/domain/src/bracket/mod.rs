//! Elimination bracket — main draw plus a consolation bracket for first-round
//! losers.
//!
//! Design: the only persisted state is the **draw** (the seeded team order),
//! held by the tiny [`Bracket`] aggregate. The entire tree — main and
//! consolation, every round, byes and advancement — is then *reconstructed
//! purely* from the seeds and the completed bracket match results, keyed by
//! unordered team pair. No per-node aggregate, no advancement events: results
//! drive everything. This sidesteps the "stored tree" pitfalls of the original
//! (id schemes, seeding paths) by deriving structure deterministically.

pub(crate) use std::collections::HashMap;

pub(crate) use cqrs_es::event_sink::EventSink;
pub(crate) use cqrs_es::{Aggregate, DomainEvent};
pub(crate) use serde::{Deserialize, Serialize};

pub(crate) use crate::ids::TeamId;

mod aggregate;
mod tree;

pub use aggregate::*;
pub use tree::*;

#[cfg(test)]
mod tests;
