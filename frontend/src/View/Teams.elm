module View.Teams exposing (viewTeams)

import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (on, onClick, onInput, preventDefaultOn)
import Json.Decode as D
import Time
import Helpers exposing (..)
import Types exposing (..)


viewTeams : Sel -> Html Msg
viewTeams s =
    div [ class "panel" ]
        [ h2 [] [ text "Équipes" ]
        , div [ class "row" ]
            [ input [ placeholder "Nom d'équipe", value s.newTeamName, onInput SetNewTeamName ] []
            , input [ placeholder "Participant 1", value s.newTeam, onInput SetNewTeam ] []
            , input [ placeholder "Participant 2", value s.newTeam2, onInput SetNewTeam2 ] []
            , button [ onClick AddTeam, disabled (String.trim s.newTeamName == "") ] [ text "+ Équipe" ]
            ]
        , div [ Html.Attributes.style "margin-top" ".5rem" ]
            [ Html.textarea
                [ placeholder "Coller une liste — une équipe par ligne (ex. « Les Aigles »)"
                , value s.importText
                , onInput SetImportText
                , Html.Attributes.rows 4
                , Html.Attributes.style "width" "100%"
                ]
                []
            , div [ class "row" ]
                [ button
                    [ class "secondary", onClick ImportList, disabled (String.trim s.importText == "") ]
                    [ text "Importer la liste" ]
                , span [ class "muted", Html.Attributes.style "font-size" ".82rem" ]
                    [ text "une équipe par ligne" ]
                ]
            ]
        , if List.isEmpty s.view.teams then
            p [ class "muted" ] [ text "Aucune équipe." ]

          else
            div [] (List.map teamRow s.view.teams)
        , div [ class "row", Html.Attributes.style "margin-top" "1rem" ]
            [ button [ onClick (GoStep StepPools), disabled (List.length s.view.teams < 2) ]
                [ text "Suivant : Poules →" ]
            ]
        ]


teamRow : Team -> Html Msg
teamRow t =
    let
        players =
            [ t.player1, t.player2 ] |> List.filter (\p -> p /= "") |> String.join " / "
    in
    div [ class "match row", Html.Attributes.style "justify-content" "space-between" ]
        [ div []
            [ div [ Html.Attributes.style "font-weight" "600" ] [ text t.name ]
            , if players == "" then
                text ""

              else
                div [ class "muted", Html.Attributes.style "font-size" ".82rem" ] [ text players ]
            ]
        , button [ class "secondary", onClick (DeleteTeam t.id) ] [ text "✕" ]
        ]


