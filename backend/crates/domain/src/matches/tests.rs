    use super::*;

    /// Run a command, returning the events it emitted. The aggregate is mutated
    /// in place (events are applied as they are written).
    async fn exec(m: &mut Match, cmd: MatchCommand) -> Result<Vec<MatchEvent>, MatchError> {
        let sink = EventSink::default();
        m.handle(cmd, &(), &sink).await?;
        Ok(sink.collect().await)
    }

    fn scheduled(format: MatchFormat) -> (Match, TeamId, TeamId) {
        let (a, b) = (TeamId::new(), TeamId::new());
        let mut m = Match::default();
        m.apply(MatchEvent::Scheduled {
            match_id: MatchId::new(),
            tournament_id: TournamentId::new(),
            format,
            team_a: a,
            team_b: b,
            pool_id: None,
        });
        (m, a, b)
    }

    fn started(format: MatchFormat) -> (Match, TeamId, TeamId) {
        let (mut m, a, b) = scheduled(format);
        m.apply(MatchEvent::MatchStarted {
            court_id: CourtId::new(),
        });
        (m, a, b)
    }

    #[tokio::test]
    async fn concede_completes_with_named_winner() {
        // No-show before start: conceding a scheduled match is allowed.
        let (mut m, a, b) = scheduled(MatchFormat::BestOf1);
        let events = exec(&mut m, MatchCommand::Concede { winner: a }).await.unwrap();
        assert_eq!(events, vec![MatchEvent::Conceded { winner: a }]);
        assert_eq!(m.status, MatchStatus::Completed);
        assert_eq!(m.winner, Some(a));

        // A non-participant cannot be the winner.
        let (mut m2, _, _) = started(MatchFormat::BestOf3);
        let outsider = TeamId::new();
        assert!(matches!(
            exec(&mut m2, MatchCommand::Concede { winner: outsider }).await,
            Err(MatchError::UnknownWinner)
        ));
        let _ = b;
    }

    #[tokio::test]
    async fn cannot_concede_a_completed_match() {
        let (mut m, a, _b) = started(MatchFormat::BestOf1);
        exec(&mut m, MatchCommand::RecordSet { a: 21, b: 0 }).await.unwrap();
        assert!(matches!(
            exec(&mut m, MatchCommand::Concede { winner: a }).await,
            Err(MatchError::AlreadyCompleted)
        ));
    }

    #[tokio::test]
    async fn cannot_schedule_twice() {
        let (mut m, a, b) = scheduled(MatchFormat::BestOf1);
        let err = exec(
            &mut m,
            MatchCommand::Schedule {
                match_id: MatchId::new(),
                tournament_id: TournamentId::new(),
                format: MatchFormat::BestOf1,
                team_a: a,
                team_b: b,
                pool_id: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, MatchError::AlreadyScheduled);
    }

    #[tokio::test]
    async fn cannot_score_before_schedule() {
        let mut m = Match::default();
        let err = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 10 })
            .await
            .unwrap_err();
        assert_eq!(err, MatchError::NotScheduled);
    }

    #[tokio::test]
    async fn cannot_score_before_start() {
        let (mut m, _a, _b) = scheduled(MatchFormat::BestOf1);
        let err = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 10 })
            .await
            .unwrap_err();
        assert_eq!(err, MatchError::NotStarted);
    }

    #[tokio::test]
    async fn start_then_cannot_start_again() {
        let (mut m, _a, _b) = scheduled(MatchFormat::BestOf1);
        let ev = exec(
            &mut m,
            MatchCommand::Start {
                court_id: CourtId::new(),
            },
        )
        .await
        .unwrap();
        assert_eq!(ev.len(), 1);
        let err = exec(
            &mut m,
            MatchCommand::Start {
                court_id: CourtId::new(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, MatchError::AlreadyStarted);
    }

    #[tokio::test]
    async fn bo1_completes_after_one_set() {
        let (mut m, a, _b) = started(MatchFormat::BestOf1);
        let events = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 15 })
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1], MatchEvent::Completed { winner: a });
    }

    #[tokio::test]
    async fn bo3_needs_two_sets() {
        let (mut m, a, _b) = started(MatchFormat::BestOf3);
        let ev = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 18 })
            .await
            .unwrap();
        assert_eq!(ev.len(), 1);
        let ev = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 12 })
            .await
            .unwrap();
        assert_eq!(ev.last(), Some(&MatchEvent::Completed { winner: a }));
    }

    #[tokio::test]
    async fn cannot_score_completed_match() {
        let (mut m, _a, _b) = started(MatchFormat::BestOf1);
        exec(&mut m, MatchCommand::RecordSet { a: 21, b: 0 })
            .await
            .unwrap();
        let err = exec(&mut m, MatchCommand::RecordSet { a: 21, b: 0 })
            .await
            .unwrap_err();
        assert_eq!(err, MatchError::AlreadyCompleted);
    }

    #[tokio::test]
    async fn unstart_returns_a_live_match_to_scheduled() {
        let (mut m, _a, _b) = started(MatchFormat::BestOf1);
        let events = exec(&mut m, MatchCommand::Unstart).await.unwrap();
        assert_eq!(events, vec![MatchEvent::MatchUnstarted]);
        assert_eq!(m.status, MatchStatus::Scheduled);
    }

    #[tokio::test]
    async fn unstart_before_start_is_rejected() {
        let (mut m, _a, _b) = scheduled(MatchFormat::BestOf1);
        let err = exec(&mut m, MatchCommand::Unstart).await.unwrap_err();
        assert_eq!(err, MatchError::NotStarted);
    }

    #[tokio::test]
    async fn unstart_a_completed_match_is_rejected() {
        let (mut m, _a, _b) = started(MatchFormat::BestOf1);
        exec(&mut m, MatchCommand::RecordSet { a: 21, b: 0 })
            .await
            .unwrap();
        let err = exec(&mut m, MatchCommand::Unstart).await.unwrap_err();
        assert_eq!(err, MatchError::AlreadyCompleted);
    }

    #[tokio::test]
    async fn force_score_completes_with_the_higher_side_winning() {
        let (mut m, a, _b) = started(MatchFormat::BestOf1);
        let events = exec(&mut m, MatchCommand::ForceScore { a: 15, b: 10 })
            .await
            .unwrap();
        assert_eq!(
            events,
            vec![MatchEvent::ScoreForced { a: 15, b: 10, winner: a }]
        );
        assert_eq!(m.status, MatchStatus::Completed);
        assert_eq!(m.winner, Some(a));
    }

    #[tokio::test]
    async fn force_score_rejects_a_tie() {
        let (mut m, _a, _b) = started(MatchFormat::BestOf1);
        let err = exec(&mut m, MatchCommand::ForceScore { a: 15, b: 15 })
            .await
            .unwrap_err();
        assert_eq!(err, MatchError::TiedScore { a: 15, b: 15 });
    }

    #[tokio::test]
    async fn force_score_before_start_is_rejected() {
        let (mut m, _a, _b) = scheduled(MatchFormat::BestOf1);
        let err = exec(&mut m, MatchCommand::ForceScore { a: 15, b: 10 })
            .await
            .unwrap_err();
        assert_eq!(err, MatchError::NotStarted);
    }
