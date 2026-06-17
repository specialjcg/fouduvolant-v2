use super::*;

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

