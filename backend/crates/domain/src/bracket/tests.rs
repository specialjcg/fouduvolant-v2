    use super::*;

    fn team(n: u128) -> TeamId {
        TeamId(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn preliminary_feeds_point_to_the_match_holding_their_winner() {
        // 6 seeds → size 4, extra 2, direct 2: two barrages, their winners fill
        // the remaining round-1 slots. Each barrage's `feeds` must name the
        // round-1 match its winner actually plays in.
        let t: Vec<TeamId> = (1..=6).map(team).collect();
        // Decide each barrage: seeds[2]>seeds[5], seeds[3]>seeds[4].
        let results: Vec<Result3> = vec![(t[2], t[5], t[2]), (t[3], t[4], t[3])];
        let nodes = build_bracket(&t, &[], &results);

        let prelims: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 0)
            .collect();
        assert_eq!(prelims.len(), 2, "two preliminary matches");

        for p in prelims {
            let feeds = p.feeds.expect("a preliminary node carries a feeds index");
            let target = nodes
                .iter()
                .find(|n| n.kind == BracketKind::Main && n.round == 1 && n.index == feeds)
                .expect("feeds points at an existing round-1 match");
            let w = p.winner.expect("barrage decided");
            assert!(
                target.team_a == Some(w) || target.team_b == Some(w),
                "barrage winner must appear in the round-1 match it feeds"
            );
        }
    }

    #[test]
    fn bracket_size_floors_to_power_of_two() {
        assert_eq!(bracket_size(2), 2);
        assert_eq!(bracket_size(3), 2);
        assert_eq!(bracket_size(4), 4);
        assert_eq!(bracket_size(5), 4);
        assert_eq!(bracket_size(8), 8);
        assert_eq!(bracket_size(9), 8);
        assert_eq!(bracket_size(12), 8);
        assert_eq!(bracket_size(16), 16);
    }

    #[test]
    fn seed_slots_keeps_top_seeds_apart() {
        assert_eq!(seed_slots(2), vec![1, 2]);
        assert_eq!(seed_slots(4), vec![1, 4, 2, 3]);
        assert_eq!(seed_slots(8), vec![1, 8, 4, 5, 2, 7, 3, 6]);
    }

    #[test]
    fn four_seeds_pair_one_vs_four() {
        let t: Vec<TeamId> = (1..=4).map(team).collect();
        let nodes = build_bracket(&t, &[], &[]);
        let r1: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 1)
            .collect();
        assert_eq!(r1.len(), 2);
        // slot order [1,4,2,3] → matches (1v4) and (2v3)
        assert_eq!((r1[0].team_a, r1[0].team_b), (Some(t[0]), Some(t[3])));
        assert_eq!((r1[1].team_a, r1[1].team_b), (Some(t[1]), Some(t[2])));
    }

    #[test]
    fn three_seeds_use_a_preliminary_round() {
        // 3 seeds → floor size 2, extra 1, direct 1. Seeds 2 and 3 play a
        // preliminary; its winner meets seed 1 in the final. No byes.
        let t: Vec<TeamId> = (1..=3).map(team).collect();
        let nodes = build_bracket(&t, &[], &[]);
        let prelim: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 0)
            .collect();
        assert_eq!(prelim.len(), 1);
        assert_eq!((prelim[0].team_a, prelim[0].team_b), (Some(t[1]), Some(t[2])));

        // Final exists but its play-in side is unknown (not a bye → undecided).
        let final_node = nodes
            .iter()
            .find(|n| n.kind == BracketKind::Main && n.round == 1)
            .unwrap();
        assert!(final_node.team_a == Some(t[0]) || final_node.team_b == Some(t[0]));
        assert_eq!(final_node.winner, None);

        // Play the preliminary: seed 2 wins → it fills the final slot.
        let nodes2 = build_bracket(&t, &[], &[(t[1], t[2], t[1])]);
        let final2 = nodes2
            .iter()
            .find(|n| n.kind == BracketKind::Main && n.round == 1)
            .unwrap();
        let teams = [final2.team_a, final2.team_b];
        assert!(teams.contains(&Some(t[0])) && teams.contains(&Some(t[1])));
    }

    #[test]
    fn five_seeds_one_barrage() {
        // 5 → size 4, extra 1, direct 3. Seeds 4 and 5 play the barrage.
        let t: Vec<TeamId> = (1..=5).map(team).collect();
        let nodes = build_bracket(&t, &[], &[]);
        let prelim: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 0)
            .collect();
        assert_eq!(prelim.len(), 1);
        assert_eq!((prelim[0].team_a, prelim[0].team_b), (Some(t[3]), Some(t[4])));
        let r1 = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Main && n.round == 1)
            .count();
        assert_eq!(r1, 2, "size-4 bracket has two first-round matches");
    }

    #[test]
    fn advancement_fills_final_from_results() {
        let t: Vec<TeamId> = (1..=4).map(team).collect();
        // 1 beats 4, 2 beats 3 → final 1 vs 2
        let results = vec![(t[0], t[3], t[0]), (t[1], t[2], t[1])];
        let nodes = build_bracket(&t, &[], &results);
        let final_node = nodes
            .iter()
            .find(|n| n.kind == BracketKind::Main && n.round == 2)
            .unwrap();
        assert_eq!((final_node.team_a, final_node.team_b), (Some(t[0]), Some(t[1])));
    }

    #[test]
    fn consolation_is_its_own_bracket_of_non_qualified() {
        let main: Vec<TeamId> = (1..=4).map(team).collect();
        let cons: Vec<TeamId> = (5..=8).map(team).collect();
        let nodes = build_bracket(&main, &cons, &[]);
        let cons_r1: Vec<&BracketNode> = nodes
            .iter()
            .filter(|n| n.kind == BracketKind::Consolation && n.round == 1)
            .collect();
        assert_eq!(cons_r1.len(), 2, "4 non-qualified → 2 first-round matches");
        // consolation seeded independently: (5 v 8), (6 v 7)
        assert_eq!(
            (cons_r1[0].team_a, cons_r1[0].team_b),
            (Some(cons[0]), Some(cons[3]))
        );
    }

    #[test]
    fn third_place_only_for_eight_plus() {
        let four: Vec<TeamId> = (1..=4).map(team).collect();
        assert_eq!(
            build_bracket(&four, &[], &[])
                .iter()
                .filter(|n| n.round == THIRD_PLACE_ROUND)
                .count(),
            0
        );
        let eight: Vec<TeamId> = (1..=8).map(team).collect();
        assert_eq!(
            build_bracket(&eight, &[], &[])
                .iter()
                .filter(|n| n.round == THIRD_PLACE_ROUND)
                .count(),
            1
        );
    }

    #[test]
    fn third_place_holds_semifinal_losers() {
        let t: Vec<TeamId> = (1..=8).map(team).collect();
        // Lower seed index always wins.
        let results = vec![
            (t[0], t[7], t[0]),
            (t[3], t[4], t[3]),
            (t[1], t[6], t[1]),
            (t[2], t[5], t[2]),
            (t[0], t[3], t[0]),
            (t[1], t[2], t[1]),
        ];
        let nodes = build_bracket(&t, &[], &results);
        let third = nodes
            .iter()
            .find(|n| n.round == THIRD_PLACE_ROUND)
            .unwrap();
        let teams = [third.team_a, third.team_b];
        assert!(teams.contains(&Some(t[3])) && teams.contains(&Some(t[2])));
    }

    #[test]
    fn no_consolation_when_everyone_qualifies() {
        let main: Vec<TeamId> = (1..=4).map(team).collect();
        let nodes = build_bracket(&main, &[], &[]);
        assert!(nodes.iter().all(|n| n.kind == BracketKind::Main));
    }

    #[test]
    fn reseed_breaks_same_pool_first_round_pairs() {
        use std::collections::HashMap;
        // Seeds [a,b,c,d]; (i,n-1-i) pairs (a,d) and (b,c). a&d in pool 1, b&c
        // in pool 2 → both pairs same-pool. Reseed must separate them.
        let (a, b, c, d) = (team(1), team(2), team(3), team(4));
        let mut seeds = vec![a, b, c, d];
        let pools: HashMap<TeamId, usize> =
            [(a, 1), (d, 1), (b, 2), (c, 2)].into_iter().collect();
        reseed_pool_separation(&mut seeds, &pools);
        let pool = |t: TeamId| pools[&t];
        let n = seeds.len();
        for i in 0..n / 2 {
            assert_ne!(
                pool(seeds[i]),
                pool(seeds[n - 1 - i]),
                "pair {i} still same-pool"
            );
        }
    }

    #[tokio::test]
    async fn cannot_draw_twice() {
        let mut b = Bracket::default();
        b.apply(BracketEvent::Drawn {
            main_seeds: vec![team(1), team(2)],
            consolation_seeds: vec![],
        });
        let sink = EventSink::default();
        let err = b
            .handle(
                BracketCommand::Draw {
                    main_seeds: vec![team(1), team(2)],
                    consolation_seeds: vec![],
                },
                &(),
                &sink,
            )
            .await
            .unwrap_err();
        assert_eq!(err, BracketError::AlreadyDrawn);
    }
