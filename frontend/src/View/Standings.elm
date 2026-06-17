module View.Standings exposing (viewStandings)

import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (on, onClick, onInput, preventDefaultOn)
import Json.Decode as D
import Time
import Helpers exposing (..)
import Types exposing (..)


viewStandings : Sel -> Html Msg
viewStandings s =
    if List.isEmpty s.standings then
        text ""

    else
        div [ class "panel" ]
            (h2 [] [ text "Classement" ]
                :: List.map standingsTable s.standings
            )


standingsTable : PoolStandings -> Html Msg
standingsTable ps =
    div []
        [ h3 [ class "muted" ] [ text ps.name ]
        , table []
            (tr []
                [ th [] [ text "#" ]
                , th [] [ text "Équipe" ]
                , th [] [ text "J" ]
                , th [] [ text "V" ]
                , th [] [ text "Pts+" ]
                , th [] [ text "Pts-" ]
                , th [] [ text "Diff" ]
                ]
                :: List.map standingsRow ps.rows
            )
        ]


standingsRow : StandingRow -> Html Msg
standingsRow r =
    tr []
        [ td [] [ text (String.fromInt r.rank) ]
        , td [] [ text r.name ]
        , td [] [ text (String.fromInt r.played) ]
        , td [] [ text (String.fromInt r.wins) ]
        , td [] [ text (String.fromInt r.pf) ]
        , td [] [ text (String.fromInt r.pa) ]
        , td [] [ text (String.fromInt r.diff) ]
        ]


