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

/// Regression (TDD): the early garbage draw schedules finals matches that the
/// dispatcher can auto-start (a court gets assigned) before any score exists.
/// Re-clicking "Générer" must still re-seed in that state — a *started but
/// unscored* finals match is itself garbage and safe to drop. Only a finals
/// match with a recorded winner should block the re-seed.
#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn regenerate_bracket_reseeds_even_if_garbage_final_started() {
    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    let t_id = TournamentId::new();
    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "ReseedStarted".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf1,
        },
    )
    .await
    .expect("create");

    let mut p1 = [TeamId::new(), TeamId::new(), TeamId::new()];
    let mut p2 = [TeamId::new(), TeamId::new(), TeamId::new()];
    p1.sort();
    p2.sort();
    let (p1_la, p1_lb, p1_win) = (p1[0], p1[1], p1[2]);
    let (p2_la, p2_lb, p2_win) = (p2[0], p2[1], p2[2]);

    for (id, name) in [
        (p1_win, "P1_WIN"),
        (p1_la, "P1_A"),
        (p1_lb, "P1_B"),
        (p2_win, "P2_WIN"),
        (p2_la, "P2_A"),
        (p2_lb, "P2_B"),
    ] {
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

    // Two courts: one for the (garbage) final, one for pool matches.
    let (c_pool, c_final) = (CourtId::new(), CourtId::new());
    app.tournament(t_id, TournamentCommand::ConfigureCourts { courts: vec![c_pool, c_final] })
        .await
        .expect("courts");
    app.tournament(t_id, TournamentCommand::StartPoolPhase)
        .await
        .expect("start pools");

    // Early draw on empty standings → returns the scheduled garbage final(s).
    let early = app.generate_bracket(t_id, 1).await.expect("early draw");
    assert!(!early.is_empty(), "early draw schedules a final");
    // Start AND SCORE the garbage final — mimics prod, where the bad draw's
    // finals were auto-dispatched and even played before anyone noticed.
    app.match_cmd(early[0], MatchCommand::Start { court_id: c_final })
        .await
        .expect("start garbage final");
    app.match_cmd(early[0], MatchCommand::RecordSet { a: 21, b: 0 })
        .await
        .expect("score garbage final");

    // Play the pools so the real winners emerge.
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
        app.match_cmd(m, MatchCommand::Start { court_id: c_pool })
            .await
            .expect("start");
        app.match_cmd(m, MatchCommand::RecordSet { a: 21, b: 0 })
            .await
            .expect("record");
    }

    // Re-generate must succeed (started-but-unscored garbage final dropped).
    app.generate_bracket(t_id, 1).await.expect("regenerate after started garbage final");

    let view = app.bracket_view(t_id).await.expect("bracket view");
    let main_names: std::collections::HashSet<String> = view
        .iter()
        .filter(|n| matches!(n.kind, BracketKind::Main))
        .flat_map(|n| [n.team_a.clone(), n.team_b.clone()])
        .flatten()
        .collect();
    assert!(main_names.contains("P1_WIN"), "pool 1 winner in main after re-seed");
    assert!(main_names.contains("P2_WIN"), "pool 2 winner in main after re-seed");
    assert!(!main_names.contains("P1_A"), "pool 1 loser not in main");
}

/// Safety: re-clicking "Générer" when the qualifiers are unchanged must NOT wipe
/// a real, in-progress bracket — its recorded finals results have to survive.
#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn regenerate_bracket_preserves_results_when_seeds_unchanged() {
    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    let t_id = TournamentId::new();
    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "Preserve".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf1,
        },
    )
    .await
    .expect("create");

    let (a, b, c, d) = (TeamId::new(), TeamId::new(), TeamId::new(), TeamId::new());
    for (id, name) in [(a, "A"), (b, "B"), (c, "C"), (d, "D")] {
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
                Pool { id: pool1, name: "P1".into(), teams: vec![a, b] },
                Pool { id: pool2, name: "P2".into(), teams: vec![c, d] },
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

    // Score the pools first: A wins P1, C wins P2.
    for (x, y, pool) in [(a, b, pool1), (c, d, pool2)] {
        let m = MatchId::new();
        app.match_cmd(
            m,
            MatchCommand::Schedule {
                match_id: m,
                tournament_id: t_id,
                format: MatchFormat::BestOf1,
                team_a: x,
                team_b: y,
                pool_id: Some(pool),
            },
        )
        .await
        .expect("schedule pool");
        app.match_cmd(m, MatchCommand::Start { court_id: court }).await.expect("start");
        app.match_cmd(m, MatchCommand::RecordSet { a: 21, b: 0 }).await.expect("record");
    }

    // Correct draw → final A vs C; play it (A wins).
    let created = app.generate_bracket(t_id, 1).await.expect("draw");
    let final_id = *created.first().expect("a final was scheduled");
    app.match_cmd(final_id, MatchCommand::Start { court_id: court }).await.expect("start final");
    app.match_cmd(final_id, MatchCommand::RecordSet { a: 21, b: 0 }).await.expect("score final");

    // Re-generate with the same per_pool → seeds identical → result preserved.
    app.generate_bracket(t_id, 1).await.expect("regenerate same seeds");

    let view = app.bracket_view(t_id).await.expect("view");
    let final_winner = view
        .iter()
        .filter(|n| matches!(n.kind, BracketKind::Main))
        .find_map(|n| n.winner.clone());
    assert_eq!(final_winner.as_deref(), Some("A"), "real finals result survives a re-Générer");
}

/// Guarantee: the `per_pool` argument actually changes who qualifies — more
/// per pool ⇒ a bigger main draw. Locks the behaviour behind the "2 ou 3 ne
/// change rien" report (which was stale client JS, not the backend).
#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn per_pool_changes_the_qualified_count() {
    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    let t_id = TournamentId::new();
    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "PerPool".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf1,
        },
    )
    .await
    .expect("create");

    // Two pools of three with a clear 1/2/3 ranking each.
    let p1 = [TeamId::new(), TeamId::new(), TeamId::new()];
    let p2 = [TeamId::new(), TeamId::new(), TeamId::new()];
    for (i, id) in p1.iter().chain(p2.iter()).enumerate() {
        app.tournament(
            t_id,
            TournamentCommand::RegisterTeam {
                team_id: *id,
                name: format!("T{i}"),
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
                Pool { id: pool1, name: "P1".into(), teams: p1.to_vec() },
                Pool { id: pool2, name: "P2".into(), teams: p2.to_vec() },
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

    // In each pool: teams[0] wins both, teams[1] beats teams[2].
    for p in [&p1, &p2] {
        for (a, b) in [(p[0], p[1]), (p[0], p[2]), (p[1], p[2])] {
            let m = MatchId::new();
            app.match_cmd(
                m,
                MatchCommand::Schedule {
                    match_id: m,
                    tournament_id: t_id,
                    format: MatchFormat::BestOf1,
                    team_a: a,
                    team_b: b,
                    pool_id: Some(if p.as_ptr() == p1.as_ptr() { pool1 } else { pool2 }),
                },
            )
            .await
            .expect("schedule");
            app.match_cmd(m, MatchCommand::Start { court_id: court }).await.expect("start");
            app.match_cmd(m, MatchCommand::RecordSet { a: 21, b: 0 }).await.expect("record");
        }
    }

    let main_count = |view: &[app::BracketNodeView]| -> usize {
        let mut s = std::collections::HashSet::new();
        for n in view {
            if matches!(n.kind, BracketKind::Main) {
                for t in [n.team_a.clone(), n.team_b.clone()].into_iter().flatten() {
                    s.insert(t);
                }
            }
        }
        s.len()
    };

    app.generate_bracket(t_id, 1).await.expect("draw per_pool=1");
    let one = main_count(&app.bracket_view(t_id).await.expect("view1"));

    app.generate_bracket(t_id, 2).await.expect("draw per_pool=2");
    let two = main_count(&app.bracket_view(t_id).await.expect("view2"));

    assert_eq!(one, 2, "per_pool=1 → 1 team per pool in the main draw");
    assert_eq!(two, 4, "per_pool=2 → 2 teams per pool in the main draw");
    assert!(two > one, "raising per_pool enlarges the main draw");
}
