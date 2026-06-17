//! Request and response bodies for the HTTP API. Field visibility is
//! `pub(crate)` so the handler modules can build and read them.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use domain::ids::{CourtId, MatchId};
use domain::score::MatchFormat;

#[derive(Deserialize)]
pub(crate) struct CreateTournament {
    pub(crate) name: String,
    pub(crate) pool_format: MatchFormat,
    pub(crate) bracket_format: MatchFormat,
}

#[derive(Deserialize)]
pub(crate) struct TeamBody {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) player1: String,
    #[serde(default)]
    pub(crate) player2: String,
}

#[derive(Deserialize)]
pub(crate) struct ImportTeams {
    pub(crate) names: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct ImportResult {
    pub(crate) created: usize,
}

#[derive(Deserialize)]
pub(crate) struct PoolInput {
    pub(crate) name: String,
    pub(crate) teams: Vec<Uuid>,
}

#[derive(Deserialize)]
pub(crate) struct GeneratePools {
    pub(crate) pools: Vec<PoolInput>,
}

#[derive(Deserialize)]
pub(crate) struct ConfigureCourts {
    pub(crate) count: usize,
}

#[derive(Deserialize)]
pub(crate) struct AssignCourt {
    pub(crate) court_id: Uuid,
}

#[derive(Deserialize)]
pub(crate) struct ScheduleMatch {
    pub(crate) format: MatchFormat,
    pub(crate) team_a: Uuid,
    pub(crate) team_b: Uuid,
    pub(crate) pool_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub(crate) struct StartMatch {
    pub(crate) court_id: Uuid,
}

#[derive(Deserialize)]
pub(crate) struct RecordSetBody {
    pub(crate) a: u8,
    pub(crate) b: u8,
}

#[derive(Deserialize)]
pub(crate) struct SetFormatBody {
    pub(crate) format: MatchFormat,
}

#[derive(Deserialize)]
pub(crate) struct SetRoundFormatBody {
    pub(crate) round_size: u16,
    pub(crate) format: MatchFormat,
}

#[derive(Deserialize)]
pub(crate) struct ConcedeBody {
    pub(crate) winner: Uuid,
}

#[derive(Serialize)]
pub(crate) struct IdResponse {
    pub(crate) id: Uuid,
}

#[derive(Serialize)]
pub(crate) struct CourtsResponse {
    pub(crate) courts: Vec<CourtId>,
}

#[derive(Serialize)]
pub(crate) struct DispatchResponse {
    pub(crate) started: Vec<MatchId>,
}

#[derive(Serialize)]
pub(crate) struct CreatedResponse {
    pub(crate) created: Vec<MatchId>,
}

#[derive(Deserialize)]
pub(crate) struct GenerateBracket {
    pub(crate) per_pool: usize,
}
