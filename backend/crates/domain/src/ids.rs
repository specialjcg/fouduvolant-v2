//! Typed identifiers. Newtypes over `Uuid` so a `TeamId` can never be passed
//! where a `MatchId` is expected.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Generate a fresh random identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self(Uuid::nil())
            }
        }

        impl From<Uuid> for $name {
            fn from(u: Uuid) -> Self {
                Self(u)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type!(
    /// Identifies a tournament aggregate.
    TournamentId
);
id_type!(
    /// Identifies a team within a tournament.
    TeamId
);
id_type!(
    /// Identifies a match aggregate.
    MatchId
);
id_type!(
    /// Identifies a pool (group-stage group).
    PoolId
);
id_type!(
    /// Identifies a physical court.
    CourtId
);
