module View.Schedule exposing (viewSchedule)

import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (on, onClick, onInput, preventDefaultOn)
import Json.Decode as D
import Time
import Helpers exposing (..)
import Types exposing (..)


{-| Prévisionnel : page dédiée, horaires réels = heure système + ETA cumulée. -}
viewSchedule : Time.Posix -> Time.Zone -> Sel -> Html Msg
viewSchedule now zone s =
    div [ class "panel" ]
        [ h2 [] [ text "Prévisionnel" ]
        , p [ class "muted" ]
            [ text ("Horaires estimés (≈15 min/match) à partir de " ++ clockAt zone now 0) ]
        , if List.all (\fc -> List.isEmpty fc.matches) s.schedule then
            p [ class "muted" ] [ text "Rien à prévoir pour l'instant." ]

          else
            div [] (List.indexedMap (forecastCourtView now zone) s.schedule)
        ]


{-| Wall-clock "HHhMM" of `base` shifted by `etaMin` minutes, in `zone`. -}
clockAt : Time.Zone -> Time.Posix -> Int -> String
clockAt zone base etaMin =
    let
        p =
            Time.millisToPosix (Time.posixToMillis base + etaMin * 60000)

        pad n =
            String.padLeft 2 '0' (String.fromInt n)
    in
    pad (Time.toHour zone p) ++ "h" ++ pad (Time.toMinute zone p)


forecastCourtView : Time.Posix -> Time.Zone -> Int -> ForecastCourt -> Html Msg
forecastCourtView now zone idx fc =
    div [ Html.Attributes.style "margin-bottom" ".8rem" ]
        [ h4 [ Html.Attributes.style "margin" ".3rem 0", Html.Attributes.style "color" "var(--primary)" ]
            [ text ("Terrain " ++ String.fromInt (idx + 1)) ]
        , table []
            (tr []
                [ th [] [ text "Heure" ]
                , th [] [ text "Poule" ]
                , th [] [ text "Match" ]
                , th [] [ text "Score" ]
                ]
                :: List.map (forecastRow now zone) fc.matches
            )
        ]


forecastRow : Time.Posix -> Time.Zone -> ForecastMatch -> Html Msg
forecastRow now zone m =
    let
        score =
            if m.status == "Done" then
                String.fromInt m.pointsA ++ "-" ++ String.fromInt m.pointsB

            else if m.status == "Playing" then
                "en cours"

            else
                "—"
    in
    tr []
        [ td [] [ text (clockAt zone now m.etaMin) ]
        , td [] [ text (Maybe.withDefault "" m.pool) ]
        , td [] [ text (m.teamA ++ " vs " ++ m.teamB) ]
        , td [] [ text score ]
        ]


