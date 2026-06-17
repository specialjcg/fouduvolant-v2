module View.Pools exposing (viewPools)

import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (on, onClick, onInput, preventDefaultOn)
import Json.Decode as D
import Time
import Helpers exposing (..)
import Types exposing (..)


import View.Common exposing (shortName)


viewPools : Sel -> Html Msg
viewPools s =
    div [ class "panel" ]
        [ h2 [] [ text "Poules & terrains" ]
        , div [ class "row" ]
            [ text "Terrains :"
            , input [ type_ "number", class "score", value s.courts, onInput SetCourts ] []
            , button [ class "secondary", onClick SaveCourts ] [ text "Définir" ]
            ]
        , h3 [] [ text "Répartition" ]
        , div [ class "row" ]
            [ button [ onClick ProposeIdealPools, disabled (List.length s.view.teams < 2) ]
                [ text ("Proposer l'idéal (" ++ String.fromInt (suggestPools (List.length s.view.teams)) ++ " poules)") ]
            , span [ class "muted" ] [ text "ou" ]
            , text "Nombre de poules :"
            , input [ type_ "number", class "score", value s.numPools, onInput SetNumPools ] []
            , button [ class "secondary", onClick AutoPools, disabled (List.length s.view.teams < 2) ]
                [ text "Répartir" ]
            ]
        , if List.isEmpty s.view.pools then
            p [ class "muted" ] [ text "Aucune poule. Répartis les équipes ci-dessus." ]

          else
            let
                names =
                    teamNames s.view.teams

                assignedOf pid =
                    s.view.poolCourts
                        |> List.filter (\pc -> pc.pool == pid)
                        |> List.head
                        |> Maybe.map .court
            in
            div []
                [ if s.view.phase == "Draft" then
                    p [ class "muted", Html.Attributes.style "font-size" ".82rem" ]
                        [ text "Glisser-déposer une équipe d'une poule à l'autre pour rééquilibrer." ]

                  else
                    text ""
                , div []
                    (List.map
                        (\pp -> poolRow (s.view.phase == "Draft") names s.view.courts s.board.matches (assignedOf pp.id) (rankedPoolTeams names s.standings pp) pp)
                        s.view.pools
                    )
                ]
        , div [ class "row", Html.Attributes.style "margin-top" "1rem" ]
            [ button
                [ onClick StartPools
                , disabled (List.isEmpty s.view.pools || List.isEmpty s.view.courts)
                ]
                [ text "Lancer les poules" ]
            , button [ class "secondary", onClick (GoStep StepBoard) ] [ text "Terrains →" ]
            , button [ class "danger", onClick ResetTournament ] [ text "Réinitialiser (relancer à vide)" ]
            ]
        ]



poolRow : Bool -> Dict String String -> List String -> List MatchV -> Maybe String -> List String -> PoolV -> Html Msg
poolRow editable names courts matches assigned ranked p =
    let
        dropZone =
            if editable then
                [ preventDefaultOn "dragover" (D.succeed ( NoOp, True ))
                , preventDefaultOn "drop" (D.succeed ( DropOn p.id, True ))
                ]

            else
                []
    in
    div (class "match" :: dropZone)
        [ div [ class "row", Html.Attributes.style "justify-content" "space-between" ]
            [ span [ Html.Attributes.style "font-weight" "600" ]
                [ text p.name
                , span [ class "muted", Html.Attributes.style "font-weight" "400", Html.Attributes.style "margin-left" ".4rem" ]
                    [ text ("(" ++ String.fromInt (List.length p.teams) ++ " équipes)") ]
                ]
            , courtSelect courts assigned p.id
            ]
        , if editable then
            div [ class "row", Html.Attributes.style "flex-wrap" "wrap", Html.Attributes.style "margin-top" ".4rem" ]
                (List.map (teamChip names) p.teams)

          else
            poolMatrix names matches ranked
        ]


teamChip : Dict String String -> String -> Html Msg
teamChip names tid =
    span
        [ class "chip"
        , Html.Attributes.draggable "true"
        , on "dragstart" (D.succeed (DragStart tid))
        ]
        [ text (nameOf names tid) ]


{-| Cross table of a pool's matches (équipe × équipe), score in each cell. -}
poolMatrix : Dict String String -> List MatchV -> List String -> Html Msg
poolMatrix names matches teams =
    if List.length teams < 2 then
        text ""

    else
        table [ Html.Attributes.style "margin-top" ".5rem" ]
            (tr []
                (th [] [ text "" ]
                    :: List.map (\t -> th [] [ text (shortName (nameOf names t)) ]) teams
                    ++ [ th [] [ text "V" ], th [] [ text "D" ], th [] [ text "Pts" ], th [] [ text "Diff" ] ]
                )
                :: List.map (matrixRow names matches teams) teams
            )


{-| Pool team ids ordered by the server-computed standings (BWF tiebreakers);
falls back to pool order for teams missing from the standings. -}
rankedPoolTeams : Dict String String -> List PoolStandings -> PoolV -> List String
rankedPoolTeams names standings p =
    case List.head (List.filter (\ps -> ps.poolId == p.id) standings) of
        Just ps ->
            let
                idFor nm =
                    List.head (List.filter (\tid -> nameOf names tid == nm) p.teams)

                ordered =
                    List.filterMap (\row -> idFor row.name) ps.rows

                rest =
                    List.filter (\tid -> not (List.member tid ordered)) p.teams
            in
            ordered ++ rest

        Nothing ->
            p.teams


matrixRow : Dict String String -> List MatchV -> List String -> String -> Html Msg
matrixRow names matches teams ti =
    let
        cell n =
            td [ Html.Attributes.style "text-align" "center", Html.Attributes.style "font-weight" "600" ]
                [ text (String.fromInt n) ]

        stat =
            teamStats matches teams ti
    in
    tr []
        (td [ Html.Attributes.style "font-weight" "600" ] [ text (nameOf names ti) ]
            :: List.map
                (\tj ->
                    td [ Html.Attributes.style "text-align" "center" ]
                        [ text
                            (if ti == tj then
                                "—"

                             else
                                scoreBetween matches ti tj
                            )
                        ]
                )
                teams
            ++ [ cell stat.w
               , cell stat.l
               , cell stat.pf
               , td [ Html.Attributes.style "text-align" "center", Html.Attributes.style "font-weight" "600" ]
                    [ text (signed (stat.pf - stat.pa)) ]
               ]
        )


type alias TeamStat =
    { w : Int, l : Int, pf : Int, pa : Int }


{-| Wins / losses / points-for / points-against of `ti` over its played pool matches. -}
teamStats : List MatchV -> List String -> String -> TeamStat
teamStats matches teams ti =
    List.foldl
        (\tj acc ->
            case playedScore matches ti tj of
                Just ( mine, opp ) ->
                    { w =
                        acc.w
                            + (if mine > opp then
                                1

                               else
                                0
                              )
                    , l =
                        acc.l
                            + (if mine < opp then
                                1

                               else
                                0
                              )
                    , pf = acc.pf + mine
                    , pa = acc.pa + opp
                    }

                Nothing ->
                    acc
        )
        { w = 0, l = 0, pf = 0, pa = 0 }
        teams


{-| Points of `i` vs `j` (own, opponent) when the match is actually played. -}
playedScore : List MatchV -> String -> String -> Maybe ( Int, Int )
playedScore matches i j =
    if i == j then
        Nothing

    else
        case List.head (List.filter (\m -> ( m.teamA, m.teamB ) == ( i, j ) || ( m.teamA, m.teamB ) == ( j, i )) matches) of
            Just m ->
                if m.pointsA == 0 && m.pointsB == 0 && m.status /= "Done" then
                    Nothing

                else if m.teamA == i then
                    Just ( m.pointsA, m.pointsB )

                else
                    Just ( m.pointsB, m.pointsA )

            Nothing ->
                Nothing


signed : Int -> String
signed n =
    if n > 0 then
        "+" ++ String.fromInt n

    else
        String.fromInt n


scoreBetween : List MatchV -> String -> String -> String
scoreBetween matches i j =
    case List.head (List.filter (\m -> ( m.teamA, m.teamB ) == ( i, j ) || ( m.teamA, m.teamB ) == ( j, i )) matches) of
        Just m ->
            if m.pointsA == 0 && m.pointsB == 0 && m.status /= "Done" then
                ""

            else if m.teamA == i then
                String.fromInt m.pointsA ++ "-" ++ String.fromInt m.pointsB

            else
                String.fromInt m.pointsB ++ "-" ++ String.fromInt m.pointsA

        Nothing ->
            ""



courtSelect : List String -> Maybe String -> String -> Html Msg
courtSelect courts assigned poolId =
    Html.select [ onInput (AssignPoolCourt poolId) ]
        (option [ value "" ] [ text "— terrain —" ]
            :: List.indexedMap
                (\i c ->
                    option [ value c, Html.Attributes.selected (assigned == Just c) ]
                        [ text ("Terrain " ++ String.fromInt (i + 1)) ]
                )
                courts
        )


