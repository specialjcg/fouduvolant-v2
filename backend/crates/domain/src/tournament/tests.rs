    use super::*;

    async fn exec(
        t: &mut Tournament,
        cmd: TournamentCommand,
    ) -> Result<Vec<TournamentEvent>, TournamentError> {
        let sink = EventSink::default();
        t.handle(cmd, &(), &sink).await?;
        Ok(sink.collect().await)
    }

    fn created() -> Tournament {
        let mut t = Tournament::default();
        t.apply(TournamentEvent::Created {
            tournament_id: TournamentId::new(),
            name: "Open".to_string(),
            pool_format: MatchFormat::BestOf1,
            bracket_format: MatchFormat::BestOf3,
        });
        t
    }

    #[tokio::test]
    async fn set_bracket_format_changes_the_format() {
        let mut t = created(); // starts BestOf3
        let events = exec(
            &mut t,
            TournamentCommand::SetBracketFormat { format: MatchFormat::BestOf1 },
        )
        .await
        .expect("set format");
        assert_eq!(
            events,
            vec![TournamentEvent::BracketFormatSet { format: MatchFormat::BestOf1 }]
        );
        for e in events {
            t.apply(e);
        }
        assert_eq!(t.bracket_format, MatchFormat::BestOf1);
    }

    #[tokio::test]
    async fn cannot_create_twice() {
        let mut t = created();
        let err = exec(
            &mut t,
            TournamentCommand::Create {
                tournament_id: TournamentId::new(),
                name: "X".into(),
                pool_format: MatchFormat::BestOf1,
                bracket_format: MatchFormat::BestOf3,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, TournamentError::AlreadyCreated);
    }

    #[tokio::test]
    async fn rejects_duplicate_team() {
        let mut t = created();
        let id = TeamId::new();
        exec(
            &mut t,
            TournamentCommand::RegisterTeam {
                team_id: id,
                name: "A".into(),
                player1: "A1".into(),
                player2: "A2".into(),
            },
        )
        .await
        .unwrap();
        let err = exec(
            &mut t,
            TournamentCommand::RegisterTeam {
                team_id: id,
                name: "A".into(),
                player1: "A1".into(),
                player2: "A2".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err, TournamentError::DuplicateTeam);
    }

    #[tokio::test]
    async fn pools_must_reference_registered_teams() {
        let mut t = created();
        let err = exec(
            &mut t,
            TournamentCommand::GeneratePools {
                pools: vec![Pool {
                    id: PoolId::new(),
                    name: "P1".into(),
                    teams: vec![TeamId::new()],
                }],
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, TournamentError::InvalidPools(_)));
    }

    #[tokio::test]
    async fn start_pool_phase_requires_pools_and_courts() {
        let mut t = created();
        let err = exec(&mut t, TournamentCommand::StartPoolPhase)
            .await
            .unwrap_err();
        assert!(matches!(err, TournamentError::CannotStartPoolPhase(_)));
    }

    #[tokio::test]
    async fn full_setup_flow() {
        let mut t = created();
        let (a, b) = (TeamId::new(), TeamId::new());
        for (id, name) in [(a, "A"), (b, "B")] {
            exec(
                &mut t,
                TournamentCommand::RegisterTeam {
                    team_id: id,
                    name: name.into(),
                    player1: String::new(),
                    player2: String::new(),
                },
            )
            .await
            .unwrap();
        }
        exec(
            &mut t,
            TournamentCommand::GeneratePools {
                pools: vec![Pool {
                    id: PoolId::new(),
                    name: "P1".into(),
                    teams: vec![a, b],
                }],
            },
        )
        .await
        .unwrap();
        exec(
            &mut t,
            TournamentCommand::ConfigureCourts {
                courts: vec![CourtId::new()],
            },
        )
        .await
        .unwrap();
        exec(&mut t, TournamentCommand::StartPoolPhase)
            .await
            .unwrap();
        assert_eq!(t.phase, Phase::PoolPhase);
    }
