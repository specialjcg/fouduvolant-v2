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
    /// For a preliminary (round 0) node only: the round-1 match index its winner
    /// advances into, so the UI can draw it facing that match. `None` otherwise.
    pub feeds: Option<u16>,
}

impl BracketNode {
    /// True when both teams are known but the match is not yet decided — i.e. a
    /// real match that should be scheduled / played.
    #[must_use]
    pub fn is_playable(&self) -> bool {
        self.team_a.is_some() && self.team_b.is_some() && self.winner.is_none()
    }
}

/// Main-draw size for `n` entrants — the largest power of two `<= n` (floor),
/// with a floor of 2. Non-powers play a preliminary round (barrages /
/// pré-tours) down into this size. Faithful to the original's
/// `compute_final_bracket_size`.
#[must_use]
pub fn bracket_size(n: usize) -> usize {
    if n.is_power_of_two() {
        n
    } else {
        (n.next_power_of_two() / 2).max(2)
    }
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

/// Reorder a seeded participant list to avoid first-round same-pool matchups,
/// greedily — a best-effort port of the original's `reseed_avoid_first_round_rematch`.
///
/// First-round pairs follow the `(i, n-1-i)` convention (same set of pairs as
/// [`seed_slots`]). When a pair shares a pool, the bottom-half team is swapped
/// with another bottom-half team that resolves both pairs. Residual conflicts
/// (unavoidable) are left as-is. `pool_of` maps a team to its pool number; 0
/// means "no pool" and is never treated as a conflict.
pub fn reseed_pool_separation(participants: &mut [TeamId], pool_of: &HashMap<TeamId, usize>) {
    let n = participants.len();
    if n < 4 {
        return;
    }
    let pairs = n / 2;
    let pool = |t: TeamId| pool_of.get(&t).copied().unwrap_or(0);

    for i in 0..pairs {
        let (a, b) = (i, n - 1 - i);
        let pa = pool(participants[a]);
        if pa == 0 || pool(participants[b]) != pa {
            continue;
        }
        for j in 0..pairs {
            if j == i {
                continue;
            }
            let (aj, bj) = (j, n - 1 - j);
            let new_opp_a = pool(participants[bj]);
            let moved_b = pool(participants[b]);
            let pool_aj = pool(participants[aj]);
            if new_opp_a != pa && moved_b != pool_aj {
                participants.swap(b, bj);
                break;
            }
        }
    }
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

/// Decide a node's winner from the results. A side that is `None` is a
/// not-yet-known entrant (a pending preliminary-round winner), so the node stays
/// undecided — there are no byes in the floor-sized + play-in model.
fn decide(
    a: Option<TeamId>,
    b: Option<TeamId>,
    lk: &HashMap<(TeamId, TeamId), TeamId>,
) -> Option<TeamId> {
    match (a, b) {
        (Some(x), Some(y)) => lk.get(&pair_key(x, y)).copied(),
        _ => None,
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
                feeds: None,
            });
            winners.push(w);
        }
        current = winners;
        round += 1;
    }
    nodes
}

/// Build one bracket from its seeded entrants (best first): a preliminary round
/// (round 0) when the field exceeds the bracket size, then the main rounds.
///
/// Floor-sized bracket `S`; `extra = n - S` preliminary matches pair the weakest
/// `2*extra` seeds best-vs-worst (`seeds[direct+i]` vs `seeds[n-1-i]`); their
/// winners take the remaining seed slots. The top `direct = S - extra` seeds go
/// straight in. Faithful to the original's barrage / pré-tour scheme.
fn build_tree(
    kind: BracketKind,
    seeds: &[TeamId],
    lk: &HashMap<(TeamId, TeamId), TeamId>,
) -> Vec<BracketNode> {
    let n = seeds.len();
    if n < 2 {
        return Vec::new();
    }
    let size = bracket_size(n);
    let extra = n - size;
    let direct = size - extra;

    let mut nodes = Vec::new();
    // Direct entrants keep their seed order; preliminary winners fill the rest.
    let mut effective: Vec<Option<TeamId>> = seeds[..direct].iter().copied().map(Some).collect();
    // slots[p] = 1-based seed number placed at bracket slot p; a barrage winner
    // takes effective index `direct + i` (seed number `direct + i + 1`), so the
    // round-1 match it feeds is the slot holding that seed, halved.
    let slots = seed_slots(size);
    for i in 0..extra {
        let a = seeds[direct + i];
        let b = seeds[n - 1 - i];
        let winner = lk.get(&pair_key(a, b)).copied();
        let feeds = slots
            .iter()
            .position(|&s| s == direct + i + 1)
            .map(|p| (p / 2) as u16);
        nodes.push(BracketNode {
            kind,
            round: 0,
            index: i as u16,
            team_a: Some(a),
            team_b: Some(b),
            winner,
            feeds,
        });
        effective.push(winner);
    }

    let slot_teams: Vec<Option<TeamId>> = seed_slots(size)
        .iter()
        .map(|&s| effective[s - 1])
        .collect();
    nodes.extend(build_rounds(kind, slot_teams, lk));

    // Third-place match (petite finale) for brackets of 8+: the two semifinal
    // losers. Round `THIRD_PLACE_ROUND` sorts it after the final.
    if size >= 8 {
        let final_round = size.trailing_zeros() as u8; // log2(size)
        let third = {
            let mut semis: Vec<&BracketNode> = nodes
                .iter()
                .filter(|n| n.round == final_round - 1)
                .collect();
            semis.sort_by_key(|n| n.index);
            let loser = |n: &BracketNode| match (n.team_a, n.team_b, n.winner) {
                (Some(a), Some(b), Some(w)) => Some(if w == a { b } else { a }),
                _ => None,
            };
            (semis.len() == 2).then(|| (loser(semis[0]), loser(semis[1])))
        };
        if let Some((la, lb)) = third {
            nodes.push(BracketNode {
                kind,
                round: THIRD_PLACE_ROUND,
                index: 0,
                team_a: la,
                team_b: lb,
                winner: decide(la, lb, lk),
                feeds: None,
            });
        }
    }

    nodes
}

/// Sentinel round number for the third-place match (sorts after the final).
pub const THIRD_PLACE_ROUND: u8 = u8::MAX;

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
    fn preliminary_feeds_point_to_the_match_holding_their_winner() {
        // 6 seeds → size 4, extra 2, direct 2: two barrages, their winners fill
        // the remaining round-1 slots. Each barrage's `feeds` must name the
        // round-1 match its winner actually plays in.
        let t: Vec<TeamId> = (1..=6).map(team).collect();
        // Decide each barrage: seeds[2]>seeds[5], seeds[3]>seeds[4].
        let results: Vec<Result3> = vec![(t[2], t[5], t[2]), (t[3], t[4], t[3])];
        let nodes = build_bracket(&t, &[], &results);

        let prelims: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 0)
            .collect();
        assert_eq!(prelims.len(), 2, "two preliminary matches");

        for p in prelims {
            let feeds = p.feeds.expect("a preliminary node carries a feeds index");
            let target = nodes
                .iter()
                .find(|n| n.kind == BracketKind::Main && n.round == 1 && n.index == feeds)
                .expect("feeds points at an existing round-1 match");
            let w = p.winner.expect("barrage decided");
            assert!(
                target.team_a == Some(w) || target.team_b == Some(w),
                "barrage winner must appear in the round-1 match it feeds"
            );
        }
    }

    #[test]
    fn bracket_size_floors_to_power_of_two() {
        assert_eq!(bracket_size(2), 2);
        assert_eq!(bracket_size(3), 2);
        assert_eq!(bracket_size(4), 4);
        assert_eq!(bracket_size(5), 4);
        assert_eq!(bracket_size(8), 8);
        assert_eq!(bracket_size(9), 8);
        assert_eq!(bracket_size(12), 8);
        assert_eq!(bracket_size(16), 16);
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
    fn three_seeds_use_a_preliminary_round() {
        // 3 seeds → floor size 2, extra 1, direct 1. Seeds 2 and 3 play a
        // preliminary; its winner meets seed 1 in the final. No byes.
        let t: Vec<TeamId> = (1..=3).map(team).collect();
        let nodes = build_bracket(&t, &[], &[]);
        let prelim: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 0)
            .collect();
        assert_eq!(prelim.len(), 1);
        assert_eq!((prelim[0].team_a, prelim[0].team_b), (Some(t[1]), Some(t[2])));

        // Final exists but its play-in side is unknown (not a bye → undecided).
        let final_node = nodes
            .iter()
            .find(|n| n.kind == BracketKind::Main && n.round == 1)
            .unwrap();
        assert!(final_node.team_a == Some(t[0]) || final_node.team_b == Some(t[0]));
        assert_eq!(final_node.winner, None);

        // Play the preliminary: seed 2 wins → it fills the final slot.
        let nodes2 = build_bracket(&t, &[], &[(t[1], t[2], t[1])]);
        let final2 = nodes2
            .iter()
            .find(|n| n.kind == BracketKind::Main && n.round == 1)
            .unwrap();
        let teams = [final2.team_a, final2.team_b];
        assert!(teams.contains(&Some(t[0])) && teams.contains(&Some(t[1])));
    }

    #[test]
    fn five_seeds_one_barrage() {
        // 5 → size 4, extra 1, direct 3. Seeds 4 and 5 play the barrage.
        let t: Vec<TeamId> = (1..=5).map(team).collect();
        let nodes = build_bracket(&t, &[], &[]);
        let prelim: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 0)
            .collect();
        assert_eq!(prelim.len(), 1);
        assert_eq!((prelim[0].team_a, prelim[0].team_b), (Some(t[3]), Some(t[4])));
        let r1 = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 1)
            .count();
        assert_eq!(r1, 2, "size-4 bracket has two first-round matches");
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
    fn third_place_only_for_eight_plus() {
        let four: Vec<TeamId> = (1..=4).map(team).collect();
        assert_eq!(
            build_bracket(&four, &[], &[])
                .iter()
                .filter(|n| n.round == THIRD_PLACE_ROUND)
                .count(),
            0
        );
        let eight: Vec<TeamId> = (1..=8).map(team).collect();
        assert_eq!(
            build_bracket(&eight, &[], &[])
                .iter()
                .filter(|n| n.round == THIRD_PLACE_ROUND)
                .count(),
            1
        );
    }

    #[test]
    fn third_place_holds_semifinal_losers() {
        let t: Vec<TeamId> = (1..=8).map(team).collect();
        // Lower seed index always wins.
        let results = vec![
            (t[0], t[7], t[0]),
            (t[3], t[4], t[3]),
            (t[1], t[6], t[1]),
            (t[2], t[5], t[2]),
            (t[0], t[3], t[0]),
            (t[1], t[2], t[1]),
        ];
        let nodes = build_bracket(&t, &[], &results);
        let third = nodes
            .iter()
            .find(|n| n.round == THIRD_PLACE_ROUND)
            .unwrap();
        let teams = [third.team_a, third.team_b];
        assert!(teams.contains(&Some(t[3])) && teams.contains(&Some(t[2])));
    }

    #[test]
    fn no_consolation_when_everyone_qualifies() {
        let main: Vec<TeamId> = (1..=4).map(team).collect();
        let nodes = build_bracket(&main, &[], &[]);
        assert!(nodes.iter().all(|n| n.kind == BracketKind::Main));
    }

    #[test]
    fn reseed_breaks_same_pool_first_round_pairs() {
        use std::collections::HashMap;
        // Seeds [a,b,c,d]; (i,n-1-i) pairs (a,d) and (b,c). a&d in pool 1, b&c
        // in pool 2 → both pairs same-pool. Reseed must separate them.
        let (a, b, c, d) = (team(1), team(2), team(3), team(4));
        let mut seeds = vec![a, b, c, d];
        let pools: HashMap<TeamId, usize> =
            [(a, 1), (d, 1), (b, 2), (c, 2)].into_iter().collect();
        reseed_pool_separation(&mut seeds, &pools);
        let pool = |t: TeamId| pools[&t];
        let n = seeds.len();
        for i in 0..n / 2 {
            assert_ne!(
                pool(seeds[i]),
                pool(seeds[n - 1 - i]),
                "pair {i} still same-pool"
            );
        }
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
