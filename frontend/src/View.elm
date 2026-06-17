module View exposing (view)

import Html exposing (..)
import Html.Attributes exposing (class)
import Html.Events exposing (onClick)
import Time
import Helpers exposing (..)
import Types exposing (..)
import View.Board exposing (viewBoard)
import View.Bracket exposing (viewBracket)
import View.List exposing (viewList)
import View.Pools exposing (viewPools)
import View.Schedule exposing (viewSchedule)
import View.Standings exposing (viewStandings)
import View.Teams exposing (viewTeams)


view : Model -> Html Msg
view model =
    div []
        [ header []
            [ h1 [] [ text "🏸 Fou du ", span [ class "accent" ] [ text "Volant" ] ]
            , case model.sel of
                Just _ ->
                    button [ class "secondary", onClick CloseT ] [ text "← Tournois" ]

                Nothing ->
                    text ""
            ]
        , main_ []
            [ case model.err of
                Just e ->
                    div [ class "panel err" ] [ text e ]

                Nothing ->
                    text ""
            , case model.sel of
                Just s ->
                    viewTournament model.showPast model.now model.zone s

                Nothing ->
                    viewList model
            ]
        ]



viewTournament : Bool -> Time.Posix -> Time.Zone -> Sel -> Html Msg
viewTournament showPast now zone s =
    let
        names =
            teamNames s.view.teams

        content =
            case s.step of
                StepTeams ->
                    viewTeams s

                StepPools ->
                    viewPools s

                StepBoard ->
                    viewBoard showPast s names

                StepSchedule ->
                    viewSchedule now zone s

                StepFinals ->
                    viewBracket s

                StepRanking ->
                    viewStandings s
    in
    div []
        [ div [ class "panel" ]
            [ h2 [] [ text s.view.name, text " ", span [ class "pill" ] [ text s.view.phase ] ]
            , p [ class "muted" ]
                [ text (String.fromInt (List.length s.view.teams) ++ " équipes · ")
                , text (String.fromInt (List.length s.view.courts) ++ " terrains")
                ]
            ]
        , stepper s.step
        , content
        ]


stepper : Step -> Html Msg
stepper active =
    let
        item st label =
            button
                [ class
                    (if st == active then
                        "step active"

                     else
                        "step"
                    )
                , onClick (GoStep st)
                ]
                [ text label ]
    in
    div [ class "stepper" ]
        [ item StepTeams "1 · Équipes"
        , item StepPools "2 · Poules"
        , item StepBoard "3 · Terrains"
        , item StepSchedule "4 · Prévisionnel"
        , item StepFinals "5 · Finales"
        , item StepRanking "6 · Classement"
        ]


