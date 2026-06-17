//! Persistence adapter (driven side of the hexagon): every event-store SQL query
//! lives here. The application layer ([`crate::App`]) depends on this adapter's
//! interface — the [`EventStore`] port — and never touches `sqlx` directly.
//!
//! Reads return raw JSON payloads; folding events into domain views is an
//! application concern and stays in [`crate`].

use serde_json::Value;
use sqlx::{Pool, Postgres, Row};

use crate::AppError;

/// The port the application needs from persistence: read raw event payloads and
/// hard-delete aggregate streams. Implemented by [`Db`] over PostgreSQL.
#[allow(async_fn_in_trait)]
pub trait EventStore {
    /// Payloads of one aggregate's events, in sequence order.
    async fn events_for(&self, agg_type: &str, agg_id: &str) -> Result<Vec<Value>, AppError>;
    /// `(aggregate_id, payload)` of every `Match` event, in global commit order.
    async fn match_events_global(&self) -> Result<Vec<(String, Value)>, AppError>;
    /// Ids of tournaments that have been created, in global order.
    async fn created_tournament_ids(&self) -> Result<Vec<String>, AppError>;
    /// Hard-delete one aggregate's events and snapshots.
    async fn delete_aggregate(&self, agg_type: &str, agg_id: &str) -> Result<(), AppError>;
    /// Hard-delete every `Match` stream belonging to a tournament.
    async fn delete_tournament_matches(&self, tournament_id: &str) -> Result<(), AppError>;
    /// Hard-delete every aggregate keyed by this id (Tournament + Bracket).
    async fn delete_by_id(&self, id: &str) -> Result<(), AppError>;
}

/// PostgreSQL-backed [`EventStore`], wrapping the shared `sqlx` pool.
#[derive(Clone)]
pub struct Db {
    pool: Pool<Postgres>,
}

impl Db {
    /// Wrap a connection pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    /// The underlying pool — needed to build the `postgres-es` CQRS framework.
    #[must_use]
    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }

    /// Apply the (idempotent) event-store schema.
    ///
    /// # Errors
    /// Returns any `sqlx` error from executing the schema statements.
    pub async fn run_migrations(&self, schema_sql: &str) -> Result<(), sqlx::Error> {
        sqlx::raw_sql(schema_sql).execute(&self.pool).await?;
        Ok(())
    }
}

impl EventStore for Db {
    async fn events_for(&self, agg_type: &str, agg_id: &str) -> Result<Vec<Value>, AppError> {
        let rows = sqlx::query(
            "SELECT payload FROM events \
             WHERE aggregate_type = $1 AND aggregate_id = $2 \
             ORDER BY sequence",
        )
        .bind(agg_type)
        .bind(agg_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(row.try_get::<Value, _>("payload")?);
        }
        Ok(out)
    }

    async fn match_events_global(&self) -> Result<Vec<(String, Value)>, AppError> {
        let rows = sqlx::query(
            "SELECT aggregate_id, payload FROM events \
             WHERE aggregate_type = 'Match' ORDER BY global_seq",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push((row.try_get::<String, _>("aggregate_id")?, row.try_get::<Value, _>("payload")?));
        }
        Ok(out)
    }

    async fn created_tournament_ids(&self) -> Result<Vec<String>, AppError> {
        let rows = sqlx::query(
            "SELECT aggregate_id FROM events \
             WHERE aggregate_type = 'Tournament' AND event_type = 'TournamentCreated' \
             ORDER BY global_seq",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(row.try_get::<String, _>("aggregate_id")?);
        }
        Ok(out)
    }

    async fn delete_aggregate(&self, agg_type: &str, agg_id: &str) -> Result<(), AppError> {
        for table in ["events", "snapshots"] {
            sqlx::query(&format!(
                "DELETE FROM {table} WHERE aggregate_type = $1 AND aggregate_id = $2"
            ))
            .bind(agg_type)
            .bind(agg_id)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn delete_tournament_matches(&self, tournament_id: &str) -> Result<(), AppError> {
        // Match aggregate ids belonging to this tournament, via the Scheduled payload.
        let subquery = "SELECT aggregate_id FROM events \
             WHERE aggregate_type = 'Match' AND event_type = 'MatchScheduled' \
             AND payload->'Scheduled'->>'tournament_id' = $1";
        // Snapshots first (their selection depends on the events still existing).
        for table in ["snapshots", "events"] {
            sqlx::query(&format!(
                "DELETE FROM {table} WHERE aggregate_type = 'Match' AND aggregate_id IN ({subquery})"
            ))
            .bind(tournament_id)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn delete_by_id(&self, id: &str) -> Result<(), AppError> {
        for table in ["events", "snapshots"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE aggregate_id = $1"))
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }
}
