//! Domain layer — pure CQRS/ES aggregates for the badminton tournament app.
//!
//! No I/O here: aggregates, commands, events and value objects only.
//! Two aggregates form the write model:
//!   - [`tournament::Tournament`] — setup lifecycle (teams, pools, courts, phases)
//!   - [`matches::Match`] — a single match: scoring and completion
//!
//! Read models (standings, schedule, brackets) live outside this crate as
//! projections built from the event streams.

pub mod generation;
pub mod ids;
pub mod matches;
pub mod projections;
pub mod scheduling;
pub mod score;
pub mod standings;
pub mod tournament;

pub use ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
pub use score::{MatchFormat, SetOutcome, SetScore};
