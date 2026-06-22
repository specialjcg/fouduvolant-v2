module Decoders exposing (..)

import Dict
import Json.Decode as D
import Types exposing (..)


summaryDec : D.Decoder Summary
summaryDec =
    D.map3 Summary (D.field "id" D.string) (D.field "name" D.string) (D.field "phase" D.string)


teamDec : D.Decoder Team
teamDec =
    D.map5 Team
        (D.field "id" D.string)
        (D.field "name" D.string)
        (D.field "player1" D.string)
        (D.field "player2" D.string)
        (D.oneOf [ D.field "forfeited" D.bool, D.succeed False ])


tviewDec : D.Decoder TView
tviewDec =
    D.succeed TView
        |> andMap (D.field "id" D.string)
        |> andMap (D.field "name" D.string)
        |> andMap (D.field "phase" D.string)
        |> andMap (D.field "teams" (D.list teamDec))
        |> andMap (D.field "pools" (D.list poolDec))
        |> andMap (D.field "courts" (D.list D.string))
        |> andMap (D.field "pool_courts" (D.list (D.map2 PoolCourt (D.field "pool" D.string) (D.field "court" D.string))))
        |> andMap (D.oneOf [ D.field "bracket_format" D.string, D.succeed "BestOf1" ])
        |> andMap (D.oneOf [ D.field "bracket_round_formats" (D.dict D.string), D.succeed Dict.empty ])


poolDec : D.Decoder PoolV
poolDec =
    D.map3 PoolV
        (D.field "id" D.string)
        (D.field "name" D.string)
        (D.field "teams" (D.list D.string))


boardDec : D.Decoder Board
boardDec =
    D.map2 Board
        (D.field "courts" (D.list courtPlanDec))
        (D.field "matches" (D.list matchVDec))


courtPlanDec : D.Decoder CourtPlan
courtPlanDec =
    D.map4 CourtPlan
        (D.field "court" D.string)
        (D.field "current" (D.nullable D.string))
        (D.field "next" (D.nullable suggDec))
        (D.field "previews" (D.list suggDec))


suggDec : D.Decoder Sugg
suggDec =
    D.map2 Sugg (D.field "match_id" D.string) (D.field "needs_rest" D.bool)


bracketNodeDec : D.Decoder BracketNode
bracketNodeDec =
    D.map7 BracketNode
        (D.field "kind" D.string)
        (D.field "round" D.int)
        (D.field "index" D.int)
        (D.field "team_a" (D.nullable D.string))
        (D.field "team_b" (D.nullable D.string))
        (D.field "winner" (D.nullable D.string))
        (D.field "feeds" (D.nullable D.int))


forecastCourtDec : D.Decoder ForecastCourt
forecastCourtDec =
    D.map2 ForecastCourt
        (D.field "court" D.string)
        (D.field "matches" (D.list forecastMatchDec))


forecastMatchDec : D.Decoder ForecastMatch
forecastMatchDec =
    D.map8 ForecastMatch
        (D.field "id" D.string)
        (D.field "team_a" D.string)
        (D.field "team_b" D.string)
        (D.field "pool" (D.nullable D.string))
        (D.field "status" D.string)
        (D.field "points_a" D.int)
        (D.field "points_b" D.int)
        (D.field "eta_min" D.int)


poolStandingsDec : D.Decoder PoolStandings
poolStandingsDec =
    D.map3 PoolStandings
        (D.field "pool_id" D.string)
        (D.field "name" D.string)
        (D.field "rows" (D.list rowDec))


rowDec : D.Decoder StandingRow
rowDec =
    D.map7 StandingRow
        (D.field "name" D.string)
        (D.field "rank" D.int)
        (D.field "played" D.int)
        (D.field "wins" D.int)
        (D.field "points_for" D.int)
        (D.field "points_against" D.int)
        (D.field "diff" D.int)


andMap : D.Decoder a -> D.Decoder (a -> b) -> D.Decoder b
andMap =
    D.map2 (|>)


matchVDec : D.Decoder MatchV
matchVDec =
    D.succeed MatchV
        |> andMap (D.field "id" D.string)
        |> andMap (D.field "team_a" D.string)
        |> andMap (D.field "team_b" D.string)
        |> andMap (D.field "status" D.string)
        |> andMap (D.field "court" (D.nullable D.string))
        |> andMap (D.field "done_order" (D.nullable D.int))
        |> andMap (D.field "points_a" D.int)
        |> andMap (D.field "points_b" D.int)
        |> andMap (D.field "pool" (D.nullable D.string))
        |> andMap (D.field "sets" (D.list (D.map2 Tuple.pair (D.index 0 D.int) (D.index 1 D.int))))
        |> andMap (D.oneOf [ D.field "conceded" D.bool, D.succeed False ])
        |> andMap (D.oneOf [ D.field "irregular" D.bool, D.succeed False ])
