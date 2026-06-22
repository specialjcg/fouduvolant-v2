port module Main exposing (main)

{-| Fou du Volant — Elm MVU frontend over the axum HTTP API.

Two screens: the tournament list (with a create form) and a selected
tournament's setup + live board. The board polls every few seconds and after
every mutation, so it reflects the event-sourced backend.
-}

import Api exposing (..)
import Browser
import Dict exposing (Dict)
import Helpers exposing (..)
import Json.Encode as E
import Task
import Time
import Types exposing (..)
import View exposing (view)



-- PORTS


{-| Persist the "show past matches" toggle to localStorage (read back via flags). -}
port saveShowPast : Bool -> Cmd msg



-- INIT


init : Flags -> ( Model, Cmd Msg )
init flags =
    let
        parts =
            String.split "/" flags.open

        tid =
            List.head parts |> Maybe.withDefault ""

        want =
            parts |> List.drop 1 |> List.head |> Maybe.map stepFromString |> Maybe.withDefault StepTeams
    in
    ( { api = flags.apiBase
      , tournaments = []
      , sel = Nothing
      , newName = ""
      , err = Nothing
      , wantStep = want
      , showPast = flags.showPast
      , now = Time.millisToPosix 0
      , zone = Time.utc
      }
    , Cmd.batch
        [ loadTournaments flags.apiBase
        , Time.here |> Task.perform GotZone
        , Time.now |> Task.perform Tick
        , if tid == "" then
            Cmd.none

          else
            openCmds flags.apiBase tid
        ]
    )



-- UPDATE


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        GotTournaments (Ok ts) ->
            ( { model | tournaments = ts, err = Nothing }, Cmd.none )

        GotTournaments (Err e) ->
            ( { model | err = Just (httpErr e) }, Cmd.none )

        SetNewName s ->
            ( { model | newName = s }, Cmd.none )

        CreateTournament ->
            if String.trim model.newName == "" then
                ( model, Cmd.none )

            else
                ( model, createTournament model.api model.newName )

        Created (Ok id) ->
            ( { model | newName = "" }, Cmd.batch [ loadTournaments model.api, openCmds model.api id ] )

        Created (Err e) ->
            ( { model | err = Just (httpErr e) }, Cmd.none )

        OpenT id ->
            ( model, openCmds model.api id )

        DeleteTournament id ->
            ( model, deleteTournament model.api id )

        Deleted _ ->
            ( model, loadTournaments model.api )

        CloseT ->
            ( { model | sel = Nothing }, loadTournaments model.api )

        GotView (Ok v) ->
            ( { model | sel = Just (mergeView model.wantStep model.sel v), err = Nothing }, Cmd.none )

        GotView (Err e) ->
            ( { model | err = Just (httpErr e) }, Cmd.none )

        GotBoard (Ok b) ->
            ( { model | sel = Maybe.map (\s -> { s | board = b }) model.sel }, Cmd.none )

        GotBoard (Err e) ->
            ( { model | err = Just (httpErr e) }, Cmd.none )

        GotStandings (Ok st) ->
            ( { model | sel = Maybe.map (\s -> { s | standings = st }) model.sel }, Cmd.none )

        GotStandings (Err _) ->
            ( model, Cmd.none )

        GotSchedule (Ok sc) ->
            ( { model | sel = Maybe.map (\s -> { s | schedule = sc }) model.sel }, Cmd.none )

        GotSchedule (Err _) ->
            ( model, Cmd.none )

        GotBracket (Ok b) ->
            ( { model | sel = Maybe.map (\s -> { s | bracket = b }) model.sel }, Cmd.none )

        GotBracket (Err _) ->
            ( model, Cmd.none )

        SetPerPool s ->
            ( mapSel (\s_ -> { s_ | perPool = s }) model, Cmd.none )

        GenBracket ->
            withSel model
                (\s ->
                    ( model, genBracket model.api s.id (Maybe.withDefault 2 (String.toInt s.perPool)) )
                )

        AdvanceBracket ->
            withSel model (\s -> ( model, advBracket model.api s.id ))

        ResetBracket ->
            withSel model (\s -> ( model, resetBracket model.api s.id ))

        SetFinalsFormat fmt ->
            withSel model (\s -> ( model, setBracketFormat model.api s.id fmt ))

        SetRoundFormat size fmt ->
            withSel model (\s -> ( model, setBracketRoundFormat model.api s.id size fmt ))

        FinalsFormatSaved (Ok _) ->
            -- Format changed → wipe the draw then re-generate so every bracket
            -- match is rescheduled with the new per-round format.
            withSel model (\s -> ( model, resetForRegen model.api s.id ))

        FinalsFormatSaved (Err e) ->
            ( { model | err = Just (httpErr e) }, Cmd.none )

        BracketResetForRegen (Ok _) ->
            withSel model
                (\s -> ( model, genBracket model.api s.id (Maybe.withDefault 2 (String.toInt s.perPool)) ))

        BracketResetForRegen (Err e) ->
            ( { model | err = Just (httpErr e) }, Cmd.none )

        SetNewTeam s ->
            ( mapSel (\s_ -> { s_ | newTeam = s }) model, Cmd.none )

        SetNewTeamName v ->
            ( mapSel (\s -> { s | newTeamName = v }) model, Cmd.none )

        SetNewTeam2 v ->
            ( mapSel (\s -> { s | newTeam2 = v }) model, Cmd.none )

        AddTeam ->
            withSel model
                (\s ->
                    let
                        name =
                            String.trim s.newTeamName
                    in
                    if name == "" then
                        ( model, Cmd.none )

                    else
                        ( mapSel (\x -> { x | newTeamName = "", newTeam = "", newTeam2 = "" }) model
                        , addTeam model.api s.id name (String.trim s.newTeam) (String.trim s.newTeam2)
                        )
                )

        SetImportText v ->
            ( mapSel (\s -> { s | importText = v }) model, Cmd.none )

        ImportList ->
            withSel model
                (\s ->
                    if String.trim s.importText == "" then
                        ( model, Cmd.none )

                    else
                        ( mapSel (\x -> { x | importText = "" }) model
                        , importTeams model.api s.id (String.lines s.importText)
                        )
                )

        SetCourts s ->
            ( mapSel (\s_ -> { s_ | courts = s }) model, Cmd.none )

        SaveCourts ->
            withSel model
                (\s ->
                    case String.toInt s.courts of
                        Just n ->
                            ( model, configureCourts model.api s.id n )

                        Nothing ->
                            ( model, Cmd.none )
                )

        GenPoolMatches poolId ->
            withSel model (\s -> ( model, genPoolMatches model.api s.id poolId ))

        AssignPoolCourt poolId courtId ->
            withSel model
                (\s ->
                    if courtId == "" then
                        ( model, Cmd.none )

                    else
                        ( model, assignPoolCourt model.api s.id poolId courtId )
                )

        DeleteTeam teamId ->
            withSel model (\s -> ( model, deleteTeam model.api s.id teamId ))

        AskForfeit teamId ->
            ( mapSel (\s -> { s | confirmForfeit = Just teamId }) model, Cmd.none )

        CancelForfeit ->
            ( mapSel (\s -> { s | confirmForfeit = Nothing }) model, Cmd.none )

        ConfirmForfeit teamId ->
            withSel model
                (\s -> ( mapSel (\x -> { x | confirmForfeit = Nothing }) model, forfeitTeam model.api s.id teamId ))

        GoStep st ->
            ( mapSel
                (\s ->
                    { s
                        | step = st
                        , numPools =
                            if st == StepPools && List.isEmpty s.view.pools then
                                String.fromInt (suggestPools (List.length s.view.teams))

                            else
                                s.numPools
                    }
                )
                model
            , Cmd.none
            )

        SetNumPools v ->
            ( mapSel (\s -> { s | numPools = v }) model, Cmd.none )

        AutoPools ->
            withSel model
                (\s ->
                    let
                        n =
                            max 1 (Maybe.withDefault 2 (String.toInt s.numPools))
                    in
                    if List.length s.view.teams < n then
                        ( model, Cmd.none )

                    else
                        ( model, postPools model.api s.id (buildPools n s.view.teams) )
                )

        ProposeIdealPools ->
            withSel model
                (\s ->
                    let
                        n =
                            suggestPools (List.length s.view.teams)
                    in
                    if List.length s.view.teams < 2 then
                        ( model, Cmd.none )

                    else
                        ( mapSel (\x -> { x | numPools = String.fromInt n }) model
                        , postPools model.api s.id (buildPools n s.view.teams)
                        )
                )

        StartPools ->
            withSel model
                (\s ->
                    ( model
                    , Cmd.batch
                        (List.map (\p -> genPoolMatches model.api s.id p.id) s.view.pools
                            ++ [ postEmpty model.api ("/tournaments/" ++ s.id ++ "/start-pools") (E.object []) ]
                        )
                    )
                )

        StartFinals ->
            withSel model (\s -> ( model, postEmpty model.api ("/tournaments/" ++ s.id ++ "/start-bracket") (E.object []) ))

        ResetTournament ->
            withSel model (\s -> ( model, postEmpty model.api ("/tournaments/" ++ s.id ++ "/reset") (E.object []) ))

        RedoPools ->
            withSel model
                (\s ->
                    ( mapSel (\x -> { x | step = StepPools }) model
                    , postEmpty model.api ("/tournaments/" ++ s.id ++ "/redo-pools") (E.object [])
                    )
                )

        DragStart teamId ->
            ( mapSel (\s -> { s | dragged = Just teamId }) model, Cmd.none )

        DropOn poolId ->
            withSel model
                (\s ->
                    case s.dragged of
                        Just tid ->
                            let
                                without t =
                                    List.filter (\x -> x /= tid) t

                                newPools =
                                    s.view.pools
                                        |> List.map
                                            (\p ->
                                                if p.id == poolId then
                                                    ( p.name, without p.teams ++ [ tid ] )

                                                else
                                                    ( p.name, without p.teams )
                                            )
                                        |> List.filter (\( _, teams ) -> not (List.isEmpty teams))
                            in
                            ( mapSel (\x -> { x | dragged = Nothing }) model
                            , postPools model.api s.id newPools
                            )

                        Nothing ->
                            ( model, Cmd.none )
                )

        NoOp ->
            ( model, Cmd.none )


        SetTeamA s ->
            ( mapSel (\s_ -> { s_ | teamA = s }) model, Cmd.none )

        SetTeamB s ->
            ( mapSel (\s_ -> { s_ | teamB = s }) model, Cmd.none )

        ScheduleMatch ->
            withSel model
                (\s ->
                    if s.teamA /= "" && s.teamB /= "" && s.teamA /= s.teamB then
                        ( model, scheduleMatch model.api s.id s.teamA s.teamB )

                    else
                        ( model, Cmd.none )
                )

        Dispatch ->
            withSel model (\s -> ( model, dispatch model.api s.id ))

        Dispatched (Ok _) ->
            ( model, refresh model )

        Dispatched (Err e) ->
            ( { model | err = Just (httpErr e) }, Cmd.none )

        StartMatch matchId courtId ->
            ( model, startMatch model.api matchId courtId )

        ToggleShowPast ->
            let
                next =
                    not model.showPast
            in
            ( { model | showPast = next }, saveShowPast next )

        SetScore matchId which v ->
            ( mapSel
                (\s ->
                    let
                        ( a, b ) =
                            Maybe.withDefault ( "", "" ) (Dict.get matchId s.scores)

                        pair =
                            if which == 0 then
                                ( v, b )

                            else
                                ( a, v )
                    in
                    { s | scores = Dict.insert matchId pair s.scores }
                )
                model
            , Cmd.none
            )

        SubmitScore matchId ->
            withSel model
                (\s ->
                    case Dict.get matchId s.scores of
                        Just ( a, b ) ->
                            case ( parseScore a, parseScore b ) of
                                ( Just na, Just nb ) ->
                                    -- Clear the inputs so the next set (best-of-3)
                                    -- starts empty.
                                    ( mapSel (\x -> { x | scores = Dict.remove matchId x.scores }) model
                                    , recordSet model.api matchId na nb
                                    )

                                _ ->
                                    ( model, Cmd.none )

                        Nothing ->
                            ( model, Cmd.none )
                )

        EditScore matchId pa pb ->
            ( mapSel
                (\s ->
                    { s
                        | editing = Just matchId
                        , scores = Dict.insert matchId ( String.fromInt pa, String.fromInt pb ) s.scores
                    }
                )
                model
            , Cmd.none
            )

        CancelEdit ->
            ( mapSel (\s -> { s | editing = Nothing }) model, Cmd.none )

        Rescore matchId ->
            withSel model
                (\s ->
                    case Dict.get matchId s.scores of
                        Just ( a, b ) ->
                            case ( parseScore a, parseScore b ) of
                                ( Just na, Just nb ) ->
                                    ( mapSel (\x -> { x | editing = Nothing }) model
                                    , rescore model.api matchId na nb
                                    )

                                _ ->
                                    ( model, Cmd.none )

                        Nothing ->
                            ( model, Cmd.none )
                )

        ResetMatch matchId ->
            ( model, resetMatch model.api matchId )

        ConcedeMatch matchId winnerId ->
            ( model, concedeMatch model.api matchId winnerId )

        UnstartMatch matchId ->
            ( model, unstartMatch model.api matchId )

        ToggleForfeit matchId ->
            ( mapSel
                (\s ->
                    { s
                        | forfeitOpen =
                            if s.forfeitOpen == Just matchId then
                                Nothing

                            else
                                Just matchId
                    }
                )
                model
            , Cmd.none
            )

        Mutated (Ok _) ->
            ( model, refresh model )

        Mutated (Err e) ->
            ( { model | err = Just (httpErr e) }, refresh model )

        Tick t ->
            ( { model | now = t }, refreshBoard model )

        GotZone z ->
            ( { model | zone = z }, Cmd.none )


{-| Keep transient input fields when a fresh TView arrives. -}
mergeView : Step -> Maybe Sel -> TView -> Sel
mergeView wantStep prev v =
    case prev of
        Just s ->
            { s | view = v }

        Nothing ->
            { id = v.id
            , view = v
            , board = { courts = [], matches = [] }
            , newTeamName = ""
            , newTeam = ""
            , newTeam2 = ""
            , importText = ""
            , courts = String.fromInt (List.length v.courts)
            , teamA = ""
            , teamB = ""
            , scores = Dict.empty
            , standings = []
            , schedule = []
            , bracket = []
            , perPool = "2"
            , step = wantStep
            , numPools = String.fromInt (suggestPools (List.length v.teams))
            , editing = Nothing
            , dragged = Nothing
            , confirmForfeit = Nothing
            , forfeitOpen = Nothing
            }


mapSel : (Sel -> Sel) -> Model -> Model
mapSel f model =
    { model | sel = Maybe.map f model.sel }


withSel : Model -> (Sel -> ( Model, Cmd Msg )) -> ( Model, Cmd Msg )
withSel model f =
    case model.sel of
        Just s ->
            f s

        Nothing ->
            ( model, Cmd.none )


openCmds : String -> String -> Cmd Msg
openCmds api id =
    Cmd.batch
        [ loadView api id
        , loadBoard api id
        , loadStandings api id
        , loadBracket api id
        , loadSchedule api id
        ]


refresh : Model -> Cmd Msg
refresh model =
    case model.sel of
        Just s ->
            openCmds model.api s.id

        Nothing ->
            Cmd.none


refreshBoard : Model -> Cmd Msg
refreshBoard model =
    case model.sel of
        Just s ->
            Cmd.batch
                [ loadBoard model.api s.id
                , loadStandings model.api s.id
                , loadBracket model.api s.id
                , loadSchedule model.api s.id
                ]

        Nothing ->
            Cmd.none



-- SUBSCRIPTIONS


subscriptions : Model -> Sub Msg
subscriptions model =
    case model.sel of
        Just _ ->
            Time.every 3000 Tick

        Nothing ->
            Sub.none



-- MAIN


main : Program Flags Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , view = view
        , subscriptions = subscriptions
        }
