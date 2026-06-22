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
            div [] (List.map (teamRow s.view.phase) s.view.teams)
        , div [ class "row", Html.Attributes.style "margin-top" "1rem" ]
            [ button [ onClick (GoStep StepPools), disabled (List.length s.view.teams < 2) ]
                [ text "Suivant : Poules →" ]
            ]
        , forfeitModal s
        ]


teamRow : String -> Team -> Html Msg
teamRow phase t =
    let
        players =
            [ t.player1, t.player2 ] |> List.filter (\p -> p /= "") |> String.join " / "

        draft =
            phase == "Draft"

        -- In draft the ✕ deletes the team; afterwards matches exist, so it
        -- declares a forfeit (with confirmation) instead of a hard delete.
        ( cross, title ) =
            if draft then
                ( DeleteTeam t.id, "Supprimer l'équipe" )

            else
                ( AskForfeit t.id, "Déclarer forfait" )
    in
    div [ class "match row", Html.Attributes.style "justify-content" "space-between" ]
        [ div []
            [ div [ Html.Attributes.style "font-weight" "600" ]
                [ text t.name
                , if t.forfeited then
                    span
                        [ class "muted"
                        , Html.Attributes.style "margin-left" ".5rem"
                        , Html.Attributes.style "font-size" ".72rem"
                        , Html.Attributes.style "color" "#c0392b"
                        ]
                        [ text "Forfait" ]

                  else
                    text ""
                ]
            , if players == "" then
                text ""

              else
                div [ class "muted", Html.Attributes.style "font-size" ".82rem" ] [ text players ]
            ]
        , if t.forfeited then
            text ""

          else
            button
                [ class "secondary", Html.Attributes.title title, onClick cross ]
                [ text "✕" ]
        ]


{-| Confirmation overlay before declaring a team forfeited. -}
forfeitModal : Sel -> Html Msg
forfeitModal s =
    case s.confirmForfeit of
        Nothing ->
            text ""

        Just teamId ->
            let
                name =
                    s.view.teams
                        |> List.filter (\t -> t.id == teamId)
                        |> List.head
                        |> Maybe.map .name
                        |> Maybe.withDefault "cette équipe"
            in
            div
                [ Html.Attributes.style "position" "fixed"
                , Html.Attributes.style "inset" "0"
                , Html.Attributes.style "background" "rgba(0,0,0,.5)"
                , Html.Attributes.style "display" "flex"
                , Html.Attributes.style "align-items" "center"
                , Html.Attributes.style "justify-content" "center"
                , Html.Attributes.style "z-index" "1000"
                ]
                [ div [ class "panel", Html.Attributes.style "max-width" "26rem" ]
                    [ h2 [] [ text "Déclarer forfait" ]
                    , p []
                        [ text "Confirmer le forfait de "
                        , strong [] [ text name ]
                        , text " ? Ses matchs non joués seront perdus par forfait (l'adversaire gagne)."
                        ]
                    , div [ class "row", Html.Attributes.style "justify-content" "flex-end" ]
                        [ button [ class "secondary", onClick CancelForfeit ] [ text "Annuler" ]
                        , button [ onClick (ConfirmForfeit teamId) ] [ text "Confirmer le forfait" ]
                        ]
                    ]
                ]


