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

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};

/// How many previews (matches after the next one) each court plan exposes.
const PREVIEW_DEPTH: usize = 2;

/// Scheduling-relevant lifecycle of a match, decoupled from the `Match`
/// aggregate so the planner can run over any snapshot source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedStatus {
    /// Not yet played; eligible to be scheduled.
    Pending,
    /// Currently being played on a court.
    Playing,
    /// Finished.
    Done,
}

/// A read-only view of one match, the planner's unit of input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchView {
    /// Match identity.
    pub id: MatchId,
    /// Tournament this match belongs to (used to scope dispatch).
    pub tournament: TournamentId,
    /// Stable creation order; drives all deterministic tiebreaks.
    pub seq: u32,
    /// Owning pool, or `None` for bracket/finals matches.
    pub pool: Option<PoolId>,
    /// First side.
    pub team_a: TeamId,
    /// Second side.
    pub team_b: TeamId,
    /// Scheduling status.
    pub status: SchedStatus,
    /// Court the match is playing on / was played on, if any.
    pub court: Option<CourtId>,
    /// User override: pin this match to a specific court (the ▶ action).
    pub manual_court: Option<CourtId>,
    /// Completion order, used to find the most recent finish per court.
    pub done_order: Option<u32>,
    /// Winner once the match is decided.
    pub winner: Option<TeamId>,
    /// Points scored by side A across all recorded sets.
    pub points_a: u16,
    /// Points scored by side B across all recorded sets.
    pub points_b: u16,
    /// Each recorded set as `(a, b)` in play order — lets the UI show
    /// "21-15 21-10" instead of a summed 42 for best-of-3 matches.
    pub sets: Vec<(u16, u16)>,
    /// True when the match was ended by forfeit / retirement.
    pub conceded: bool,
}

impl MatchView {
    fn teams(&self) -> [TeamId; 2] {
        [self.team_a, self.team_b]
    }
}

/// A single proposed match for a court.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// The proposed match.
    pub match_id: MatchId,
    /// True when the proposal forces a team to play back-to-back because no
    /// rested alternative exists (the UI flags "needs rest").
    pub needs_rest: bool,
}

/// The plan for one court: what is playing now, what is next, and a short
/// look-ahead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourtPlan {
    /// Court identity.
    pub court: CourtId,
    /// Match currently being played on this court, if any.
    pub current: Option<MatchId>,
    /// Proposed next match, if one fits.
    pub next: Option<Suggestion>,
    /// Up to [`PREVIEW_DEPTH`] further matches after `next`.
    pub previews: Vec<Suggestion>,
}

/// Default pool→court assignment by greedy bin-packing.
///
/// Pools are discovered in first-appearance order (by `seq`), sorted by match
/// count descending (stable, so ties keep first-appearance order), then each is
/// placed on the court with the least load so far. With pools ≤ courts every
/// pool gets its own court; with more pools, the busiest pools spread first and
/// later pools share the lightest courts.
#[must_use]
pub fn assign_pools_to_courts(
    matches: &[MatchView],
    courts: &[CourtId],
) -> HashMap<PoolId, CourtId> {
    let mut map = HashMap::new();
    if courts.is_empty() {
        return map;
    }

    let mut order: Vec<MatchView> = matches.to_vec();
    order.sort_by_key(|m| m.seq);

    let mut pool_ids: Vec<PoolId> = Vec::new();
    let mut count: HashMap<PoolId, usize> = HashMap::new();
    for m in &order {
        if let Some(p) = m.pool {
            if !count.contains_key(&p) {
                pool_ids.push(p);
            }
            *count.entry(p).or_default() += 1;
        }
    }

    // Stable sort by count descending keeps first-appearance order on ties.
    pool_ids.sort_by(|a, b| count[b].cmp(&count[a]));

    let mut load = vec![0usize; courts.len()];
    for p in pool_ids {
        let idx = load
            .iter()
            .enumerate()
            .min_by_key(|(_, l)| **l)
            .map(|(i, _)| i)
            .unwrap_or(0);
        map.insert(p, courts[idx]);
        load[idx] += count[&p];
    }
    map
}

/// Phase ordering for dispatch priority: pool matches first, bracket last.
fn phase(m: &MatchView) -> u8 {
    match m.pool {
        Some(_) => 0,
        None => 1,
    }
}

/// Completion fraction of a pool, counting matches already assigned this round.
fn pool_fraction(
    pool: Option<PoolId>,
    total: &HashMap<PoolId, usize>,
    done: &HashMap<PoolId, usize>,
    assigned: &HashMap<PoolId, usize>,
) -> f64 {
    let Some(p) = pool else {
        return 1.0; // bracket matches carry no pool weave weight
    };
    let t = (*total.get(&p).unwrap_or(&1)).max(1);
    let d = done.get(&p).copied().unwrap_or(0) + assigned.get(&p).copied().unwrap_or(0);
    d as f64 / t as f64
}

/// Outcome of picking a match for one court slot.
struct Pick {
    match_id: MatchId,
    pool: Option<PoolId>,
    teams: [TeamId; 2],
    needs_rest: bool,
}

/// Select the best pending match for a court slot, applying the legacy cascade.
///
/// `recent` is the soft anti-back-to-back set (teams that just played); `busy`
/// is the hard set (teams playing right now — never selectable). The cascade
/// prefers, in order: a manually pinned match, a preferred-pool rested match,
/// a preferred-pool match (relaxing rest), then — only without an explicit map —
/// any rested match, then any match.
#[allow(clippy::too_many_arguments)]
fn pick_for_court(
    pending: &[&MatchView],
    taken: &HashSet<MatchId>,
    court: CourtId,
    preferred: &HashSet<PoolId>,
    recent: &HashSet<TeamId>,
    busy: &HashSet<TeamId>,
    has_explicit_map: bool,
    total: &HashMap<PoolId, usize>,
    done: &HashMap<PoolId, usize>,
    assigned: &HashMap<PoolId, usize>,
) -> Option<Pick> {
    let available = |m: &&MatchView| -> bool {
        !taken.contains(&m.id)
            && m.manual_court.is_none_or(|c| c == court)
            && m.teams().iter().all(|t| !busy.contains(t))
    };
    let rested = |m: &&MatchView| m.teams().iter().all(|t| !recent.contains(t));
    let preferred_pool = |m: &&MatchView| m.pool.is_some_and(|p| preferred.contains(&p));

    // Order-sensitive helper: among candidates, pick the least-complete pool,
    // breaking ties by creation order.
    let best = |cands: Vec<&MatchView>| -> Option<MatchId> {
        cands
            .into_iter()
            .min_by(|a, b| {
                let fa = pool_fraction(a.pool, total, done, assigned);
                let fb = pool_fraction(b.pool, total, done, assigned);
                fa.total_cmp(&fb).then(a.seq.cmp(&b.seq))
            })
            .map(|m| m.id)
    };

    let to_pick = |id: MatchId, needs_rest: bool| -> Pick {
        let m = pending.iter().find(|m| m.id == id).expect("id from pending");
        Pick {
            match_id: id,
            pool: m.pool,
            teams: m.teams(),
            needs_rest,
        }
    };

    // 1. Manual override pinned to this court (prefer rested, else allow).
    let manual: Vec<&MatchView> = pending
        .iter()
        .copied()
        .filter(|m| m.manual_court == Some(court) && available(m))
        .collect();
    if !manual.is_empty() {
        if let Some(id) = best(manual.iter().copied().filter(rested).collect()) {
            return Some(to_pick(id, false));
        }
        // Manual choice wins even back-to-back: the user asked for it.
        if let Some(id) = best(manual) {
            return Some(to_pick(id, true));
        }
    }

    // 2. Preferred pool, rested.
    if let Some(id) = best(
        pending
            .iter()
            .copied()
            .filter(|m| available(m) && preferred_pool(m) && rested(m))
            .collect(),
    ) {
        return Some(to_pick(id, false));
    }
    // 3. Preferred pool, relax rest.
    if let Some(id) = best(
        pending
            .iter()
            .copied()
            .filter(|m| available(m) && preferred_pool(m))
            .collect(),
    ) {
        return Some(to_pick(id, true));
    }

    // Without an explicit map, courts may serve any pool (idle-to-rest off).
    if !has_explicit_map {
        // 4. Any rested match.
        if let Some(id) = best(
            pending
                .iter()
                .copied()
                .filter(|m| available(m) && rested(m))
                .collect(),
        ) {
            return Some(to_pick(id, false));
        }
        // 5. Any match.
        if let Some(id) = best(pending.iter().copied().filter(available).collect()) {
            return Some(to_pick(id, true));
        }
    }

    None
}

/// Full forecast: the ordered list of every match each court will host.
///
/// A match goes to the court it played/plays on if known, otherwise to the court
/// of its pool (explicit `pool_court_map`, else the greedy default). Within a
/// court, finished matches come first (by completion order), then the one in
/// progress, then the pending ones in creation order — giving the "prévisionnel".
#[must_use]
pub fn forecast(
    matches: &[MatchView],
    courts: &[CourtId],
    pool_court_map: &HashMap<PoolId, CourtId>,
) -> Vec<(CourtId, Vec<MatchId>)> {
    let owned;
    let map: &HashMap<PoolId, CourtId> = if pool_court_map.is_empty() {
        owned = assign_pools_to_courts(matches, courts);
        &owned
    } else {
        pool_court_map
    };

    let court_of = |m: &MatchView| -> Option<CourtId> {
        m.court.or_else(|| m.pool.and_then(|p| map.get(&p).copied()))
    };
    let rank = |m: &MatchView| -> (u8, u32) {
        match m.status {
            SchedStatus::Done => (0, m.done_order.unwrap_or(0)),
            SchedStatus::Playing => (1, m.seq),
            SchedStatus::Pending => (2, m.seq),
        }
    };

    courts
        .iter()
        .map(|&court| {
            let mut ms: Vec<&MatchView> =
                matches.iter().filter(|m| court_of(m) == Some(court)).collect();
            ms.sort_by_key(|m| rank(m));
            (court, ms.into_iter().map(|m| m.id).collect())
        })
        .collect()
}

/// Compute the dispatch plan for every court.
///
/// `pool_court_map` may be empty, in which case a greedy default is computed and
/// the idle-to-rest rule is disabled (any court may serve any pool). When it is
/// non-empty, a court only proposes matches from its assigned pools — unless a
/// match is manually pinned to it, or it already has history there.
#[must_use]
pub fn plan(
    matches: &[MatchView],
    courts: &[CourtId],
    pool_court_map: &HashMap<PoolId, CourtId>,
) -> Vec<CourtPlan> {
    let has_explicit_map = !pool_court_map.is_empty();
    let owned_map;
    let map: &HashMap<PoolId, CourtId> = if has_explicit_map {
        pool_court_map
    } else {
        owned_map = assign_pools_to_courts(matches, courts);
        &owned_map
    };

    // Pool weave bookkeeping.
    let mut total: HashMap<PoolId, usize> = HashMap::new();
    let mut done: HashMap<PoolId, usize> = HashMap::new();
    for m in matches {
        if let Some(p) = m.pool {
            *total.entry(p).or_default() += 1;
            if m.status != SchedStatus::Pending {
                *done.entry(p).or_default() += 1;
            }
        }
    }
    let mut assigned: HashMap<PoolId, usize> = HashMap::new();

    // Per-court current match and most-recent finish.
    let mut current: HashMap<CourtId, &MatchView> = HashMap::new();
    let mut last_done: HashMap<CourtId, &MatchView> = HashMap::new();
    for m in matches {
        let Some(c) = m.court else { continue };
        match m.status {
            SchedStatus::Playing => {
                current.insert(c, m);
            }
            SchedStatus::Done => {
                let better = last_done
                    .get(&c)
                    .is_none_or(|p| m.done_order.unwrap_or(0) >= p.done_order.unwrap_or(0));
                if better {
                    last_done.insert(c, m);
                }
            }
            SchedStatus::Pending => {}
        }
    }

    // Hard-busy teams (playing now) and the anti-btb "recent" set: per court the
    // teams of the current match if playing, else of the last finished match.
    let mut busy: HashSet<TeamId> = HashSet::new();
    let mut recent: HashSet<TeamId> = HashSet::new();
    for c in courts {
        if let Some(m) = current.get(c) {
            busy.extend(m.teams());
            recent.extend(m.teams());
        } else if let Some(m) = last_done.get(c) {
            recent.extend(m.teams());
        }
    }

    // Pending matches, deterministically ordered.
    let mut pending: Vec<&MatchView> = matches
        .iter()
        .filter(|m| m.status == SchedStatus::Pending)
        .collect();
    pending.sort_by(|a, b| {
        phase(a)
            .cmp(&phase(b))
            .then(a.seq.cmp(&b.seq))
    });

    let mut taken: HashSet<MatchId> = HashSet::new();

    // Per-court static facts.
    struct Slot {
        court: CourtId,
        current: Option<MatchId>,
        preferred: HashSet<PoolId>,
        skip: bool,
    }
    let slots: Vec<Slot> = courts
        .iter()
        .map(|&court| {
            let preferred: HashSet<PoolId> = map
                .iter()
                .filter(|(_, &c)| c == court)
                .map(|(&p, _)| p)
                .collect();
            // Idle-to-rest: with an explicit map, a court with no assigned pool,
            // no pinned match and no history here makes no suggestion.
            let has_pin = pending.iter().any(|m| m.manual_court == Some(court));
            let has_history =
                last_done.contains_key(&court) || current.contains_key(&court);
            let skip = has_explicit_map && preferred.is_empty() && !has_pin && !has_history;
            Slot {
                court,
                current: current.get(&court).map(|m| m.id),
                preferred,
                skip,
            }
        })
        .collect();

    let mut pick_slot = |court, preferred: &HashSet<PoolId>, blocked: &HashSet<TeamId>| {
        pick_for_court(
            &pending, &taken, court, preferred, blocked, &busy, has_explicit_map, &total,
            &done, &assigned,
        )
        .inspect(|p| {
            taken.insert(p.match_id);
            if let Some(pl) = p.pool {
                *assigned.entry(pl).or_default() += 1;
            }
        })
    };

    // Phase 1 — allocate every court's `next` first, so a court's preview
    // look-ahead can never starve another court's real assignment.
    let mut nexts: Vec<Option<Suggestion>> = Vec::with_capacity(slots.len());
    let mut blocks: Vec<HashSet<TeamId>> = Vec::with_capacity(slots.len());
    for slot in &slots {
        let mut blocked = recent.clone();
        let next = if slot.skip {
            None
        } else {
            pick_slot(slot.court, &slot.preferred, &blocked).map(|p| {
                blocked.extend(p.teams);
                Suggestion {
                    match_id: p.match_id,
                    needs_rest: p.needs_rest,
                }
            })
        };
        nexts.push(next);
        blocks.push(blocked);
    }

    // Phase 2 — fill each court's previews (look-ahead) from what remains.
    let mut plans = Vec::with_capacity(slots.len());
    for (i, slot) in slots.iter().enumerate() {
        let mut previews = Vec::new();
        if !slot.skip {
            let mut blocked = blocks[i].clone();
            for _ in 0..PREVIEW_DEPTH {
                let Some(p) = pick_slot(slot.court, &slot.preferred, &blocked) else {
                    break;
                };
                blocked.extend(p.teams);
                previews.push(Suggestion {
                    match_id: p.match_id,
                    needs_rest: p.needs_rest,
                });
            }
        }
        plans.push(CourtPlan {
            court: slot.court,
            current: slot.current,
            next: nexts[i].clone(),
            previews,
        });
    }

    plans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn court(n: u128) -> CourtId {
        CourtId(uuid::Uuid::from_u128(n))
    }
    fn pool(n: u128) -> PoolId {
        PoolId(uuid::Uuid::from_u128(n))
    }
    fn team(n: u128) -> TeamId {
        TeamId(uuid::Uuid::from_u128(n))
    }

    fn tourney() -> TournamentId {
        TournamentId(uuid::Uuid::from_u128(999))
    }

    fn pending_match(seq: u32, p: PoolId, a: TeamId, b: TeamId) -> MatchView {
        MatchView {
            id: MatchId::new(),
            tournament: tourney(),
            seq,
            pool: Some(p),
            team_a: a,
            team_b: b,
            status: SchedStatus::Pending,
            court: None,
            manual_court: None,
            done_order: None,
            winner: None,
            points_a: 0,
            points_b: 0,
            sets: Vec::new(),
            conceded: false,
        }
    }

    #[test]
    fn one_pool_per_court_when_enough_courts() {
        let (p1, p2) = (pool(1), pool(2));
        let matches = vec![
            pending_match(0, p1, team(10), team(11)),
            pending_match(1, p1, team(12), team(13)),
            pending_match(2, p2, team(20), team(21)),
            pending_match(3, p2, team(22), team(23)),
        ];
        let courts = vec![court(1), court(2)];
        let map = assign_pools_to_courts(&matches, &courts);
        assert_eq!(map.len(), 2);
        assert_ne!(map[&p1], map[&p2], "distinct pools get distinct courts");
    }

    #[test]
    fn overflow_pools_share_lightest_courts() {
        // 3 pools, 2 courts. Biggest pool alone, the two smaller share.
        let (p1, p2, p3) = (pool(1), pool(2), pool(3));
        let mut matches = vec![
            pending_match(0, p1, team(10), team(11)),
            pending_match(1, p1, team(12), team(13)),
            pending_match(2, p1, team(14), team(15)),
        ];
        matches.push(pending_match(3, p2, team(20), team(21)));
        matches.push(pending_match(4, p3, team(30), team(31)));
        let courts = vec![court(1), court(2)];
        let map = assign_pools_to_courts(&matches, &courts);
        // p1 (3 matches) lands alone; p2 and p3 land on the other court.
        assert_eq!(map[&p2], map[&p3]);
        assert_ne!(map[&p1], map[&p2]);
    }

    #[test]
    fn anti_btb_prefers_rested_team() {
        // Court just finished a match with teams 10/11. The next suggestion on a
        // free court must avoid both, even though their pool is least complete.
        let p = pool(1);
        let courts = vec![court(1)];
        let just_done = MatchView {
            id: MatchId::new(),
            tournament: tourney(),
            seq: 0,
            pool: Some(p),
            team_a: team(10),
            team_b: team(11),
            status: SchedStatus::Done,
            court: Some(court(1)),
            manual_court: None,
            done_order: Some(1),
            winner: Some(team(10)),
            points_a: 21,
            points_b: 11,
            sets: vec![(21, 11)],
            conceded: false,
        };
        let btb = pending_match(1, p, team(10), team(20)); // reuses team 10
        let fresh = pending_match(2, p, team(30), team(31));
        let matches = vec![just_done, btb.clone(), fresh.clone()];
        let plans = plan(&matches, &courts, &HashMap::new());
        let next = plans[0].next.as_ref().unwrap();
        assert_eq!(next.match_id, fresh.id);
        assert!(!next.needs_rest);
    }

    #[test]
    fn anti_btb_relaxes_when_unavoidable() {
        // Only one pending match and it reuses a team that just played.
        let p = pool(1);
        let courts = vec![court(1)];
        let just_done = MatchView {
            id: MatchId::new(),
            tournament: tourney(),
            seq: 0,
            pool: Some(p),
            team_a: team(10),
            team_b: team(11),
            status: SchedStatus::Done,
            court: Some(court(1)),
            manual_court: None,
            done_order: Some(1),
            winner: Some(team(10)),
            points_a: 21,
            points_b: 11,
            sets: vec![(21, 11)],
            conceded: false,
        };
        let only = pending_match(1, p, team(10), team(20));
        let matches = vec![just_done, only.clone()];
        let plans = plan(&matches, &courts, &HashMap::new());
        let next = plans[0].next.as_ref().unwrap();
        assert_eq!(next.match_id, only.id);
        assert!(next.needs_rest, "unavoidable btb is flagged");
    }

    #[test]
    fn weave_spreads_small_pool_across_time() {
        // Court hosts a big pool (p1: 4 matches) and a small one (p2: 2). Across
        // the next+previews (3 slots) the small pool must not be exhausted first;
        // a balanced weave interleaves them.
        let (p1, p2) = (pool(1), pool(2));
        let courts = vec![court(1)];
        let matches = vec![
            pending_match(0, p1, team(10), team(11)),
            pending_match(1, p1, team(12), team(13)),
            pending_match(2, p1, team(14), team(15)),
            pending_match(3, p1, team(16), team(17)),
            pending_match(4, p2, team(20), team(21)),
            pending_match(5, p2, team(22), team(23)),
        ];
        let mut map = HashMap::new();
        map.insert(p1, court(1));
        map.insert(p2, court(1));
        let plans = plan(&matches, &courts, &map);
        let chosen: Vec<Option<PoolId>> = std::iter::once(plans[0].next.as_ref().unwrap().match_id)
            .chain(plans[0].previews.iter().map(|s| s.match_id))
            .map(|id| matches.iter().find(|m| m.id == id).unwrap().pool)
            .collect();
        // Least-complete-pool-first picks p2 at least once within the first 3.
        assert!(chosen.contains(&Some(p2)), "small pool woven in early: {chosen:?}");
    }

    #[test]
    fn manual_move_targets_its_court_without_stealing() {
        // Explicit map: pool 1 → court 1. A pool-1 match is manually pinned to
        // court 2 (which owns no pool). Court 2 must take it; court 1 must not.
        let p1 = pool(1);
        let courts = vec![court(1), court(2)];
        let mut map = HashMap::new();
        map.insert(p1, court(1));

        let normal = pending_match(0, p1, team(10), team(11));
        let mut pinned = pending_match(1, p1, team(12), team(13));
        pinned.manual_court = Some(court(2));
        let matches = vec![normal.clone(), pinned.clone()];

        let plans = plan(&matches, &courts, &map);
        let c1 = plans.iter().find(|p| p.court == court(1)).unwrap();
        let c2 = plans.iter().find(|p| p.court == court(2)).unwrap();
        assert_eq!(c1.next.as_ref().unwrap().match_id, normal.id);
        assert_eq!(c2.next.as_ref().unwrap().match_id, pinned.id);
    }

    #[test]
    fn idle_court_with_explicit_map_makes_no_suggestion() {
        // Court 2 owns no pool, has no pin and no history → no suggestion.
        let p1 = pool(1);
        let courts = vec![court(1), court(2)];
        let mut map = HashMap::new();
        map.insert(p1, court(1));
        let matches = vec![pending_match(0, p1, team(10), team(11))];
        let plans = plan(&matches, &courts, &map);
        let c2 = plans.iter().find(|p| p.court == court(2)).unwrap();
        assert!(c2.next.is_none());
    }
}
