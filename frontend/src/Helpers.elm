module Helpers exposing (..)

import Dict exposing (Dict)
import Http
import Types exposing (..)


stepFromString : String -> Step
stepFromString s =
    case s of
        "poules" ->
            StepPools

        "terrains" ->
            StepBoard

        "previsionnel" ->
            StepSchedule

        "finales" ->
            StepFinals

        "classement" ->
            StepRanking

        _ ->
            StepTeams


{-| Parse a score input: empty means 0 (e.g. a 21-0 win). -}
parseScore : String -> Maybe Int
parseScore s =
    if String.trim s == "" then
        Just 0

    else
        String.toInt (String.trim s)


{-| Suggested pool count: aim for pools of about 6 teams. -}
suggestPools : Int -> Int
suggestPools teams =
    Basics.max 1 ((teams + 5) // 6)


{-| Distribute teams round-robin into `n` balanced pools (sizes differ by ≤1). -}
buildPools : Int -> List Team -> List ( String, List String )
buildPools n teams =
    let
        indexed =
            List.indexedMap Tuple.pair teams
    in
    List.range 0 (n - 1)
        |> List.map
            (\k ->
                let
                    members =
                        indexed
                            |> List.filter (\( i, _ ) -> modBy n i == k)
                            |> List.map (\( _, t ) -> t.id)
                in
                ( "Poule " ++ String.fromChar (Char.fromCode (65 + k)), members )
            )


teamNames : List Team -> Dict String String
teamNames teams =
    Dict.fromList (List.map (\t -> ( t.id, t.name )) teams)


nameOf : Dict String String -> String -> String
nameOf names id =
    Dict.get id names |> Maybe.withDefault (String.left 4 id)


matchLabel : Dict String String -> MatchV -> String
matchLabel names m =
    nameOf names m.teamA ++ " vs " ++ nameOf names m.teamB


findMatch : List MatchV -> String -> Maybe MatchV
findMatch matches id =
    List.filter (\m -> m.id == id) matches |> List.head


httpErr : Http.Error -> String
httpErr e =
    case e of
        Http.BadStatus code ->
            "Erreur serveur (" ++ String.fromInt code ++ ")"

        Http.BadBody b ->
            "Réponse invalide : " ++ b

        Http.NetworkError ->
            "Erreur réseau (backend démarré ?)"

        Http.Timeout ->
            "Délai dépassé"

        Http.BadUrl u ->
            "URL invalide : " ++ u
