//! Scoring value objects and badminton rules.
//!
//! A match is a sequence of sets. Each set is played to 21 points, win by 2,
//! capped at 30 (so 30-29 is a valid winning score). The match format decides
//! how many sets win the match.

use serde::{Deserialize, Serialize};

/// How many sets decide a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MatchFormat {
    /// One set. Used for pool matches.
    #[default]
    BestOf1,
    /// First to two sets. Used for bracket/finals matches.
    BestOf3,
}

impl MatchFormat {
    /// Number of set wins required to win the match.
    #[must_use]
    pub const fn sets_to_win(self) -> u8 {
        match self {
            MatchFormat::BestOf1 => 1,
            MatchFormat::BestOf3 => 2,
        }
    }

    /// Maximum number of sets that can be played.
    #[must_use]
    pub const fn max_sets(self) -> u8 {
        match self {
            MatchFormat::BestOf1 => 1,
            MatchFormat::BestOf3 => 3,
        }
    }
}

/// Which side won a set or match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetOutcome {
    /// Side A (`team_a`) won.
    SideA,
    /// Side B (`team_b`) won.
    SideB,
}

/// A completed set score. Construction enforces the badminton rules, so an
/// existing `SetScore` is always a valid, finished set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetScore {
    a: u8,
    b: u8,
}

/// Reasons a proposed set score is not a valid finished badminton set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScoreError {
    /// A score exceeds the hard cap of 30.
    #[error("score {0} exceeds the maximum of 30")]
    AboveCap(u8),
    /// The set is not finished (no side reached a winning score).
    #[error("set is not finished: {a}-{b}")]
    Unfinished {
        /// Points for side A.
        a: u8,
        /// Points for side B.
        b: u8,
    },
    /// Winner did not have the required 2-point margin.
    #[error("invalid margin for {a}-{b}: must win by 2 (unless capped at 30-29)")]
    BadMargin {
        /// Points for side A.
        a: u8,
        /// Points for side B.
        b: u8,
    },
}

const TARGET: u8 = 21;
const CAP: u8 = 30;

impl SetScore {
    /// Build a validated, finished set score.
    ///
    /// # Errors
    /// Returns [`ScoreError`] if the score is over the cap, unfinished, or has
    /// an illegal winning margin.
    pub fn new(a: u8, b: u8) -> Result<Self, ScoreError> {
        if a > CAP {
            return Err(ScoreError::AboveCap(a));
        }
        if b > CAP {
            return Err(ScoreError::AboveCap(b));
        }
        let (hi, lo) = (a.max(b), a.min(b));
        // The winner must reach at least 21.
        if hi < TARGET {
            return Err(ScoreError::Unfinished { a, b });
        }
        // 30-29 is the only legal one-point set (hard cap reached).
        if hi == CAP {
            if lo == CAP - 1 {
                return Ok(Self { a, b });
            }
            return Err(ScoreError::BadMargin { a, b });
        }
        // Below the cap, the winner needs a two-point lead.
        if hi - lo >= 2 {
            // ...but cannot have run away past 21 without going through deuce:
            // a sub-cap winner is either 21 vs <=19, or N-(N-2) for 22..=29.
            if hi == TARGET || hi - lo == 2 {
                return Ok(Self { a, b });
            }
        }
        Err(ScoreError::BadMargin { a, b })
    }

    /// Points scored by side A.
    #[must_use]
    pub const fn a(self) -> u8 {
        self.a
    }

    /// Points scored by side B.
    #[must_use]
    pub const fn b(self) -> u8 {
        self.b
    }

    /// Which side won this set.
    #[must_use]
    pub const fn winner(self) -> SetOutcome {
        if self.a > self.b {
            SetOutcome::SideA
        } else {
            SetOutcome::SideB
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_win() {
        assert_eq!(SetScore::new(21, 19).unwrap().winner(), SetOutcome::SideA);
        assert_eq!(SetScore::new(15, 21).unwrap().winner(), SetOutcome::SideB);
    }

    #[test]
    fn deuce_wins() {
        assert!(SetScore::new(22, 20).is_ok());
        assert!(SetScore::new(29, 27).is_ok());
        assert!(SetScore::new(30, 29).is_ok());
    }

    #[test]
    fn rejects_unfinished() {
        assert_eq!(
            SetScore::new(20, 18),
            Err(ScoreError::Unfinished { a: 20, b: 18 })
        );
        assert!(SetScore::new(19, 21).is_ok());
    }

    #[test]
    fn rejects_bad_margin() {
        assert!(matches!(SetScore::new(21, 20), Err(ScoreError::BadMargin { .. })));
        assert!(matches!(SetScore::new(23, 20), Err(ScoreError::BadMargin { .. })));
        assert!(matches!(SetScore::new(30, 28), Err(ScoreError::BadMargin { .. })));
    }

    #[test]
    fn rejects_above_cap() {
        assert_eq!(SetScore::new(31, 10), Err(ScoreError::AboveCap(31)));
    }

    #[test]
    fn format_thresholds() {
        assert_eq!(MatchFormat::BestOf1.sets_to_win(), 1);
        assert_eq!(MatchFormat::BestOf3.sets_to_win(), 2);
        assert_eq!(MatchFormat::BestOf3.max_sets(), 3);
    }
}
