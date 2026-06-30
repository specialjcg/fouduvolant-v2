module View.Schedule exposing (viewSchedule)

import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (on, onClick, onInput, preventDefaultOn)
import Json.Decode as D
import Time
import Helpers exposing (..)
import Types exposing (..)
import View.Board exposing (launchButtons)


{-| Prévisionnel : page dédiée, horaires réels = heure système + ETA cumulée. -}
viewSchedule : Time.Posix -> Time.Zone -> Sel -> Html Msg
viewSchedule now zone s =
    let
        -- Courts with nobody playing right now (free → can host a match).
        playingCourts =
            s.board.matches |> List.filter (\m -> m.status == "Playing") |> List.filterMap .court

        freeCourts =
            s.view.courts
                |> List.indexedMap (\i c -> ( i + 1, c ))
                |> List.filter (\( _, c ) -> not (List.member c playingCourts))

        q =
            String.trim (String.toLower s.scheduleSearch)

        matches m =
            q == "" || String.contains q (String.toLower m.teamA) || String.contains q (String.toLower m.teamB)

        -- Keep original court indices so "Terrain N" stays correct after filtering.
        filtered =
            s.schedule
                |> List.indexedMap (\i fc -> ( i, { fc | matches = List.filter matches fc.matches } ))
                |> List.filter (\( _, fc ) -> not (List.isEmpty fc.matches))

        nextHit =
            filtered
                |> List.concatMap (\( _, fc ) -> fc.matches)
                |> List.filter (\m -> m.status == "Pending" || m.status == "Playing")
                |> List.sortBy .etaMin
                |> List.head
    in
    div [ class "panel" ]
        [ h2 [] [ text "Prévisionnel" ]
        , p [ class "muted" ]
            [ text ("Horaires estimés (≈15 min/match) à partir de " ++ clockAt zone now 0) ]
        , div [ class "row", Html.Attributes.style "margin" ".4rem 0" ]
            [ input
                [ type_ "search"
                , placeholder "🔍 Ton équipe → tes matchs"
                , value s.scheduleSearch
                , onInput SetScheduleSearch
                , Html.Attributes.style "flex" "1"
                ]
                []
            , if s.scheduleSearch /= "" then
                button [ class "secondary", onClick (SetScheduleSearch "") ] [ text "✕" ]

              else
                text ""
            ]
        , case ( q /= "", nextHit ) of
            ( True, Just m ) ->
                p [ Html.Attributes.style "font-weight" "600", Html.Attributes.style "color" "var(--primary)" ]
                    [ text ("▶ Prochain : " ++ m.teamA ++ " vs " ++ m.teamB ++ " — " ++ clockAt zone now m.etaMin) ]

            _ ->
                text ""
        , if List.isEmpty filtered then
            p [ class "muted" ]
                [ text
                    (if q /= "" then
                        "Aucun match à venir pour « " ++ s.scheduleSearch ++ " »."

                     else
                        "Rien à prévoir pour l'instant."
                    )
                ]

          else
            div [] (List.map (\( i, fc ) -> forecastCourtView now zone freeCourts i fc) filtered)
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


forecastCourtView : Time.Posix -> Time.Zone -> List ( Int, String ) -> Int -> ForecastCourt -> Html Msg
forecastCourtView now zone freeCourts idx fc =
    let
        -- Only the next two pending matches of this court get launch buttons,
        -- to keep the timeline readable.
        launchable =
            fc.matches
                |> List.filter (\m -> m.status == "Pending")
                |> List.take 2
                |> List.map .id
    in
    div [ Html.Attributes.style "margin-bottom" ".8rem" ]
        [ h4 [ Html.Attributes.style "margin" ".3rem 0", Html.Attributes.style "color" "var(--primary)" ]
            [ text ("Terrain " ++ String.fromInt (idx + 1)) ]
        , table []
            (tr []
                [ th [] [ text "Heure" ]
                , th [] [ text "Poule" ]
                , th [] [ text "Match" ]
                , th [] [ text "Score" ]
                , th [] [ text "Action" ]
                ]
                :: List.map (forecastRow now zone freeCourts fc.court launchable) fc.matches
            )
        ]


forecastRow : Time.Posix -> Time.Zone -> List ( Int, String ) -> String -> List String -> ForecastMatch -> Html Msg
forecastRow now zone freeCourts court launchable m =
    let
        score =
            if m.status == "Done" then
                String.fromInt m.pointsA ++ "-" ++ String.fromInt m.pointsB

            else if m.status == "Playing" then
                "en cours"

            else
                "—"

        action =
            if List.member m.id launchable then
                div [ class "row", Html.Attributes.style "gap" "4px", Html.Attributes.style "flex-wrap" "wrap" ]
                    (launchButtons court freeCourts m.id)

            else
                text ""
    in
    tr []
        [ td [] [ text (clockAt zone now m.etaMin) ]
        , td [] [ text (Maybe.withDefault "" m.pool) ]
        , td [] [ text (m.teamA ++ " vs " ++ m.teamB) ]
        , td [] [ text score ]
        , td [] [ action ]
        ]


