use super::*;


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

    // Third-place match (petite finale) as soon as there are real semifinals
    // (4+ entrants): the two semifinal losers. A 2-entrant draw is just a final,
    // so no petite finale. Round `THIRD_PLACE_ROUND` sorts it after the final.
    if size >= 4 {
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
