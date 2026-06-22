    use super::*;

    fn court(n: u128) -> CourtId {
        CourtId(uuid::Uuid::from_u128(n))
    }
    fn pool(n: u128) -> PoolId {
        PoolId(uuid::Uuid::from_u128(n))
    }
    fn team(n: u128) -> TeamId {
        TeamId(uuid::Uuid::from_u128(n))
    }

    fn tourney() -> TournamentId {
        TournamentId(uuid::Uuid::from_u128(999))
    }

    fn pending_match(seq: u32, p: PoolId, a: TeamId, b: TeamId) -> MatchView {
        MatchView {
            id: MatchId::new(),
            tournament: tourney(),
            seq,
            pool: Some(p),
            team_a: a,
            team_b: b,
            status: SchedStatus::Pending,
            court: None,
            manual_court: None,
            done_order: None,
            winner: None,
            points_a: 0,
            points_b: 0,
            sets: Vec::new(),
            conceded: false,
        }
    }

    #[test]
    fn one_pool_per_court_when_enough_courts() {
        let (p1, p2) = (pool(1), pool(2));
        let matches = vec![
            pending_match(0, p1, team(10), team(11)),
            pending_match(1, p1, team(12), team(13)),
            pending_match(2, p2, team(20), team(21)),
            pending_match(3, p2, team(22), team(23)),
        ];
        let courts = vec![court(1), court(2)];
        let map = assign_pools_to_courts(&matches, &courts);
        assert_eq!(map.len(), 2);
        assert_ne!(map[&p1], map[&p2], "distinct pools get distinct courts");
    }

    #[test]
    fn overflow_pools_share_lightest_courts() {
        // 3 pools, 2 courts. Biggest pool alone, the two smaller share.
        let (p1, p2, p3) = (pool(1), pool(2), pool(3));
        let mut matches = vec![
            pending_match(0, p1, team(10), team(11)),
            pending_match(1, p1, team(12), team(13)),
            pending_match(2, p1, team(14), team(15)),
        ];
        matches.push(pending_match(3, p2, team(20), team(21)));
        matches.push(pending_match(4, p3, team(30), team(31)));
        let courts = vec![court(1), court(2)];
        let map = assign_pools_to_courts(&matches, &courts);
        // p1 (3 matches) lands alone; p2 and p3 land on the other court.
        assert_eq!(map[&p2], map[&p3]);
        assert_ne!(map[&p1], map[&p2]);
    }

    #[test]
    fn anti_btb_prefers_rested_team() {
        // Court just finished a match with teams 10/11. The next suggestion on a
        // free court must avoid both, even though their pool is least complete.
        let p = pool(1);
        let courts = vec![court(1)];
        let just_done = MatchView {
            id: MatchId::new(),
            tournament: tourney(),
            seq: 0,
            pool: Some(p),
            team_a: team(10),
            team_b: team(11),
            status: SchedStatus::Done,
            court: Some(court(1)),
            manual_court: None,
            done_order: Some(1),
            winner: Some(team(10)),
            points_a: 21,
            points_b: 11,
            sets: vec![(21, 11)],
            conceded: false,
        };
        let btb = pending_match(1, p, team(10), team(20)); // reuses team 10
        let fresh = pending_match(2, p, team(30), team(31));
        let matches = vec![just_done, btb.clone(), fresh.clone()];
        let plans = plan(&matches, &courts, &HashMap::new());
        let next = plans[0].next.as_ref().unwrap();
        assert_eq!(next.match_id, fresh.id);
        assert!(!next.needs_rest);
    }

    #[test]
    fn anti_btb_relaxes_when_unavoidable() {
        // Only one pending match and it reuses a team that just played.
        let p = pool(1);
        let courts = vec![court(1)];
        let just_done = MatchView {
            id: MatchId::new(),
            tournament: tourney(),
            seq: 0,
            pool: Some(p),
            team_a: team(10),
            team_b: team(11),
            status: SchedStatus::Done,
            court: Some(court(1)),
            manual_court: None,
            done_order: Some(1),
            winner: Some(team(10)),
            points_a: 21,
            points_b: 11,
            sets: vec![(21, 11)],
            conceded: false,
        };
        let only = pending_match(1, p, team(10), team(20));
        let matches = vec![just_done, only.clone()];
        let plans = plan(&matches, &courts, &HashMap::new());
        let next = plans[0].next.as_ref().unwrap();
        assert_eq!(next.match_id, only.id);
        assert!(next.needs_rest, "unavoidable btb is flagged");
    }

    #[test]
    fn weave_spreads_small_pool_across_time() {
        // Court hosts a big pool (p1: 4 matches) and a small one (p2: 2). Across
        // the next+previews (3 slots) the small pool must not be exhausted first;
        // a balanced weave interleaves them.
        let (p1, p2) = (pool(1), pool(2));
        let courts = vec![court(1)];
        let matches = vec![
            pending_match(0, p1, team(10), team(11)),
            pending_match(1, p1, team(12), team(13)),
            pending_match(2, p1, team(14), team(15)),
            pending_match(3, p1, team(16), team(17)),
            pending_match(4, p2, team(20), team(21)),
            pending_match(5, p2, team(22), team(23)),
        ];
        let mut map = HashMap::new();
        map.insert(p1, court(1));
        map.insert(p2, court(1));
        let plans = plan(&matches, &courts, &map);
        let chosen: Vec<Option<PoolId>> = std::iter::once(plans[0].next.as_ref().unwrap().match_id)
            .chain(plans[0].previews.iter().map(|s| s.match_id))
            .map(|id| matches.iter().find(|m| m.id == id).unwrap().pool)
            .collect();
        // Least-complete-pool-first picks p2 at least once within the first 3.
        assert!(chosen.contains(&Some(p2)), "small pool woven in early: {chosen:?}");
    }

    #[test]
    fn manual_move_targets_its_court_without_stealing() {
        // Explicit map: pool 1 → court 1. A pool-1 match is manually pinned to
        // court 2 (which owns no pool). Court 2 must take it; court 1 must not.
        let p1 = pool(1);
        let courts = vec![court(1), court(2)];
        let mut map = HashMap::new();
        map.insert(p1, court(1));

        let normal = pending_match(0, p1, team(10), team(11));
        let mut pinned = pending_match(1, p1, team(12), team(13));
        pinned.manual_court = Some(court(2));
        let matches = vec![normal.clone(), pinned.clone()];

        let plans = plan(&matches, &courts, &map);
        let c1 = plans.iter().find(|p| p.court == court(1)).unwrap();
        let c2 = plans.iter().find(|p| p.court == court(2)).unwrap();
        assert_eq!(c1.next.as_ref().unwrap().match_id, normal.id);
        assert_eq!(c2.next.as_ref().unwrap().match_id, pinned.id);
    }

    #[test]
    fn idle_court_with_explicit_map_makes_no_suggestion() {
        // Court 2 owns no pool, has no pin and no history → no suggestion.
        let p1 = pool(1);
        let courts = vec![court(1), court(2)];
        let mut map = HashMap::new();
        map.insert(p1, court(1));
        let matches = vec![pending_match(0, p1, team(10), team(11))];
        let plans = plan(&matches, &courts, &map);
        let c2 = plans.iter().find(|p| p.court == court(2)).unwrap();
        assert!(c2.next.is_none());
    }

    #[test]
    fn a_team_is_never_suggested_on_two_courts_at_once() {
        // m1 and m2 share team X; m3 is independent. With two free courts, the
        // two simultaneous `next` picks must never both involve X.
        let p1 = pool(1);
        let (x, y, z, w, v) = (team(1), team(2), team(3), team(4), team(5));
        let m1 = pending_match(0, p1, x, y);
        let m2 = pending_match(1, p1, x, z);
        let m3 = pending_match(2, p1, w, v);
        let matches = vec![m1.clone(), m2.clone(), m3.clone()];
        let courts = vec![court(1), court(2)];
        let map = HashMap::new(); // no explicit map: any court serves any match

        let plans = plan(&matches, &courts, &map);
        let next_ids: Vec<MatchId> = plans
            .iter()
            .filter_map(|p| p.next.as_ref().map(|s| s.match_id))
            .collect();

        // Both free courts get a suggestion (m1 + m3 are conflict-free).
        assert_eq!(next_ids.len(), 2, "both free courts get a suggestion");

        let teams_of = |id: MatchId| {
            let m = matches.iter().find(|m| m.id == id).unwrap();
            [m.team_a, m.team_b]
        };
        let a = teams_of(next_ids[0]);
        let b = teams_of(next_ids[1]);
        assert!(
            !a.iter().any(|t| b.contains(t)),
            "no team plays two courts at once: {a:?} vs {b:?}"
        );
        // Concretely: the two X-sharing matches are never started together.
        assert!(
            !(next_ids.contains(&m1.id) && next_ids.contains(&m2.id)),
            "team X is not double-booked"
        );
    }
