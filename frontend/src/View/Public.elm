module View.Public exposing (viewPublic)

{-| Read-only public view, reached via `/?public=<tid>`. No admin controls:
just "en cours par terrain" + the searchable forecast with reassuring ETA.
Target for a big screen at the venue or a QR code at the scorer's table.
-}

import Html exposing (..)
import Html.Attributes exposing (class)
import Time
import Types exposing (..)
import View.Schedule exposing (viewSchedule)


viewPublic : Time.Posix -> Time.Zone -> Sel -> Html Msg
viewPublic now zone s =
    div []
        [ div [ class "panel" ]
            [ h2 []
                [ text s.view.name
                , text " "
                , span [ class "pill" ] [ text "📺 Vue publique" ]
                ]
            , liveLine s
            ]
        , viewSchedule True now zone s
        ]


{-| "En cours" summary: for each court, the match currently being played. -}
liveLine : Sel -> Html Msg
liveLine s =
    let
        playing =
            s.schedule
                |> List.indexedMap
                    (\i fc ->
                        fc.matches
                            |> List.filter (\m -> m.status == "Playing")
                            |> List.head
                            |> Maybe.map (\m -> "T" ++ String.fromInt (i + 1) ++ " · " ++ m.teamA ++ " vs " ++ m.teamB)
                    )
                |> List.filterMap identity
    in
    if List.isEmpty playing then
        p [ class "muted" ] [ text "Aucun match en cours." ]

    else
        p [ Html.Attributes.style "font-weight" "600" ]
            [ text ("● En cours : " ++ String.join "   |   " playing) ]
