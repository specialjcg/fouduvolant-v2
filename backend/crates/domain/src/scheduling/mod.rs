//! Court scheduling — pure planning logic, ported from the original fouduvolant
//! `court_dispatcher`. No I/O, no aggregate coupling: it takes a snapshot view of
//! matches and produces per-court suggestions.
//!
//! Two responsibilities:
//!   - [`assign_pools_to_courts`]: greedy default mapping of pools to courts.
//!   - [`plan`]: live dispatch — for each court, what plays next (+ a short
//!     preview), honouring manual overrides, anti-back-to-back, pool weaving
//!     and the idle-to-rest rule.
//!
//! Hybrid dispatch: by default one pool maps to one court; overflow pools and
//! reassignments are driven by [`MatchView::manual_court`] (the user's ▶ click).
//!
//! Determinism: every ordering derives from [`MatchView::seq`] (stable creation
//! order). No `HashMap` is ever iterated for an order-sensitive decision.


pub(crate) use std::collections::{HashMap, HashSet};

pub(crate) use serde::{Deserialize, Serialize};

pub(crate) use crate::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};

/// How many previews (matches after the next one) each court plan exposes.
pub(crate) const PREVIEW_DEPTH: usize = 2;

mod dispatch;
mod types;

pub use dispatch::*;
pub use types::*;

#[cfg(test)]
mod tests;
