module View.List exposing (viewList)

import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (on, onClick, onInput, preventDefaultOn)
import Json.Decode as D
import Time
import Helpers exposing (..)
import Types exposing (..)


viewList : Model -> Html Msg
viewList model =
    div []
        [ div [ class "panel" ]
            [ h2 [] [ text "Nouveau tournoi" ]
            , div [ class "row" ]
                [ input [ placeholder "Nom du tournoi", value model.newName, onInput SetNewName ] []
                , button [ onClick CreateTournament ] [ text "Créer" ]
                ]
            ]
        , div [ class "panel" ]
            [ h2 [] [ text "Tournois" ]
            , if List.isEmpty model.tournaments then
                p [ class "muted" ] [ text "Aucun tournoi." ]

              else
                div [] (List.map tournamentRow model.tournaments)
            ]
        ]


tournamentRow : Summary -> Html Msg
tournamentRow t =
    div [ class "match row", Html.Attributes.style "justify-content" "space-between" ]
        [ div [ class "row" ]
            [ a [ onClick (OpenT t.id) ] [ text t.name ]
            , span [ class "pill" ] [ text t.phase ]
            ]
        , button [ class "secondary", onClick (DeleteTournament t.id) ] [ text "✕" ]
        ]


