//! HTTP API for fouduvolant — a thin axum layer over the [`app`] command and
//! read methods. Each handler deserialises a request, issues a domain command or
//! read, and maps the result to JSON. Identifiers for new aggregates are
//! generated server-side and returned in the response.

mod dto;
mod error;
mod handlers;
mod router;

use std::sync::Arc;

use app::App;

const DEFAULT_DATABASE_URL: &str =
    "postgresql://fouduvolant:fouduvolant@localhost:5432/fouduvolant";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .init();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string());
    let app = App::connect(&database_url).await;
    app.run_migrations()
        .await
        .expect("event-store migrations failed");

    let router = router::router(Arc::new(app));

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".into());
    let host = std::env::var("BIND_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    tracing::info!("listening on {addr}");
    axum::serve(listener, router).await.expect("server error");
}
