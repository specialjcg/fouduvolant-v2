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

use std::collections::HashMap;

use cqrs_es::event_sink::EventSink;
use cqrs_es::{Aggregate, DomainEvent};
use serde::{Deserialize, Serialize};

use crate::ids::TeamId;

/// Which bracket a node belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BracketKind {
    /// Main elimination draw.
    Main,
    /// Consolation draw (first-round main losers).
    Consolation,
}

/// One match position in a bracket tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BracketNode {
    /// Which bracket.
    pub kind: BracketKind,
    /// Round number, 1-based (1 = first round).
    pub round: u8,
    /// Position within the round, 0-based.
    pub index: u16,
    /// First side (`None` = bye / not yet known).
    pub team_a: Option<TeamId>,
    /// Second side.
    pub team_b: Option<TeamId>,
    /// Winner once decided (by a result, or auto for a bye).
    pub winner: Option<TeamId>,
}

impl BracketNode {
    /// True when both teams are known but the match is not yet decided — i.e. a
    /// real match that should be scheduled / played.
    #[must_use]
    pub fn is_playable(&self) -> bool {
        self.team_a.is_some() && self.team_b.is_some() && self.winner.is_none()
    }
}

/// Smallest power of two `>= n`, with a floor of 2 (a bracket needs ≥2 slots).
#[must_use]
pub fn next_pow2(n: usize) -> usize {
    let mut s = 2;
    while s < n {
        s *= 2;
    }
    s
}

/// Standard single-elimination seed-to-slot order for a `size`-slot bracket
/// (`size` a power of two). Returns the 1-based seed occupying each slot, so
/// that top seeds are kept apart until late rounds.
#[must_use]
pub fn seed_slots(size: usize) -> Vec<usize> {
    let mut v = vec![1, 2];
    while v.len() < size {
        let n = v.len() * 2;
        let mut next = Vec::with_capacity(n);
        for &s in &v {
            next.push(s);
            next.push(n + 1 - s);
        }
        v = next;
    }
    v
}

/// A completed match outcome feeding bracket reconstruction.
pub type Result3 = (TeamId, TeamId, TeamId);

fn pair_key(a: TeamId, b: TeamId) -> (TeamId, TeamId) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn winners_lookup(results: &[Result3]) -> HashMap<(TeamId, TeamId), TeamId> {
    results
        .iter()
        .map(|&(a, b, w)| (pair_key(a, b), w))
        .collect()
}

fn decide(
    a: Option<TeamId>,
    b: Option<TeamId>,
    lk: &HashMap<(TeamId, TeamId), TeamId>,
) -> Option<TeamId> {
    match (a, b) {
        (Some(x), Some(y)) => lk.get(&pair_key(x, y)).copied(),
        (Some(x), None) => Some(x), // bye
        (None, Some(y)) => Some(y), // bye
        (None, None) => None,
    }
}

/// Build every round of one bracket from its slot-ordered entrants.
fn build_rounds(
    kind: BracketKind,
    slot_teams: Vec<Option<TeamId>>,
    lk: &HashMap<(TeamId, TeamId), TeamId>,
) -> Vec<BracketNode> {
    let mut nodes = Vec::new();
    let mut current = slot_teams;
    let mut round = 1u8;
    while current.len() >= 2 {
        let mut winners = Vec::with_capacity(current.len() / 2);
        for j in 0..current.len() / 2 {
            let a = current[2 * j];
            let b = current[2 * j + 1];
            let w = decide(a, b, lk);
            nodes.push(BracketNode {
                kind,
                round,
                index: j as u16,
                team_a: a,
                team_b: b,
                winner: w,
            });
            winners.push(w);
        }
        current = winners;
        round += 1;
    }
    nodes
}

/// Build one bracket's whole tree from its seeded entrants (best first).
fn build_tree(
    kind: BracketKind,
    seeds: &[TeamId],
    lk: &HashMap<(TeamId, TeamId), TeamId>,
) -> Vec<BracketNode> {
    if seeds.len() < 2 {
        return Vec::new();
    }
    let size = next_pow2(seeds.len());
    let slot_teams: Vec<Option<TeamId>> = seed_slots(size)
        .iter()
        .map(|&s| seeds.get(s - 1).copied())
        .collect();
    build_rounds(kind, slot_teams, lk)
}

/// Reconstruct the full draw — main bracket (qualified teams) plus the
/// consolation bracket (non-qualified teams) — from the seeded draws and the
/// completed results so far.
///
/// Faithful to the original fouduvolant: the consolation bracket is its own
/// single-elimination of the teams that did *not* qualify from the pools (best
/// non-qualified first), not the first-round losers of the main draw.
/// Deterministic and idempotent in the results; byes fill any shortfall to the
/// next power of two.
#[must_use]
pub fn build_bracket(
    main_seeds: &[TeamId],
    consolation_seeds: &[TeamId],
    results: &[Result3],
) -> Vec<BracketNode> {
    let lk = winners_lookup(results);
    let mut nodes = build_tree(BracketKind::Main, main_seeds, &lk);
    nodes.extend(build_tree(BracketKind::Consolation, consolation_seeds, &lk));
    nodes
}

// ---- Bracket aggregate: persists only the draw ----

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

#[cfg(test)]
mod tests {
    use super::*;

    fn team(n: u128) -> TeamId {
        TeamId(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn pow2_thresholds() {
        assert_eq!(next_pow2(2), 2);
        assert_eq!(next_pow2(3), 4);
        assert_eq!(next_pow2(5), 8);
        assert_eq!(next_pow2(8), 8);
        assert_eq!(next_pow2(9), 16);
    }

    #[test]
    fn seed_slots_keeps_top_seeds_apart() {
        assert_eq!(seed_slots(2), vec![1, 2]);
        assert_eq!(seed_slots(4), vec![1, 4, 2, 3]);
        assert_eq!(seed_slots(8), vec![1, 8, 4, 5, 2, 7, 3, 6]);
    }

    #[test]
    fn four_seeds_pair_one_vs_four() {
        let t: Vec<TeamId> = (1..=4).map(team).collect();
        let nodes = build_bracket(&t, &[], &[]);
        let r1: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 1)
            .collect();
        assert_eq!(r1.len(), 2);
        // slot order [1,4,2,3] → matches (1v4) and (2v3)
        assert_eq!((r1[0].team_a, r1[0].team_b), (Some(t[0]), Some(t[3])));
        assert_eq!((r1[1].team_a, r1[1].team_b), (Some(t[1]), Some(t[2])));
    }

    #[test]
    fn bye_auto_advances() {
        // 3 seeds in a 4-slot bracket: seed 1 gets a bye (paired with slot for
        // seed 4, which is empty) and auto-advances.
        let t: Vec<TeamId> = (1..=3).map(team).collect();
        let nodes = build_bracket(&t, &[], &[]);
        let r1: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 1)
            .collect();
        // (1 vs bye) → winner seed1 auto ; (2 vs 3) → undecided
        let bye = r1.iter().find(|n| n.team_b.is_none()).unwrap();
        assert_eq!(bye.winner, Some(t[0]));
        let real = r1.iter().find(|n| n.team_b.is_some()).unwrap();
        assert_eq!(real.winner, None);
    }

    #[test]
    fn advancement_fills_final_from_results() {
        let t: Vec<TeamId> = (1..=4).map(team).collect();
        // 1 beats 4, 2 beats 3 → final 1 vs 2
        let results = vec![(t[0], t[3], t[0]), (t[1], t[2], t[1])];
        let nodes = build_bracket(&t, &[], &results);
        let final_node = nodes
            .iter()
            .find(|n| n.kind == BracketKind::Main && n.round == 2)
            .unwrap();
        assert_eq!((final_node.team_a, final_node.team_b), (Some(t[0]), Some(t[1])));
    }

    #[test]
    fn consolation_is_its_own_bracket_of_non_qualified() {
        let main: Vec<TeamId> = (1..=4).map(team).collect();
        let cons: Vec<TeamId> = (5..=8).map(team).collect();
        let nodes = build_bracket(&main, &cons, &[]);
        let cons_r1: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Consolation && n.round == 1)
            .collect();
        assert_eq!(cons_r1.len(), 2, "4 non-qualified → 2 first-round matches");
        // consolation seeded independently: (5 v 8), (6 v 7)
        assert_eq!(
            (cons_r1[0].team_a, cons_r1[0].team_b),
            (Some(cons[0]), Some(cons[3]))
        );
    }

    #[test]
    fn no_consolation_when_everyone_qualifies() {
        let main: Vec<TeamId> = (1..=4).map(team).collect();
        let nodes = build_bracket(&main, &[], &[]);
        assert!(nodes.iter().all(|n| n.kind == BracketKind::Main));
    }

    #[tokio::test]
    async fn cannot_draw_twice() {
        let mut b = Bracket::default();
        b.apply(BracketEvent::Drawn {
            main_seeds: vec![team(1), team(2)],
            consolation_seeds: vec![],
        });
        let sink = EventSink::default();
        let err = b
            .handle(
                BracketCommand::Draw {
                    main_seeds: vec![team(1), team(2)],
                    consolation_seeds: vec![],
                },
                &(),
                &sink,
            )
            .await
            .unwrap_err();
        assert_eq!(err, BracketError::AlreadyDrawn);
    }
}
