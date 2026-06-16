//! Match generation for the pool stage.
//!
//! Pure combinatorics, no I/O: turning a pool's team list into the set of
//! matches to schedule. Single round-robin — every team plays every other once.

use crate::ids::TeamId;

/// All pairs of a pool's teams (single round-robin) in **circle-method** order:
/// the schedule is split into rounds where every team plays at most once, so a
/// team never plays twice in a row and gets rest between its matches.
///
/// Yields `n * (n - 1) / 2` pairs, each exactly once. An odd team count uses a
/// virtual "bye" (the team facing it simply has no match that round).
#[must_use]
pub fn round_robin_pairs(teams: &[TeamId]) -> Vec<(TeamId, TeamId)> {
    let n = teams.len();
    if n < 2 {
        return Vec::new();
    }
    // Working slots of team indices; append a bye (None) for an odd count.
    let mut slots: Vec<Option<usize>> = (0..n).map(Some).collect();
    if n % 2 == 1 {
        slots.push(None);
    }
    let m = slots.len(); // even
    let rounds = m - 1;
    let half = m / 2;

    let mut pairs = Vec::with_capacity(n * (n - 1) / 2);
    for _ in 0..rounds {
        for k in 0..half {
            if let (Some(a), Some(b)) = (slots[k], slots[m - 1 - k]) {
                pairs.push((teams[a], teams[b]));
            }
        }
        // Rotate all but the first slot clockwise.
        let last = slots[m - 1];
        let mut k = m - 1;
        while k > 1 {
            slots[k] = slots[k - 1];
            k -= 1;
        }
        slots[1] = last;
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_teams_make_six_unique_pairs() {
        let teams: Vec<TeamId> = (0..4).map(|_| TeamId::new()).collect();
        let pairs = round_robin_pairs(&teams);
        assert_eq!(pairs.len(), 6);

        // Every pair distinct, and no team paired with itself.
        let mut seen = std::collections::HashSet::new();
        for (a, b) in pairs {
            assert_ne!(a, b);
            let key = if a < b { (a, b) } else { (b, a) };
            assert!(seen.insert(key), "duplicate pair");
        }
    }

    #[test]
    fn circle_order_spreads_first_round() {
        // 6 teams → first round = 3 disjoint matches using all 6 teams (no team
        // plays twice in a row at the start, unlike naive i<j order).
        let teams: Vec<TeamId> = (0..6).map(|_| TeamId::new()).collect();
        let pairs = round_robin_pairs(&teams);
        let first_round = &pairs[0..3];
        let mut seen = std::collections::HashSet::new();
        for (a, b) in first_round {
            assert!(seen.insert(*a), "team twice in first round");
            assert!(seen.insert(*b), "team twice in first round");
        }
        assert_eq!(seen.len(), 6);
        assert_eq!(pairs.len(), 15);
    }

    #[test]
    fn degenerate_sizes() {
        assert!(round_robin_pairs(&[]).is_empty());
        assert!(round_robin_pairs(&[TeamId::new()]).is_empty());
        assert_eq!(round_robin_pairs(&[TeamId::new(), TeamId::new()]).len(), 1);
    }
}
