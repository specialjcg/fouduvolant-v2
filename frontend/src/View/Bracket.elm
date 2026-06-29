module View.Bracket exposing (viewBracket)

import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (on, onClick, onInput, preventDefaultOn)
import Json.Decode as D
import Time
import Helpers exposing (..)
import Types exposing (..)


viewBracket : Sel -> Html Msg
viewBracket s =
    div [ class "panel finals" ]
        [ div [ class "row" ]
            [ h2 [] [ text "Finales" ]
            , text "Qualifiés/poule :"
            , input [ type_ "number", class "score", value s.perPool, onInput SetPerPool ] []
            , button [ onClick GenBracket ] [ text "Générer" ]
            , button [ class "secondary", onClick AdvanceBracket ] [ text "Avancer" ]
            , button [ class "danger", onClick ResetBracket ] [ text "Réinitialiser le bracket" ]
            , span [ class "muted", Html.Attributes.style "margin-left" ".5rem" ] [ text "Tout :" ]
            , button
                [ class
                    (if s.view.bracketFormat == "BestOf1" then
                        ""

                     else
                        "secondary"
                    )
                , onClick (SetFinalsFormat "BestOf1")
                ]
                [ text "1 set" ]
            , button
                [ class
                    (if s.view.bracketFormat == "BestOf3" then
                        ""

                     else
                        "secondary"
                    )
                , onClick (SetFinalsFormat "BestOf3")
                ]
                [ text "2 sets gagnants" ]
            ]
        , if List.isEmpty s.bracket then
            p [ class "muted" ] [ text "Bracket non tiré." ]

          else
            div []
                [ roundFormatBar s.view.bracketFormat s.view.roundFormats (List.filter (\n -> n.kind == "Main") s.bracket)
                , bracketTree s "Principal" (List.filter (\n -> n.kind == "Main") s.bracket)
                , bracketTree s "Consolante" (List.filter (\n -> n.kind == "Consolation") s.bracket)
                ]
        ]


thirdPlaceRound : Int
thirdPlaceRound =
    255


{-| Per-round format chooser: one "1 set / BO3" pair per round of the main draw,
labelled by team count (8es, quarts, demi, finale). Applies by size to the
consolation draw too. -}
roundFormatBar : String -> Dict String String -> List BracketNode -> Html Msg
roundFormatBar defaultFmt roundFormats nodes =
    let
        maxRound =
            nodes
                |> List.filter (\n -> n.round /= 0 && n.round /= thirdPlaceRound)
                |> List.map .round
                |> List.maximum
                |> Maybe.withDefault 1

        sizes =
            List.range 1 maxRound |> List.map (\r -> 2 ^ (maxRound - r + 1))

        control size =
            let
                cur =
                    Dict.get (String.fromInt size) roundFormats |> Maybe.withDefault defaultFmt

                cls want =
                    if cur == want then
                        ""

                    else
                        "secondary"
            in
            span [ class "row", Html.Attributes.style "gap" "3px", Html.Attributes.style "align-items" "center" ]
                [ span [ class "muted", Html.Attributes.style "font-size" ".78rem" ] [ text (roundSizeLabel size ++ " :") ]
                , button [ class (cls "BestOf1"), onClick (SetRoundFormat size "BestOf1") ] [ text "1 set" ]
                , button [ class (cls "BestOf3"), onClick (SetRoundFormat size "BestOf3") ] [ text "BO3" ]
                ]
    in
    div [ class "row", Html.Attributes.style "flex-wrap" "wrap", Html.Attributes.style "gap" ".7rem", Html.Attributes.style "margin-bottom" ".6rem" ]
        (span [ class "muted" ] [ text "Format par tour :" ] :: List.map control sizes)


roundSizeLabel : Int -> String
roundSizeLabel size =
    case size of
        2 ->
            "Finale"

        4 ->
            "Demi-finales"

        8 ->
            "Quarts"

        16 ->
            "8es"

        32 ->
            "16es"

        64 ->
            "32es"

        n ->
            "Tour de " ++ String.fromInt n


{-| One bracket (main/consolation) as a deterministic positioned tree with
connector lines, reproducing the original look. Binary rounds spread evenly over
the height; barrages spread over their own (denser) count, each connected to the
round-1 match it feeds. -}
bracketTree : Sel -> String -> List BracketNode -> Html Msg
bracketTree s title nodes =
    if List.isEmpty nodes then
        text ""

    else
        let
            maxRound =
                nodes
                    |> List.filter (\n -> n.round /= 0 && n.round /= thirdPlaceRound)
                    |> List.map .round
                    |> List.maximum
                    |> Maybe.withDefault 1

            barrages =
                nodes
                    |> List.filter (\n -> n.round == 0)
                    |> List.sortBy (\n -> ( Maybe.withDefault 0 n.feeds, n.index ))

            thirdNodes =
                List.filter (\n -> n.round == thirdPlaceRound) nodes

            hasBarr =
                not (List.isEmpty barrages)

            countOf r =
                List.length (List.filter (\n -> n.round == r) nodes)

            r1c =
                Basics.max 1 (countOf 1)

            rows =
                toFloat (Basics.max r1c (List.length barrages))

            totalH =
                rows * brkCell

            colOf r =
                if hasBarr then
                    r

                else
                    r - 1

            maxCol =
                if hasBarr then
                    maxRound

                else
                    maxRound - 1

            xOf c =
                toFloat c * (brkBoxW + brkColGap)

            cy count i =
                brkTopPad + (toFloat i + 0.5) * totalH / toFloat (Basics.max 1 count)

            barrEls =
                barrages
                    |> List.indexedMap
                        (\k n ->
                            let
                                sy =
                                    cy (List.length barrages) k

                                ty =
                                    cy r1c (Maybe.withDefault 0 n.feeds)
                            in
                            connector (xOf 0 + brkBoxW) sy (xOf (colOf 1)) ty
                                ++ [ posBox s (xOf 0) sy brkBarH n ]
                        )
                    |> List.concat

            roundEls r =
                List.filter (\n -> n.round == r) nodes
                    |> List.sortBy .index
                    |> List.map
                        (\n ->
                            let
                                sy =
                                    cy (countOf r) n.index

                                conn =
                                    if r < maxRound then
                                        connector (xOf (colOf r) + brkBoxW) sy (xOf (colOf (r + 1))) (cy (countOf (r + 1)) (n.index // 2))

                                    else
                                        []
                            in
                            conn ++ [ posBox s (xOf (colOf r)) sy brkBoxH n ]
                        )
                    |> List.concat

            titles =
                (if hasBarr then
                    [ titleAt (xOf 0) "Barrages" ]

                 else
                    []
                )
                    ++ List.map (\r -> titleAt (xOf (colOf r)) (roundLabel maxRound r)) (List.range 1 maxRound)

            boxesAndLines =
                barrEls ++ List.concatMap roundEls (List.range 1 maxRound)
        in
        div []
            [ h3 [] [ text title ]
            , div [ class "bracket" ]
                [ div
                    [ class "bracket-abs"
                    , Html.Attributes.style "width" (px (xOf maxCol + brkBoxW))
                    , Html.Attributes.style "height" (px (totalH + brkTopPad + brkBoxH))
                    ]
                    (titles ++ boxesAndLines)
                ]
            , if List.isEmpty thirdNodes then
                text ""

              else
                div [ class "brk-third" ]
                    (span [ class "brk-third-label" ] [ text "3e place" ]
                        :: List.map (plainBox s) thirdNodes
                    )
            ]


brkCell : Float
brkCell =
    96


brkBoxW : Float
brkBoxW =
    186


brkBoxH : Float
brkBoxH =
    50


brkBarH : Float
brkBarH =
    50


brkColGap : Float
brkColGap =
    104


brkTopPad : Float
brkTopPad =
    42


px : Float -> String
px f =
    String.fromFloat f ++ "px"


titleAt : Float -> String -> Html Msg
titleAt x label =
    div
        [ class "brk-title"
        , Html.Attributes.style "left" (px x)
        , Html.Attributes.style "width" (px brkBoxW)
        ]
        [ text label ]


posBox : Sel -> Float -> Float -> Float -> BracketNode -> Html Msg
posBox s x cyc h n =
    div
        [ class "bmatch"
        , Html.Attributes.style "position" "absolute"
        , Html.Attributes.style "left" (px x)
        , Html.Attributes.style "top" (px (cyc - h / 2))
        , Html.Attributes.style "width" (px brkBoxW)
        , Html.Attributes.style "min-height" (px h)
        ]
        [ seedRow n.teamA n.winner, seedRow n.teamB n.winner, scoreFooter s n ]


plainBox : Sel -> BracketNode -> Html Msg
plainBox s n =
    div [ class "bmatch", Html.Attributes.style "width" (px brkBoxW) ]
        [ seedRow n.teamA n.winner, seedRow n.teamB n.winner, scoreFooter s n ]


{-| Inline score entry inside a bracket box. A node only carries a `matchId`
once its pairing is materialized (auto-scheduled by the backend as soon as both
teams are known). Decided match → recorded score + edit (✎). Materialized but
undecided → a set-entry row that appends a set, exactly like the board. Reuses
the board's draft state (`s.scores` / `s.editing`) and messages. -}
scoreFooter : Sel -> BracketNode -> Html Msg
scoreFooter s n =
    case n.matchId of
        Nothing ->
            text ""

        Just mid ->
            if n.winner /= Nothing then
                brkDone s mid n

            else
                brkEntry s mid n


brkDone : Sel -> String -> BracketNode -> Html Msg
brkDone s mid n =
    let
        ( a, b ) =
            Maybe.withDefault ( String.fromInt n.pointsA, String.fromInt n.pointsB )
                (Dict.get mid s.scores)
    in
    if s.editing == Just mid then
        div [ class "row brk-score" ]
            [ input [ class "score", type_ "number", value a, onInput (SetScore mid 0) ] []
            , text "-"
            , input [ class "score", type_ "number", value b, onInput (SetScore mid 1) ] []
            , button [ onClick (Rescore mid) ] [ text "OK" ]
            , button [ class "secondary", onClick CancelEdit ] [ text "✕" ]
            ]

    else
        div []
            [ div [ class "row brk-score" ]
                [ span [ Html.Attributes.style "font-weight" "600" ] [ text (brkSetsLabel n) ]
                , button [ class "secondary", onClick (EditScore mid n.pointsA n.pointsB) ] [ text "✎" ]
                ]
            , brkWarn n
            ]


brkEntry : Sel -> String -> BracketNode -> Html Msg
brkEntry s mid n =
    let
        ( a, b ) =
            Maybe.withDefault ( "", "" ) (Dict.get mid s.scores)
    in
    div []
        [ if List.isEmpty n.sets then
            text ""

          else
            div [ class "muted", Html.Attributes.style "font-size" ".68rem" ]
                [ text ("Sets : " ++ brkSetsLabel n) ]
        , div [ class "row brk-score" ]
            [ input [ class "score", type_ "number", placeholder "0", value a, onInput (SetScore mid 0) ] []
            , text "-"
            , input [ class "score", type_ "number", placeholder "0", value b, onInput (SetScore mid 1) ] []
            , button [ class "secondary", onClick (SubmitScore mid) ] [ text "OK" ]
            ]
        , brkWarn n
        ]


brkSetsLabel : BracketNode -> String
brkSetsLabel n =
    if List.isEmpty n.sets then
        String.fromInt n.pointsA ++ "-" ++ String.fromInt n.pointsB

    else
        n.sets
            |> List.map (\( a, b ) -> String.fromInt a ++ "-" ++ String.fromInt b)
            |> String.join "  "


brkWarn : BracketNode -> Html Msg
brkWarn n =
    if n.irregular then
        div
            [ class "muted"
            , Html.Attributes.style "font-size" ".66rem"
            , Html.Attributes.style "color" "#c0392b"
            , Html.Attributes.title "Score enregistré tel quel, ne suit pas la règle BWF (21, écart de 2, plafond 30)"
            ]
            [ text "⚠ hors BWF" ]

    else
        text ""


connector : Float -> Float -> Float -> Float -> List (Html Msg)
connector x1 y1 x2 y2 =
    let
        mx =
            (x1 + x2) / 2
    in
    [ lnDiv (Basics.min x1 mx) y1 (abs (mx - x1)) 1
    , lnDiv mx (Basics.min y1 y2) 1 (abs (y2 - y1))
    , lnDiv (Basics.min mx x2) y2 (abs (x2 - mx)) 1
    ]


lnDiv : Float -> Float -> Float -> Float -> Html Msg
lnDiv x y w h =
    div
        [ class "ln"
        , Html.Attributes.style "left" (px x)
        , Html.Attributes.style "top" (px y)
        , Html.Attributes.style "width" (px w)
        , Html.Attributes.style "height" (px h)
        ]
        []


roundLabel : Int -> Int -> String
roundLabel maxRound r =
    if r == 0 then
        "Barrages"

    else
        case 2 ^ (maxRound - r + 1) of
            2 ->
                "Finale"

            4 ->
                "Demi-finales"

            8 ->
                "Quarts"

            16 ->
                "8es de finale"

            32 ->
                "16es de finale"

            n ->
                "Tour de " ++ String.fromInt n


seedRow : Maybe String -> Maybe String -> Html Msg
seedRow team winner =
    let
        isWin =
            case ( team, winner ) of
                ( Just t, Just w ) ->
                    t == w

                _ ->
                    False
    in
    div
        [ class
            (case team of
                Just _ ->
                    if isWin then
                        "seed win"

                    else
                        "seed"

                Nothing ->
                    "seed empty"
            )
        ]
        [ span [ class "nm" ] [ text (Maybe.withDefault "" team) ] ]


