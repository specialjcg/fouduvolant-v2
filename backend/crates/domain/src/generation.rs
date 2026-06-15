//! Match generation for the pool stage.
//!
//! Pure combinatorics, no I/O: turning a pool's team list into the set of
//! matches to schedule. Single round-robin — every team plays every other once.

use crate::ids::TeamId;

/// All unordered pairs of a pool's teams (single round-robin).
///
/// For `n` teams this yields `n * (n - 1) / 2` pairs, each exactly once, in a
/// deterministic order derived from the input slice order.
#[must_use]
pub fn round_robin_pairs(teams: &[TeamId]) -> Vec<(TeamId, TeamId)> {
    let mut pairs = Vec::with_capacity(teams.len() * teams.len().saturating_sub(1) / 2);
    for i in 0..teams.len() {
        for j in (i + 1)..teams.len() {
            pairs.push((teams[i], teams[j]));
        }
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
    fn degenerate_sizes() {
        assert!(round_robin_pairs(&[]).is_empty());
        assert!(round_robin_pairs(&[TeamId::new()]).is_empty());
        assert_eq!(round_robin_pairs(&[TeamId::new(), TeamId::new()]).len(), 1);
    }
}
