//! Pool standings with BWF-style tiebreakers.
//!
//! Pure: turns a pool's completed match results into a ranked table. Ranking
//! keys, in order:
//!   1. wins (descending)
//!   2. head-to-head wins among the teams tied on wins
//!   3. overall point difference (descending)
//!   4. overall points scored (descending)
//!   5. team id (deterministic final tiebreak)
//!
//! H2H is applied *before* point difference, and only within a group of teams
//! that are level on wins — matching the original fouduvolant behaviour.

use std::collections::{HashMap, HashSet};

use crate::ids::TeamId;

/// One finished match's outcome, the input unit for standings.
#[derive(Debug, Clone, Copy)]
pub struct MatchResult {
    /// First side.
    pub team_a: TeamId,
    /// Second side.
    pub team_b: TeamId,
    /// Winning team (must be `team_a` or `team_b`).
    pub winner: TeamId,
    /// Total points scored by side A (summed over sets).
    pub points_a: u32,
    /// Total points scored by side B (summed over sets).
    pub points_b: u32,
}

/// A team's aggregated record within a pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    /// The team.
    pub team: TeamId,
    /// Matches played.
    pub played: u32,
    /// Matches won.
    pub wins: u32,
    /// Total points scored.
    pub points_for: u32,
    /// Total points conceded.
    pub points_against: u32,
}

impl Standing {
    /// Point difference (for − against).
    #[must_use]
    pub fn diff(&self) -> i32 {
        self.points_for as i32 - self.points_against as i32
    }
}

/// Rank a pool's teams. Teams with no recorded matches still appear (0 played).
#[must_use]
pub fn pool_standings(teams: &[TeamId], results: &[MatchResult]) -> Vec<Standing> {
    let mut table: HashMap<TeamId, Standing> = teams
        .iter()
        .map(|&t| {
            (
                t,
                Standing {
                    team: t,
                    played: 0,
                    wins: 0,
                    points_for: 0,
                    points_against: 0,
                },
            )
        })
        .collect();

    for r in results {
        if let Some(s) = table.get_mut(&r.team_a) {
            s.played += 1;
            s.points_for += r.points_a;
            s.points_against += r.points_b;
            if r.winner == r.team_a {
                s.wins += 1;
            }
        }
        if let Some(s) = table.get_mut(&r.team_b) {
            s.played += 1;
            s.points_for += r.points_b;
            s.points_against += r.points_a;
            if r.winner == r.team_b {
                s.wins += 1;
            }
        }
    }

    let mut standings: Vec<Standing> = table.into_values().collect();
    // Primary order by wins; team id keeps it deterministic before tiebreaking.
    standings.sort_by(|a, b| b.wins.cmp(&a.wins).then(a.team.cmp(&b.team)));

    // Resolve each run of equal-wins teams with the H2H-first tiebreak.
    let mut start = 0;
    while start < standings.len() {
        let mut end = start + 1;
        while end < standings.len() && standings[end].wins == standings[start].wins {
            end += 1;
        }
        if end - start > 1 {
            let group: HashSet<TeamId> =
                standings[start..end].iter().map(|s| s.team).collect();
            standings[start..end].sort_by(|a, b| {
                let ha = h2h_wins(a.team, &group, results);
                let hb = h2h_wins(b.team, &group, results);
                hb.cmp(&ha)
                    .then(b.diff().cmp(&a.diff()))
                    .then(b.points_for.cmp(&a.points_for))
                    .then(a.team.cmp(&b.team))
            });
        }
        start = end;
    }

    standings
}

/// Wins by `team` in matches played strictly between members of `group`.
fn h2h_wins(team: TeamId, group: &HashSet<TeamId>, results: &[MatchResult]) -> u32 {
    results
        .iter()
        .filter(|r| {
            r.winner == team && group.contains(&r.team_a) && group.contains(&r.team_b)
        })
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team(n: u128) -> TeamId {
        TeamId(uuid::Uuid::from_u128(n))
    }

    fn res(a: TeamId, b: TeamId, winner: TeamId, pa: u32, pb: u32) -> MatchResult {
        MatchResult {
            team_a: a,
            team_b: b,
            winner,
            points_a: pa,
            points_b: pb,
        }
    }

    #[test]
    fn orders_by_wins() {
        let (a, b, c) = (team(1), team(2), team(3));
        let results = vec![
            res(a, b, a, 21, 10),
            res(a, c, a, 21, 12),
            res(b, c, b, 21, 18),
        ];
        let s = pool_standings(&[a, b, c], &results);
        assert_eq!(s[0].team, a); // 2 wins
        assert_eq!(s[1].team, b); // 1 win
        assert_eq!(s[2].team, c); // 0 wins
    }

    #[test]
    fn three_way_circular_falls_to_diff() {
        // a>b, b>c, c>a: all on 1 win, H2H mini-table circular (each 1) → diff.
        let (a, b, c) = (team(1), team(2), team(3));
        let results = vec![
            res(a, b, a, 21, 19), // a diff +2 here
            res(b, c, b, 21, 2),  // b diff +19 here
            res(a, c, c, 15, 21), // a -6, c +6
        ];
        // diffs: a = 2-6 = -4 ; b = -2+19 = 17 ; c = -19+6 = -13
        let s = pool_standings(&[a, b, c], &results);
        assert_eq!(s[0].team, b);
        assert_eq!(s[1].team, a);
        assert_eq!(s[2].team, c);
    }

    #[test]
    fn two_way_tie_uses_h2h_over_diff() {
        // Full 4-team round robin. a and b tie on 2 wins; a beat b head-to-head
        // but has a far worse overall diff — H2H must still rank a above b.
        let (a, b, c, d) = (team(1), team(2), team(3), team(4));
        let results = vec![
            res(a, b, a, 21, 19), // a beats b (close)
            res(a, c, a, 21, 20),
            res(d, a, d, 21, 15), // a loses to d
            res(b, c, b, 21, 1),  // b crushes c
            res(b, d, b, 21, 1),  // b crushes d
            res(c, d, c, 21, 10), // c beats d
        ];
        // wins: a=2, b=2, c=1, d=1. a diff = +2+1-6 = -3 ; b diff = -2+20+20 = +38.
        let s = pool_standings(&[a, b, c, d], &results);
        assert_eq!(s[0].team, a, "a wins H2H despite worse diff");
        assert_eq!(s[1].team, b);
        assert_eq!(s[2].team, c, "c beat d head-to-head");
        assert_eq!(s[3].team, d);
    }

    #[test]
    fn empty_teams_appear_with_zero() {
        let (a, b) = (team(1), team(2));
        let s = pool_standings(&[a, b], &[]);
        assert_eq!(s.len(), 2);
        assert!(s.iter().all(|x| x.played == 0 && x.wins == 0));
    }
}
