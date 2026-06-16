//! End-to-end persistence test against a real PostgreSQL instance.
//!
//! Ignored by default — it needs a running database. Run with:
//!   DATABASE_URL=postgresql://fouduvolant:fouduvolant@localhost:5432/fouduvolant \
//!     cargo test -p app -- --ignored
//!
//! Each run uses fresh UUIDs, so events never collide on the primary key.

use domain::bracket::BracketKind;
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

/// Regression: a bracket drawn *before* pool results exist freezes its seeding
/// on empty (all-zero) standings, which rank teams by id. Re-clicking "Générer"
/// after the scores are entered must re-seed from the now-correct standings —
/// previously it was a no-op (AlreadyDrawn) and the wrong teams stayed qualified.
#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn regenerate_bracket_reseeds_after_pool_scores() {
    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    let t_id = TournamentId::new();
    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "Reseed".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf1,
        },
    )
    .await
    .expect("create");

    // Two pools of three. In each pool the winner is the MAX team id, so the
    // empty-standings draw (which ranks by id ascending) qualifies the WRONG
    // team — making the bug deterministic.
    let mut p1 = [TeamId::new(), TeamId::new(), TeamId::new()];
    let mut p2 = [TeamId::new(), TeamId::new(), TeamId::new()];
    p1.sort();
    p2.sort();
    let (p1_la, p1_lb, p1_win) = (p1[0], p1[1], p1[2]);
    let (p2_la, p2_lb, p2_win) = (p2[0], p2[1], p2[2]);

    let names = [
        (p1_win, "P1_WIN"),
        (p1_la, "P1_A"),
        (p1_lb, "P1_B"),
        (p2_win, "P2_WIN"),
        (p2_la, "P2_A"),
        (p2_lb, "P2_B"),
    ];
    for (id, name) in names {
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

    let (pool1, pool2) = (PoolId::new(), PoolId::new());
    app.tournament(
        t_id,
        TournamentCommand::GeneratePools {
            pools: vec![
                Pool { id: pool1, name: "P1".into(), teams: vec![p1_la, p1_lb, p1_win] },
                Pool { id: pool2, name: "P2".into(), teams: vec![p2_la, p2_lb, p2_win] },
            ],
        },
    )
    .await
    .expect("pools");

    let court = CourtId::new();
    app.tournament(t_id, TournamentCommand::ConfigureCourts { courts: vec![court] })
        .await
        .expect("courts");
    app.tournament(t_id, TournamentCommand::StartPoolPhase)
        .await
        .expect("start pools");

    // Draw the bracket EARLY — no scores yet, standings all zero.
    app.generate_bracket(t_id, 1).await.expect("early draw");

    // Now play the pool matches: each winner (team_a) beats both opponents on a
    // single court, sequentially — BO1 completes and frees the court each time.
    for (a, b, pool) in [
        (p1_win, p1_la, pool1),
        (p1_win, p1_lb, pool1),
        (p2_win, p2_la, pool2),
        (p2_win, p2_lb, pool2),
    ] {
        let m = MatchId::new();
        app.match_cmd(
            m,
            MatchCommand::Schedule {
                match_id: m,
                tournament_id: t_id,
                format: MatchFormat::BestOf1,
                team_a: a,
                team_b: b,
                pool_id: Some(pool),
            },
        )
        .await
        .expect("schedule pool match");
        app.match_cmd(m, MatchCommand::Start { court_id: court })
            .await
            .expect("start");
        app.match_cmd(m, MatchCommand::RecordSet { a: 21, b: 0 })
            .await
            .expect("record");
    }

    // Re-generate — must re-seed from the real standings.
    app.generate_bracket(t_id, 1).await.expect("regenerate");

    let view = app.bracket_view(t_id).await.expect("bracket view");
    let main_names: std::collections::HashSet<String> = view
        .iter()
        .filter(|n| matches!(n.kind, BracketKind::Main))
        .flat_map(|n| [n.team_a.clone(), n.team_b.clone()])
        .flatten()
        .collect();
    let cons_names: std::collections::HashSet<String> = view
        .iter()
        .filter(|n| matches!(n.kind, BracketKind::Consolation))
        .flat_map(|n| [n.team_a.clone(), n.team_b.clone()])
        .flatten()
        .collect();

    assert!(main_names.contains("P1_WIN"), "pool 1 winner is in the main draw");
    assert!(main_names.contains("P2_WIN"), "pool 2 winner is in the main draw");
    assert!(!main_names.contains("P1_A") && !main_names.contains("P1_B"), "pool 1 losers not in main");
    assert!(cons_names.contains("P1_A"), "pool 1 loser is in the consolation draw");
}
