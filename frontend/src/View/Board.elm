module View.Board exposing (viewBoard)

import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (on, onClick, onInput, preventDefaultOn)
import Json.Decode as D
import Time
import Helpers exposing (..)
import Types exposing (..)




viewBoard : Bool -> Sel -> Dict String String -> Html Msg
viewBoard showPast s names =
    div [ class "panel" ]
        [ div [ class "row" ]
            [ h2 [] [ text "Terrains" ]
            , button [ onClick Dispatch ] [ text "⟳ Dispatch auto" ]
            , if s.view.phase == "PoolPhase" && not (List.any (\m -> m.pool /= Nothing && m.status == "Done") s.board.matches) then
                button [ class "danger", onClick RedoPools ] [ text "Refaire les poules" ]

              else
                text ""
            , button [ class "secondary", onClick ToggleShowPast ]
                [ text
                    (if showPast then
                        "Cacher les matchs passés"

                     else
                        "Afficher les matchs passés"
                    )
                ]
            ]
        , if List.isEmpty s.view.courts then
            p [ class "muted" ] [ text "Aucun terrain configuré." ]

          else
            div [ class "lanes" ] (List.indexedMap (viewLane showPast s names) s.board.courts)
        , viewPending s names
        ]



{-| One court as a horizontal timeline: completed (left) → current → next →
previews (right). -}
viewLane : Bool -> Sel -> Dict String String -> Int -> CourtPlan -> Html Msg
viewLane showPast s names idx cp =
    let
        -- Past matches are hidden by default so the live match sits first; the
        -- toggle reveals them on the left.
        completed =
            if showPast then
                s.board.matches
                    |> List.filter (\m -> m.status == "Done" && m.court == Just cp.court)
                    |> List.sortBy (\m -> Maybe.withDefault 0 m.doneOrder)

            else
                []

        currentNode =
            cp.current
                |> Maybe.andThen (findMatch s.board.matches)
                |> Maybe.map (liveNode s names)
                |> maybeList

        -- Courts with nobody playing right now → a prévision can be launched there.
        playingCourts =
            s.board.matches |> List.filter (\m -> m.status == "Playing") |> List.filterMap .court

        freeCourts =
            s.view.courts
                |> List.indexedMap (\i c -> ( i + 1, c ))
                |> List.filter (\( _, c ) -> not (List.member c playingCourts))

        nextNode =
            cp.next
                |> Maybe.andThen (\sg -> Maybe.map (\m -> ( m, sg )) (findMatch s.board.matches sg.matchId))
                |> Maybe.map (\( m, sg ) -> suggestNode s freeCourts names cp.court m sg)
                |> maybeList

        previewNodes =
            List.filterMap
                (\sg -> Maybe.map (previewNode freeCourts cp.court names) (findMatch s.board.matches sg.matchId))
                cp.previews

        ( badgeClass, badgeText ) =
            if cp.current /= Nothing then
                ( "badge live", "En cours" )

            else if cp.next /= Nothing then
                ( "badge free", "Libre" )

            else
                ( "badge idle", "Inactif" )

        nodes =
            List.map (doneNode s names) completed ++ currentNode ++ nextNode ++ previewNodes
    in
    div [ class "lane" ]
        [ div [ class "lane-head" ]
            [ span [ class "lane-name" ] [ text ("Terrain " ++ String.fromInt (idx + 1)) ]
            , span [ class badgeClass ] [ text badgeText ]
            ]
        , div [ class "track" ]
            (if List.isEmpty nodes then
                [ p [ class "muted" ] [ text "Aucun match" ] ]

             else
                nodes
            )
        ]


maybeList : Maybe a -> List a
maybeList m =
    case m of
        Just x ->
            [ x ]

        Nothing ->
            []


nodeHead : String -> Html Msg
nodeHead label =
    div [ class "node-head" ] [ text label ]


{-| Display the recorded sets ("21-15  21-10") for a finished match; falls back
to the summed points if no per-set detail is available. -}
setsLabel : MatchV -> String
setsLabel m =
    if List.isEmpty m.sets then
        String.fromInt m.pointsA ++ "-" ++ String.fromInt m.pointsB

    else
        m.sets
            |> List.map (\( a, b ) -> String.fromInt a ++ "-" ++ String.fromInt b)
            |> String.join "  "


doneNode : Sel -> Dict String String -> MatchV -> Html Msg
doneNode s names m =
    let
        ( a, b ) =
            Maybe.withDefault ( String.fromInt m.pointsA, String.fromInt m.pointsB )
                (Dict.get m.id s.scores)

        footer =
            if s.editing == Just m.id then
                div [ class "row" ]
                    [ input [ class "score", type_ "number", value a, onInput (SetScore m.id 0) ] []
                    , text "-"
                    , input [ class "score", type_ "number", value b, onInput (SetScore m.id 1) ] []
                    , button [ onClick (Rescore m.id) ] [ text "OK" ]
                    , button [ class "secondary", onClick CancelEdit ] [ text "✕" ]
                    ]

            else
                div [ class "row" ]
                    (span [ Html.Attributes.style "font-weight" "600" ]
                        [ text
                            (if m.conceded then
                                "Forfait"

                             else
                                setsLabel m
                            )
                        ]
                        :: button [ class "secondary", onClick (EditScore m.id m.pointsA m.pointsB) ] [ text "✎" ]
                        :: (if m.pool == Nothing then
                                [ button [ class "secondary", onClick (ResetMatch m.id), Html.Attributes.title "Réinitialiser ce match" ] [ text "↺" ] ]

                            else
                                []
                           )
                    )
    in
    div [ class "node done" ]
        [ nodeHead "Terminé"
        , div [ class "node-teams" ] [ text (matchLabel names m) ]
        , footer
        ]


liveNode : Sel -> Dict String String -> MatchV -> Html Msg
liveNode s names m =
    div [ class "node live" ]
        [ nodeHead "● En cours"
        , div [ class "node-teams" ] [ text (matchLabel names m) ]
        , if List.isEmpty m.sets then
            text ""

          else
            div [ class "muted", Html.Attributes.style "font-size" ".78rem", Html.Attributes.style "margin-bottom" ".25rem" ]
                [ text ("Sets : " ++ setsLabel m) ]
        , scoreEntry s m.id
        , forfeitArea s names m
        ]


{-| Forfeit is rare, so it stays behind a small toggle. Collapsed: a discreet
"Forfait" button. Expanded: two stacked buttons naming the team that *gives up*
(the other wins) plus a cancel. Works before start (no-show) or during play. -}
forfeitArea : Sel -> Dict String String -> MatchV -> Html Msg
forfeitArea s names m =
    if s.forfeitOpen == Just m.id then
        div [ class "forfeit" ]
            [ span [ class "muted forfeit-label" ] [ text "Forfait — qui abandonne ?" ]
            , button [ class "secondary forfeit-btn", onClick (ConcedeMatch m.id m.teamB) ] [ text (nameOf names m.teamA) ]
            , button [ class "secondary forfeit-btn", onClick (ConcedeMatch m.id m.teamA) ] [ text (nameOf names m.teamB) ]
            , button [ class "secondary forfeit-cancel", onClick (ToggleForfeit m.id) ] [ text "Annuler" ]
            ]

    else
        div [ class "forfeit-trigger" ]
            [ button [ class "secondary forfeit-mini", onClick (ToggleForfeit m.id) ] [ text "Forfait" ] ]


suggestNode : Sel -> List ( Int, String ) -> Dict String String -> String -> MatchV -> Sugg -> Html Msg
suggestNode s freeCourts names court m sg =
    div [ class "node suggest" ]
        [ nodeHead
            (if sg.needsRest then
                "Suivant · repos"

             else
                "Suivant"
            )
        , div [ class "node-teams" ] [ text (matchLabel names m) ]
        , div [ class "row", Html.Attributes.style "gap" "4px", Html.Attributes.style "flex-wrap" "wrap" ]
            (launchButtons court freeCourts m.id)
        , forfeitArea s names m
        ]


{-| Launch buttons for a prévision: "▶ Démarrer" on this lane's own court when
free, plus "▶ T{n}" for every other free court so a match can be sent elsewhere.
-}
launchButtons : String -> List ( Int, String ) -> String -> List (Html Msg)
launchButtons court freeCourts matchId =
    if List.isEmpty freeCourts then
        [ span [ class "muted", Html.Attributes.style "font-size" ".78rem" ]
            [ text "à la fin du match en cours" ]
        ]

    else
        let
            ( own, others ) =
                List.partition (\( _, c ) -> c == court) freeCourts
        in
        List.map (\( _, c ) -> button [ onClick (StartMatch matchId c) ] [ text "▶ Démarrer" ]) own
            ++ List.map
                (\( n, c ) ->
                    button [ class "secondary", onClick (StartMatch matchId c) ]
                        [ text ("▶ T" ++ String.fromInt n) ]
                )
                others


previewNode : List ( Int, String ) -> String -> Dict String String -> MatchV -> Html Msg
previewNode freeCourts court names m =
    div [ class "node preview" ]
        [ nodeHead "À venir"
        , div [ class "node-teams" ] [ text (matchLabel names m) ]
        , div [ class "row", Html.Attributes.style "gap" "4px", Html.Attributes.style "flex-wrap" "wrap" ]
            (launchButtons court freeCourts m.id)
        ]


scoreEntry : Sel -> String -> Html Msg
scoreEntry s matchId =
    let
        ( a, b ) =
            Maybe.withDefault ( "", "" ) (Dict.get matchId s.scores)
    in
    div [ class "row" ]
        [ input [ class "score", type_ "number", placeholder "0", value a, onInput (SetScore matchId 0) ] []
        , text "-"
        , input [ class "score", type_ "number", placeholder "0", value b, onInput (SetScore matchId 1) ] []
        , button [ class "secondary", onClick (SubmitScore matchId) ] [ text "OK" ]
        ]


viewPending : Sel -> Dict String String -> Html Msg
viewPending s names =
    let
        -- Matches already placed in a lane (current / next / preview).
        laneIds =
            s.board.courts
                |> List.concatMap
                    (\cp ->
                        maybeList cp.current
                            ++ maybeList (Maybe.map .matchId cp.next)
                            ++ List.map .matchId cp.previews
                    )

        queue =
            s.board.matches
                |> List.filter (\m -> m.status == "Pending" && not (List.member m.id laneIds))

        -- Courts with nobody playing right now (free → can host a queued match).
        playingCourts =
            s.board.matches |> List.filter (\m -> m.status == "Playing") |> List.filterMap .court

        freeCourts =
            s.view.courts
                |> List.indexedMap (\i c -> ( i + 1, c ))
                |> List.filter (\( _, c ) -> not (List.member c playingCourts))
    in
    if List.isEmpty queue then
        text ""

    else
        let
            poolNames =
                Dict.fromList (List.map (\p -> ( p.id, p.name )) s.view.pools)
        in
        div []
            [ h3 [ class "muted" ] [ text "En attente — proposer un terrain" ]
            , div [] (List.map (pendingRow poolNames names freeCourts) queue)
            ]


pendingRow : Dict String String -> Dict String String -> List ( Int, String ) -> MatchV -> Html Msg
pendingRow poolNames names freeCourts m =
    div [ class "match row", Html.Attributes.style "justify-content" "space-between" ]
        [ span []
            [ case m.pool |> Maybe.andThen (\pid -> Dict.get pid poolNames) of
                Just pn ->
                    span [ class "pill", Html.Attributes.style "margin-right" ".4rem" ] [ text pn ]

                Nothing ->
                    text ""
            , text (matchLabel names m)
            ]
        , div [ class "row" ]
            (if List.isEmpty freeCourts then
                [ span [ class "muted", Html.Attributes.style "font-size" ".8rem" ] [ text "terrains occupés" ] ]

             else
                List.map
                    (\( n, c ) ->
                        button [ class "secondary", onClick (StartMatch m.id c) ]
                            [ text ("▶ T" ++ String.fromInt n) ]
                    )
                    freeCourts
            )
        ]
