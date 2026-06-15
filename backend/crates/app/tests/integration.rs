//! End-to-end persistence test against a real PostgreSQL instance.
//!
//! Ignored by default — it needs a running database. Run with:
//!   DATABASE_URL=postgresql://fouduvolant:fouduvolant@localhost:5432/fouduvolant \
//!     cargo test -p app -- --ignored
//!
//! Each run uses fresh UUIDs, so events never collide on the primary key.

use domain::ids::{CourtId, MatchId, PoolId, TeamId, TournamentId};
use domain::matches::MatchCommand;
use domain::score::MatchFormat;
use domain::tournament::{Pool, TournamentCommand};

use app::App;

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .expect("set DATABASE_URL to a running PostgreSQL instance")
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn full_tournament_and_match_flow_persists() {
    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    // --- Tournament setup ---
    let t_id = TournamentId::new();
    let (a, b) = (TeamId::new(), TeamId::new());

    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "Integration Open".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf3,
        },
    )
    .await
    .expect("create");

    for (id, name) in [(a, "A"), (b, "B")] {
        app.tournament(
            t_id,
            TournamentCommand::RegisterTeam {
                team_id: id,
                name: name.into(),
                player1: String::new(),
                player2: String::new(),
            },
        )
        .await
        .expect("register");
    }

    let pool = PoolId::new();
    app.tournament(
        t_id,
        TournamentCommand::GeneratePools {
            pools: vec![Pool {
                id: pool,
                name: "P1".into(),
                teams: vec![a, b],
            }],
        },
    )
    .await
    .expect("pools");

    let court = CourtId::new();
    app.tournament(
        t_id,
        TournamentCommand::ConfigureCourts {
            courts: vec![court],
        },
    )
    .await
    .expect("courts");

    app.tournament(t_id, TournamentCommand::StartPoolPhase)
        .await
        .expect("start pool phase");

    // Re-create on the same id must be rejected (state was rehydrated from store).
    let dup = app
        .tournament(
            t_id,
            TournamentCommand::Create {
                tournament_id: t_id,
                name: "dup".into(),
                pool_format: MatchFormat::BestOf1,
                bracket_format: MatchFormat::BestOf3,
            },
        )
        .await;
    assert!(dup.is_err(), "duplicate create rejected after rehydration");

    // --- Match flow ---
    let m_id = MatchId::new();
    app.match_cmd(
        m_id,
        MatchCommand::Schedule {
            match_id: m_id,
            tournament_id: t_id,
            format: MatchFormat::BestOf1,
            team_a: a,
            team_b: b,
            pool_id: Some(pool),
        },
    )
    .await
    .expect("schedule");

    app.match_cmd(m_id, MatchCommand::Start { court_id: court })
        .await
        .expect("start");

    // BO1: one set finishes the match.
    app.match_cmd(m_id, MatchCommand::RecordSet { a: 21, b: 15 })
        .await
        .expect("record set");

    // Scoring again must be rejected — match completed and rehydrated.
    let after = app
        .match_cmd(m_id, MatchCommand::RecordSet { a: 21, b: 0 })
        .await;
    assert!(after.is_err(), "completed match rejects more scoring");
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn dispatch_courts_starts_free_courts() {
    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    // Tournament with two courts (no pool phase needed for dispatch).
    let t_id = TournamentId::new();
    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "Dispatch".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf3,
        },
    )
    .await
    .expect("create");

    let courts = [CourtId::new(), CourtId::new()];
    app.tournament(
        t_id,
        TournamentCommand::ConfigureCourts {
            courts: courts.to_vec(),
        },
    )
    .await
    .expect("courts");

    // Three pending matches, all-distinct teams (so anti-btb never blocks).
    let pool = PoolId::new();
    let mut match_ids = Vec::new();
    for _ in 0..3 {
        let m = MatchId::new();
        match_ids.push(m);
        app.match_cmd(
            m,
            MatchCommand::Schedule {
                match_id: m,
                tournament_id: t_id,
                format: MatchFormat::BestOf1,
                team_a: TeamId::new(),
                team_b: TeamId::new(),
                pool_id: Some(pool),
            },
        )
        .await
        .expect("schedule");
    }

    // Two free courts → two matches started.
    let first = app.dispatch_courts(t_id).await.expect("dispatch 1");
    assert_eq!(first.len(), 2, "both courts filled");

    // Both courts busy now → nothing more starts.
    let none = app.dispatch_courts(t_id).await.expect("dispatch 2");
    assert!(none.is_empty(), "no free court");

    // Finish one started match → its court frees.
    app.match_cmd(first[0], MatchCommand::RecordSet { a: 21, b: 0 })
        .await
        .expect("finish one");

    // One free court → the third match starts.
    let third = app.dispatch_courts(t_id).await.expect("dispatch 3");
    assert_eq!(third.len(), 1, "freed court gets the last match");
    assert!(!first.contains(&third[0]), "a different match started");
}
