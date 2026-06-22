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
use domain::scheduling::SchedStatus;
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

/// A bracket match becomes playable as soon as both its teams are known —
/// recording a result must schedule the next round automatically, so the
/// operator can launch any match without first clicking "Avancer".
#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn recording_a_bracket_result_auto_advances() {
    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    let t_id = TournamentId::new();
    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "AutoAdvance".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf1,
        },
    )
    .await
    .expect("create");

    // Two pools of three; in each, t0 beats both and t1 beats t2 → ranks 1/2/3.
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

    for (a, b, pool) in [
        (p1[0], p1[1], pool1),
        (p1[0], p1[2], pool1),
        (p2[0], p2[1], pool2),
        (p2[0], p2[2], pool2),
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
        .expect("schedule");
        app.match_cmd(m, MatchCommand::Start { court_id: court }).await.expect("start");
        app.match_cmd(m, MatchCommand::RecordSet { a: 21, b: 0 }).await.expect("record");
    }

    // 2 qualified/pool → main draw of 4 (two semis + a final).
    app.generate_bracket(t_id, 2).await.expect("draw");

    // Play every playable bracket match via the auto-advancing path, never
    // calling advance_bracket. If auto-advance works, the final gets scheduled
    // and played; otherwise the loop runs dry after the semis.
    let mut rounds = 0;
    loop {
        let board = app.board(t_id).await.expect("board");
        let pending: Vec<MatchId> = board
            .matches
            .iter()
            .filter(|m| m.pool.is_none() && m.status == SchedStatus::Pending)
            .map(|m| m.id)
            .collect();
        if pending.is_empty() {
            break;
        }
        for id in pending {
            app.match_cmd(id, MatchCommand::Start { court_id: court }).await.expect("start bracket");
            app.record_set(id, 21, 0).await.expect("record bracket");
        }
        rounds += 1;
        assert!(rounds < 6, "bracket should converge");
    }

    // The main final exists, is decided, and was reached without a manual advance.
    let view = app.bracket_view(t_id).await.expect("view");
    let final_node = view
        .iter()
        .find(|n| matches!(n.kind, BracketKind::Main) && n.round == 2);
    let final_node = final_node.expect("a main final exists");
    assert!(final_node.team_a.is_some() && final_node.team_b.is_some(), "final has both teams");
    assert!(final_node.winner.is_some(), "final was played thanks to auto-advance");
    assert!(rounds >= 2, "took at least a semis round then a final round");
}

/// Resetting a bracket match drops its result so it can be replayed: the match
/// is re-created fresh (teams still known) and any downstream match that used
/// its result is reconciled away.
#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn reset_bracket_match_clears_result_for_replay() {
    use domain::bracket::BracketCommand;

    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    let t_id = TournamentId::new();
    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "Reset".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf1,
        },
    )
    .await
    .expect("create");

    let teams = [TeamId::new(), TeamId::new(), TeamId::new(), TeamId::new()];
    for (i, id) in teams.iter().enumerate() {
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
    let court = CourtId::new();
    app.tournament(t_id, TournamentCommand::ConfigureCourts { courts: vec![court] })
        .await
        .expect("courts");

    // Draw a 4-team main bracket (two semis + final) and schedule the semis.
    app.bracket(
        t_id,
        BracketCommand::Draw { main_seeds: teams.to_vec(), consolation_seeds: vec![] },
    )
    .await
    .expect("draw");
    app.advance_bracket(t_id).await.expect("advance");

    // Play one semi.
    let semi = {
        let board = app.board(t_id).await.expect("board");
        board
            .matches
            .iter()
            .find(|m| m.pool.is_none() && m.status == SchedStatus::Pending)
            .map(|m| m.id)
            .expect("a semi is scheduled")
    };
    app.match_cmd(semi, MatchCommand::Start { court_id: court }).await.expect("start");
    app.record_set(semi, 21, 0).await.expect("record");

    let won = app
        .bracket_view(t_id)
        .await
        .expect("view")
        .iter()
        .filter(|n| matches!(n.kind, BracketKind::Main) && n.round == 1 && n.winner.is_some())
        .count();
    assert_eq!(won, 1, "one semi decided after playing it");

    // Reset the played semi.
    let done_id = {
        let board = app.board(t_id).await.expect("board");
        board
            .matches
            .iter()
            .find(|m| m.pool.is_none() && m.status == SchedStatus::Done)
            .map(|m| m.id)
            .expect("the played semi is done")
    };
    app.reset_bracket_match(done_id).await.expect("reset");

    // No semi is decided anymore, and both semis are pending again.
    let view = app.bracket_view(t_id).await.expect("view");
    let still_won = view
        .iter()
        .filter(|n| matches!(n.kind, BracketKind::Main) && n.round == 1 && n.winner.is_some())
        .count();
    assert_eq!(still_won, 0, "reset cleared the result");

    let board = app.board(t_id).await.expect("board");
    let pending = board
        .matches
        .iter()
        .filter(|m| m.pool.is_none() && m.status == SchedStatus::Pending)
        .count();
    assert_eq!(pending, 2, "both semis playable again after reset");

    // A pool match cannot be reset.
    let pm = MatchId::new();
    let pool = PoolId::new();
    app.match_cmd(
        pm,
        MatchCommand::Schedule {
            match_id: pm,
            tournament_id: t_id,
            format: MatchFormat::BestOf1,
            team_a: teams[0],
            team_b: teams[1],
            pool_id: Some(pool),
        },
    )
    .await
    .expect("schedule pool");
    assert!(app.reset_bracket_match(pm).await.is_err(), "pool match reset rejected");
}

/// A full bracket reset drops every finals match and the draw, returning to the
/// "not drawn" state; pool data is untouched and "Générer" can re-seed.
#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn reset_bracket_clears_the_whole_draw() {
    use domain::bracket::BracketCommand;

    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    let t_id = TournamentId::new();
    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "ResetAll".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf1,
        },
    )
    .await
    .expect("create");

    let teams = [TeamId::new(), TeamId::new(), TeamId::new(), TeamId::new()];
    for (i, id) in teams.iter().enumerate() {
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
    app.bracket(
        t_id,
        BracketCommand::Draw { main_seeds: teams.to_vec(), consolation_seeds: vec![] },
    )
    .await
    .expect("draw");
    app.advance_bracket(t_id).await.expect("advance");
    assert!(!app.bracket_view(t_id).await.expect("view").is_empty(), "bracket drawn");

    app.reset_bracket(t_id).await.expect("reset bracket");

    assert!(app.bracket_view(t_id).await.expect("view").is_empty(), "bracket cleared");
    let finals = app
        .board(t_id)
        .await
        .expect("board")
        .matches
        .iter()
        .filter(|m| m.pool.is_none())
        .count();
    assert_eq!(finals, 0, "no finals matches remain");

    // Re-draw works after a reset.
    app.bracket(
        t_id,
        BracketCommand::Draw { main_seeds: teams.to_vec(), consolation_seeds: vec![] },
    )
    .await
    .expect("redraw after reset");
}

/// Per-round bracket format: setting the final (size 2) to best-of-3 makes a
/// final need two sets — one recorded set leaves it in progress.
#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn per_round_format_makes_the_final_best_of_3() {
    use domain::bracket::BracketCommand;

    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    let t_id = TournamentId::new();
    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "RoundFmt".into(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf1, // default 1 set
        },
    )
    .await
    .expect("create");

    let (a, b) = (TeamId::new(), TeamId::new());
    for (i, id) in [a, b].iter().enumerate() {
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
    let court = CourtId::new();
    app.tournament(t_id, TournamentCommand::ConfigureCourts { courts: vec![court] })
        .await
        .expect("courts");

    // Final (2 teams) is best-of-3.
    app.tournament(
        t_id,
        TournamentCommand::SetBracketRoundFormat { round_size: 2, format: MatchFormat::BestOf3 },
    )
    .await
    .expect("set round format");

    app.bracket(
        t_id,
        BracketCommand::Draw { main_seeds: vec![a, b], consolation_seeds: vec![] },
    )
    .await
    .expect("draw");
    app.advance_bracket(t_id).await.expect("advance");

    let final_id = {
        let board = app.board(t_id).await.expect("board");
        board
            .matches
            .iter()
            .find(|m| m.pool.is_none() && m.status == SchedStatus::Pending)
            .map(|m| m.id)
            .expect("final scheduled")
    };
    app.match_cmd(final_id, MatchCommand::Start { court_id: court }).await.expect("start");
    app.record_set(final_id, 21, 0).await.expect("set 1");

    // best-of-3 → one set is not enough; still playing.
    let still_playing = app
        .board(t_id)
        .await
        .expect("board")
        .matches
        .iter()
        .any(|m| m.id == final_id && m.status == SchedStatus::Playing);
    assert!(still_playing, "BO3 final still in progress after one set");

    app.record_set(final_id, 21, 0).await.expect("set 2");
    let done = app
        .board(t_id)
        .await
        .expect("board")
        .matches
        .iter()
        .any(|m| m.id == final_id && m.status == SchedStatus::Done);
    assert!(done, "BO3 final completes after the second set");
}

#[tokio::test]
#[ignore = "requires a running PostgreSQL (set DATABASE_URL)"]
async fn forfeit_team_concedes_pending_matches_and_badges_the_team() {
    let app = App::connect(&database_url()).await;
    app.run_migrations().await.expect("migrations");

    let t_id = TournamentId::new();
    let (a, b) = (TeamId::new(), TeamId::new());

    app.tournament(
        t_id,
        TournamentCommand::Create {
            tournament_id: t_id,
            name: "Forfeit Open".into(),
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

    app.tournament(
        t_id,
        TournamentCommand::ConfigureCourts {
            courts: vec![CourtId::new()],
        },
    )
    .await
    .expect("courts");

    app.tournament(t_id, TournamentCommand::StartPoolPhase)
        .await
        .expect("start pool phase");

    // A pending pool match A vs B.
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

    // Team A forfeits: its pending match is conceded to B, A is badged.
    app.forfeit_team(t_id, a).await.expect("forfeit");

    let board = app.board(t_id).await.expect("board");
    let m = board
        .matches
        .iter()
        .find(|m| m.id == m_id)
        .expect("match present");
    assert_eq!(m.status, SchedStatus::Done, "conceded match is done");

    let view = app
        .tournament_view(t_id)
        .await
        .expect("view")
        .expect("exists");
    let team_a = view.teams.iter().find(|t| t.id == a).expect("team A");
    assert!(team_a.forfeited, "forfeited team is badged");
    let team_b = view.teams.iter().find(|t| t.id == b).expect("team B");
    assert!(!team_b.forfeited, "opponent is not badged");
}
