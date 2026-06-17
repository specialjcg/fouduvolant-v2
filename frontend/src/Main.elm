port module Main exposing (main)

{-| Fou du Volant — Elm MVU frontend over the axum HTTP API.

Two screens: the tournament list (with a create form) and a selected
tournament's setup + live board. The board polls every few seconds and after
every mutation, so it reflects the event-sourced backend.
-}

import Browser
import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (on, onClick, onInput, preventDefaultOn)
import Http
import Json.Decode as D
import Json.Encode as E
import Task
import Time



-- PORTS


{-| Persist the "show past matches" toggle to localStorage (read back via flags). -}
port saveShowPast : Bool -> Cmd msg



-- MODEL


type alias Flags =
    { apiBase : String, open : String, showPast : Bool }


type alias Model =
    { api : String
    , tournaments : List Summary
    , sel : Maybe Sel
    , newName : String
    , err : Maybe String
    , wantStep : Step
    , showPast : Bool
    , now : Time.Posix
    , zone : Time.Zone
    }


{-| State of the currently selected tournament. -}
type alias Sel =
    { id : String
    , view : TView
    , board : Board
    , newTeamName : String
    , newTeam : String
    , newTeam2 : String
    , importText : String
    , courts : String
    , teamA : String
    , teamB : String
    , scores : Dict String ( String, String )
    , standings : List PoolStandings
    , schedule : List ForecastCourt
    , bracket : List BracketNode
    , perPool : String
    , step : Step
    , numPools : String
    , editing : Maybe String
    , dragged : Maybe String
    }


type Step
    = StepTeams
    | StepPools
    | StepBoard
    | StepSchedule
    | StepFinals
    | StepRanking


type alias BracketNode =
    { kind : String
    , round : Int
    , index : Int
    , teamA : Maybe String
    , teamB : Maybe String
    , winner : Maybe String
    , feeds : Maybe Int
    }


type alias PoolStandings =
    { poolId : String, name : String, rows : List StandingRow }


type alias StandingRow =
    { name : String, rank : Int, played : Int, wins : Int, pf : Int, pa : Int, diff : Int }


type alias Summary =
    { id : String, name : String, phase : String }


type alias Team =
    { id : String, name : String, player1 : String, player2 : String }


type alias TView =
    { id : String
    , name : String
    , phase : String
    , teams : List Team
    , pools : List PoolV
    , courts : List String
    , poolCourts : List PoolCourt
    , bracketFormat : String
    , roundFormats : Dict String String
    }


type alias PoolV =
    { id : String, name : String, teams : List String }


type alias PoolCourt =
    { pool : String, court : String }


type alias Board =
    { courts : List CourtPlan, matches : List MatchV }


type alias ForecastCourt =
    { court : String, matches : List ForecastMatch }


type alias ForecastMatch =
    { id : String
    , teamA : String
    , teamB : String
    , pool : Maybe String
    , status : String
    , pointsA : Int
    , pointsB : Int
    , etaMin : Int
    }


type alias CourtPlan =
    { court : String, current : Maybe String, next : Maybe Sugg, previews : List Sugg }


type alias Sugg =
    { matchId : String, needsRest : Bool }


type alias MatchV =
    { id : String
    , teamA : String
    , teamB : String
    , status : String
    , court : Maybe String
    , doneOrder : Maybe Int
    , pointsA : Int
    , pointsB : Int
    , pool : Maybe String
    , sets : List ( Int, Int )
    }


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


stepFromString : String -> Step
stepFromString s =
    case s of
        "poules" ->
            StepPools

        "terrains" ->
            StepBoard

        "previsionnel" ->
            StepSchedule

        "finales" ->
            StepFinals

        "classement" ->
            StepRanking

        _ ->
            StepTeams



-- UPDATE


type Msg
    = GotTournaments (Result Http.Error (List Summary))
    | SetNewName String
    | CreateTournament
    | Created (Result Http.Error String)
    | OpenT String
    | DeleteTournament String
    | Deleted (Result Http.Error ())
    | CloseT
    | GotView (Result Http.Error TView)
    | GotBoard (Result Http.Error Board)
    | GotStandings (Result Http.Error (List PoolStandings))
    | GotSchedule (Result Http.Error (List ForecastCourt))
    | GotBracket (Result Http.Error (List BracketNode))
    | SetPerPool String
    | GenBracket
    | AdvanceBracket
    | ResetBracket
    | SetFinalsFormat String
    | SetRoundFormat Int String
    | FinalsFormatSaved (Result Http.Error ())
    | BracketResetForRegen (Result Http.Error ())
    | SetNewTeamName String
    | SetNewTeam String
    | SetNewTeam2 String
    | AddTeam
    | SetImportText String
    | ImportList
    | DeleteTeam String
    | GoStep Step
    | SetNumPools String
    | AutoPools
    | StartPools
    | StartFinals
    | ResetTournament
    | DragStart String
    | DropOn String
    | NoOp
    | SetCourts String
    | SaveCourts
    | GenPoolMatches String
    | AssignPoolCourt String String
    | SetTeamA String
    | SetTeamB String
    | ScheduleMatch
    | Dispatch
    | Dispatched (Result Http.Error (List String))
    | StartMatch String String
    | SetScore String Int String
    | SubmitScore String
    | EditScore String Int Int
    | CancelEdit
    | Rescore String
    | ResetMatch String
    | Mutated (Result Http.Error ())
    | Tick Time.Posix
    | GotZone Time.Zone
    | ToggleShowPast


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



-- HTTP


loadTournaments : String -> Cmd Msg
loadTournaments api =
    Http.get { url = api ++ "/tournaments", expect = Http.expectJson GotTournaments (D.list summaryDec) }


deleteTournament : String -> String -> Cmd Msg
deleteTournament api id =
    Http.request
        { method = "DELETE"
        , headers = []
        , url = api ++ "/tournaments/" ++ id
        , body = Http.emptyBody
        , expect = Http.expectWhatever Deleted
        , timeout = Nothing
        , tracker = Nothing
        }


loadView : String -> String -> Cmd Msg
loadView api id =
    Http.get { url = api ++ "/tournaments/" ++ id, expect = Http.expectJson GotView tviewDec }


loadBoard : String -> String -> Cmd Msg
loadBoard api id =
    Http.get { url = api ++ "/tournaments/" ++ id ++ "/board", expect = Http.expectJson GotBoard boardDec }


loadStandings : String -> String -> Cmd Msg
loadStandings api id =
    Http.get
        { url = api ++ "/tournaments/" ++ id ++ "/standings"
        , expect = Http.expectJson GotStandings (D.list poolStandingsDec)
        }


loadSchedule : String -> String -> Cmd Msg
loadSchedule api id =
    Http.get
        { url = api ++ "/tournaments/" ++ id ++ "/schedule"
        , expect = Http.expectJson GotSchedule (D.list forecastCourtDec)
        }


createTournament : String -> String -> Cmd Msg
createTournament api name =
    Http.post
        { url = api ++ "/tournaments"
        , body =
            Http.jsonBody
                (E.object
                    [ ( "name", E.string name )
                    , ( "pool_format", E.string "BestOf1" )
                    , ( "bracket_format", E.string "BestOf1" )
                    ]
                )
        , expect = Http.expectJson Created (D.field "id" D.string)
        }


addTeam : String -> String -> String -> String -> String -> Cmd Msg
addTeam api tid name player1 player2 =
    postEmpty api
        ("/tournaments/" ++ tid ++ "/teams")
        (E.object
            [ ( "name", E.string name )
            , ( "player1", E.string player1 )
            , ( "player2", E.string player2 )
            ]
        )


importTeams : String -> String -> List String -> Cmd Msg
importTeams api tid names =
    postEmpty api
        ("/tournaments/" ++ tid ++ "/teams/import")
        (E.object [ ( "names", E.list E.string names ) ])


configureCourts : String -> String -> Int -> Cmd Msg
configureCourts api tid n =
    postEmpty api ("/tournaments/" ++ tid ++ "/courts") (E.object [ ( "count", E.int n ) ])


loadBracket : String -> String -> Cmd Msg
loadBracket api id =
    Http.get
        { url = api ++ "/tournaments/" ++ id ++ "/bracket"
        , expect = Http.expectJson GotBracket (D.list bracketNodeDec)
        }


genBracket : String -> String -> Int -> Cmd Msg
genBracket api tid perPool =
    postEmpty api ("/tournaments/" ++ tid ++ "/bracket") (E.object [ ( "per_pool", E.int perPool ) ])


advBracket : String -> String -> Cmd Msg
advBracket api tid =
    Http.post
        { url = api ++ "/tournaments/" ++ tid ++ "/bracket/advance"
        , body = Http.emptyBody
        , expect = Http.expectWhatever Mutated
        }


resetBracket : String -> String -> Cmd Msg
resetBracket api tid =
    Http.post
        { url = api ++ "/tournaments/" ++ tid ++ "/bracket/reset"
        , body = Http.emptyBody
        , expect = Http.expectWhatever Mutated
        }


setBracketFormat : String -> String -> String -> Cmd Msg
setBracketFormat api tid fmt =
    Http.post
        { url = api ++ "/tournaments/" ++ tid ++ "/bracket-format"
        , body = Http.jsonBody (E.object [ ( "format", E.string fmt ) ])
        , expect = Http.expectWhatever FinalsFormatSaved
        }


setBracketRoundFormat : String -> String -> Int -> String -> Cmd Msg
setBracketRoundFormat api tid size fmt =
    Http.post
        { url = api ++ "/tournaments/" ++ tid ++ "/bracket-round-format"
        , body = Http.jsonBody (E.object [ ( "round_size", E.int size ), ( "format", E.string fmt ) ])
        , expect = Http.expectWhatever FinalsFormatSaved
        }


resetForRegen : String -> String -> Cmd Msg
resetForRegen api tid =
    Http.post
        { url = api ++ "/tournaments/" ++ tid ++ "/bracket/reset"
        , body = Http.emptyBody
        , expect = Http.expectWhatever BracketResetForRegen
        }


deleteTeam : String -> String -> String -> Cmd Msg
deleteTeam api tid teamId =
    Http.request
        { method = "DELETE"
        , headers = []
        , url = api ++ "/tournaments/" ++ tid ++ "/teams/" ++ teamId
        , body = Http.emptyBody
        , expect = Http.expectWhatever Mutated
        , timeout = Nothing
        , tracker = Nothing
        }


{-| Suggested pool count: aim for pools of about 6 teams. -}
suggestPools : Int -> Int
suggestPools teams =
    Basics.max 1 ((teams + 5) // 6)


{-| Distribute teams round-robin into `n` balanced pools (sizes differ by ≤1). -}
buildPools : Int -> List Team -> List ( String, List String )
buildPools n teams =
    let
        indexed =
            List.indexedMap Tuple.pair teams
    in
    List.range 0 (n - 1)
        |> List.map
            (\k ->
                let
                    members =
                        indexed
                            |> List.filter (\( i, _ ) -> modBy n i == k)
                            |> List.map (\( _, t ) -> t.id)
                in
                ( "Poule " ++ String.fromChar (Char.fromCode (65 + k)), members )
            )


postPools : String -> String -> List ( String, List String ) -> Cmd Msg
postPools api tid pools =
    postEmpty api
        ("/tournaments/" ++ tid ++ "/pools")
        (E.object
            [ ( "pools"
              , E.list
                    (\( name, teams ) ->
                        E.object
                            [ ( "name", E.string name )
                            , ( "teams", E.list E.string teams )
                            ]
                    )
                    pools
              )
            ]
        )


genPoolMatches : String -> String -> String -> Cmd Msg
genPoolMatches api tid poolId =
    Http.post
        { url = api ++ "/tournaments/" ++ tid ++ "/pools/" ++ poolId ++ "/matches"
        , body = Http.emptyBody
        , expect = Http.expectWhatever Mutated
        }


assignPoolCourt : String -> String -> String -> String -> Cmd Msg
assignPoolCourt api tid poolId courtId =
    postEmpty api
        ("/tournaments/" ++ tid ++ "/pools/" ++ poolId ++ "/court")
        (E.object [ ( "court_id", E.string courtId ) ])


scheduleMatch : String -> String -> String -> String -> Cmd Msg
scheduleMatch api tid a b =
    postEmpty api
        ("/tournaments/" ++ tid ++ "/matches")
        (E.object
            [ ( "format", E.string "BestOf1" )
            , ( "team_a", E.string a )
            , ( "team_b", E.string b )
            ]
        )


startMatch : String -> String -> String -> Cmd Msg
startMatch api matchId courtId =
    postEmpty api ("/matches/" ++ matchId ++ "/start") (E.object [ ( "court_id", E.string courtId ) ])


recordSet : String -> String -> Int -> Int -> Cmd Msg
recordSet api matchId a b =
    postEmpty api ("/matches/" ++ matchId ++ "/sets") (E.object [ ( "a", E.int a ), ( "b", E.int b ) ])


rescore : String -> String -> Int -> Int -> Cmd Msg
rescore api matchId a b =
    postEmpty api ("/matches/" ++ matchId ++ "/rescore") (E.object [ ( "a", E.int a ), ( "b", E.int b ) ])


resetMatch : String -> String -> Cmd Msg
resetMatch api matchId =
    Http.post
        { url = api ++ "/matches/" ++ matchId ++ "/reset"
        , body = Http.emptyBody
        , expect = Http.expectWhatever Mutated
        }


dispatch : String -> String -> Cmd Msg
dispatch api tid =
    Http.post
        { url = api ++ "/tournaments/" ++ tid ++ "/dispatch"
        , body = Http.emptyBody
        , expect = Http.expectJson Dispatched (D.field "started" (D.list D.string))
        }


{-| POST a JSON body to an endpoint whose success body we ignore. -}
postEmpty : String -> String -> E.Value -> Cmd Msg
postEmpty api path body =
    Http.post { url = api ++ path, body = Http.jsonBody body, expect = Http.expectWhatever Mutated }



-- DECODERS


summaryDec : D.Decoder Summary
summaryDec =
    D.map3 Summary (D.field "id" D.string) (D.field "name" D.string) (D.field "phase" D.string)


teamDec : D.Decoder Team
teamDec =
    D.map4 Team
        (D.field "id" D.string)
        (D.field "name" D.string)
        (D.field "player1" D.string)
        (D.field "player2" D.string)


tviewDec : D.Decoder TView
tviewDec =
    D.succeed TView
        |> andMap (D.field "id" D.string)
        |> andMap (D.field "name" D.string)
        |> andMap (D.field "phase" D.string)
        |> andMap (D.field "teams" (D.list teamDec))
        |> andMap (D.field "pools" (D.list poolDec))
        |> andMap (D.field "courts" (D.list D.string))
        |> andMap (D.field "pool_courts" (D.list (D.map2 PoolCourt (D.field "pool" D.string) (D.field "court" D.string))))
        |> andMap (D.oneOf [ D.field "bracket_format" D.string, D.succeed "BestOf1" ])
        |> andMap (D.oneOf [ D.field "bracket_round_formats" (D.dict D.string), D.succeed Dict.empty ])


poolDec : D.Decoder PoolV
poolDec =
    D.map3 PoolV
        (D.field "id" D.string)
        (D.field "name" D.string)
        (D.field "teams" (D.list D.string))


boardDec : D.Decoder Board
boardDec =
    D.map2 Board
        (D.field "courts" (D.list courtPlanDec))
        (D.field "matches" (D.list matchVDec))


courtPlanDec : D.Decoder CourtPlan
courtPlanDec =
    D.map4 CourtPlan
        (D.field "court" D.string)
        (D.field "current" (D.nullable D.string))
        (D.field "next" (D.nullable suggDec))
        (D.field "previews" (D.list suggDec))


suggDec : D.Decoder Sugg
suggDec =
    D.map2 Sugg (D.field "match_id" D.string) (D.field "needs_rest" D.bool)


bracketNodeDec : D.Decoder BracketNode
bracketNodeDec =
    D.map7 BracketNode
        (D.field "kind" D.string)
        (D.field "round" D.int)
        (D.field "index" D.int)
        (D.field "team_a" (D.nullable D.string))
        (D.field "team_b" (D.nullable D.string))
        (D.field "winner" (D.nullable D.string))
        (D.field "feeds" (D.nullable D.int))


forecastCourtDec : D.Decoder ForecastCourt
forecastCourtDec =
    D.map2 ForecastCourt
        (D.field "court" D.string)
        (D.field "matches" (D.list forecastMatchDec))


forecastMatchDec : D.Decoder ForecastMatch
forecastMatchDec =
    D.map8 ForecastMatch
        (D.field "id" D.string)
        (D.field "team_a" D.string)
        (D.field "team_b" D.string)
        (D.field "pool" (D.nullable D.string))
        (D.field "status" D.string)
        (D.field "points_a" D.int)
        (D.field "points_b" D.int)
        (D.field "eta_min" D.int)


poolStandingsDec : D.Decoder PoolStandings
poolStandingsDec =
    D.map3 PoolStandings
        (D.field "pool_id" D.string)
        (D.field "name" D.string)
        (D.field "rows" (D.list rowDec))


rowDec : D.Decoder StandingRow
rowDec =
    D.map7 StandingRow
        (D.field "name" D.string)
        (D.field "rank" D.int)
        (D.field "played" D.int)
        (D.field "wins" D.int)
        (D.field "points_for" D.int)
        (D.field "points_against" D.int)
        (D.field "diff" D.int)


andMap : D.Decoder a -> D.Decoder (a -> b) -> D.Decoder b
andMap =
    D.map2 (|>)


matchVDec : D.Decoder MatchV
matchVDec =
    D.succeed MatchV
        |> andMap (D.field "id" D.string)
        |> andMap (D.field "team_a" D.string)
        |> andMap (D.field "team_b" D.string)
        |> andMap (D.field "status" D.string)
        |> andMap (D.field "court" (D.nullable D.string))
        |> andMap (D.field "done_order" (D.nullable D.int))
        |> andMap (D.field "points_a" D.int)
        |> andMap (D.field "points_b" D.int)
        |> andMap (D.field "pool" (D.nullable D.string))
        |> andMap (D.field "sets" (D.list (D.map2 Tuple.pair (D.index 0 D.int) (D.index 1 D.int))))



-- VIEW


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


viewPools : Sel -> Html Msg
viewPools s =
    div [ class "panel" ]
        [ h2 [] [ text "Poules & terrains" ]
        , div [ class "row" ]
            [ text "Terrains :"
            , input [ type_ "number", class "score", value s.courts, onInput SetCourts ] []
            , button [ class "secondary", onClick SaveCourts ] [ text "Définir" ]
            ]
        , h3 [] [ text "Répartition" ]
        , div [ class "row" ]
            [ text "Nombre de poules :"
            , input [ type_ "number", class "score", value s.numPools, onInput SetNumPools ] []
            , button [ onClick AutoPools, disabled (List.length s.view.teams < 2) ]
                [ text "Répartir automatiquement" ]
            ]
        , if List.isEmpty s.view.pools then
            p [ class "muted" ] [ text "Aucune poule. Répartis les équipes ci-dessus." ]

          else
            let
                names =
                    teamNames s.view.teams

                assignedOf pid =
                    s.view.poolCourts
                        |> List.filter (\pc -> pc.pool == pid)
                        |> List.head
                        |> Maybe.map .court
            in
            div []
                [ if s.view.phase == "Draft" then
                    p [ class "muted", Html.Attributes.style "font-size" ".82rem" ]
                        [ text "Glisser-déposer une équipe d'une poule à l'autre pour rééquilibrer." ]

                  else
                    text ""
                , div []
                    (List.map
                        (\pp -> poolRow (s.view.phase == "Draft") names s.view.courts s.board.matches (assignedOf pp.id) (rankedPoolTeams names s.standings pp) pp)
                        s.view.pools
                    )
                ]
        , div [ class "row", Html.Attributes.style "margin-top" "1rem" ]
            [ button
                [ onClick StartPools
                , disabled (List.isEmpty s.view.pools || List.isEmpty s.view.courts)
                ]
                [ text "Lancer les poules" ]
            , button [ class "secondary", onClick (GoStep StepBoard) ] [ text "Terrains →" ]
            , button [ class "danger", onClick ResetTournament ] [ text "Réinitialiser (relancer à vide)" ]
            ]
        ]


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
                , bracketTree "Principal" (List.filter (\n -> n.kind == "Main") s.bracket)
                , bracketTree "Consolante" (List.filter (\n -> n.kind == "Consolation") s.bracket)
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
bracketTree : String -> List BracketNode -> Html Msg
bracketTree title nodes =
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
                                ++ [ posBox (xOf 0) sy brkBarH n ]
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
                            conn ++ [ posBox (xOf (colOf r)) sy brkBoxH n ]
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
                        :: List.map plainBox thirdNodes
                    )
            ]


brkCell : Float
brkCell =
    72


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


posBox : Float -> Float -> Float -> BracketNode -> Html Msg
posBox x cyc h n =
    div
        [ class "bmatch"
        , Html.Attributes.style "position" "absolute"
        , Html.Attributes.style "left" (px x)
        , Html.Attributes.style "top" (px (cyc - h / 2))
        , Html.Attributes.style "width" (px brkBoxW)
        , Html.Attributes.style "min-height" (px h)
        ]
        [ seedRow n.teamA n.winner, seedRow n.teamB n.winner ]


plainBox : BracketNode -> Html Msg
plainBox n =
    div [ class "bmatch", Html.Attributes.style "width" (px brkBoxW) ]
        [ seedRow n.teamA n.winner, seedRow n.teamB n.winner ]


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


viewStandings : Sel -> Html Msg
viewStandings s =
    if List.isEmpty s.standings then
        text ""

    else
        div [ class "panel" ]
            (h2 [] [ text "Classement" ]
                :: List.map standingsTable s.standings
            )


standingsTable : PoolStandings -> Html Msg
standingsTable ps =
    div []
        [ h3 [ class "muted" ] [ text ps.name ]
        , table []
            (tr []
                [ th [] [ text "#" ]
                , th [] [ text "Équipe" ]
                , th [] [ text "J" ]
                , th [] [ text "V" ]
                , th [] [ text "Pts+" ]
                , th [] [ text "Pts-" ]
                , th [] [ text "Diff" ]
                ]
                :: List.map standingsRow ps.rows
            )
        ]


standingsRow : StandingRow -> Html Msg
standingsRow r =
    tr []
        [ td [] [ text (String.fromInt r.rank) ]
        , td [] [ text r.name ]
        , td [] [ text (String.fromInt r.played) ]
        , td [] [ text (String.fromInt r.wins) ]
        , td [] [ text (String.fromInt r.pf) ]
        , td [] [ text (String.fromInt r.pa) ]
        , td [] [ text (String.fromInt r.diff) ]
        ]


poolRow : Bool -> Dict String String -> List String -> List MatchV -> Maybe String -> List String -> PoolV -> Html Msg
poolRow editable names courts matches assigned ranked p =
    let
        dropZone =
            if editable then
                [ preventDefaultOn "dragover" (D.succeed ( NoOp, True ))
                , preventDefaultOn "drop" (D.succeed ( DropOn p.id, True ))
                ]

            else
                []
    in
    div (class "match" :: dropZone)
        [ div [ class "row", Html.Attributes.style "justify-content" "space-between" ]
            [ span [ Html.Attributes.style "font-weight" "600" ] [ text p.name ]
            , courtSelect courts assigned p.id
            ]
        , if editable then
            div [ class "row", Html.Attributes.style "flex-wrap" "wrap", Html.Attributes.style "margin-top" ".4rem" ]
                (List.map (teamChip names) p.teams)

          else
            poolMatrix names matches ranked
        ]


teamChip : Dict String String -> String -> Html Msg
teamChip names tid =
    span
        [ class "chip"
        , Html.Attributes.draggable "true"
        , on "dragstart" (D.succeed (DragStart tid))
        ]
        [ text (nameOf names tid) ]


{-| Cross table of a pool's matches (équipe × équipe), score in each cell. -}
poolMatrix : Dict String String -> List MatchV -> List String -> Html Msg
poolMatrix names matches teams =
    if List.length teams < 2 then
        text ""

    else
        table [ Html.Attributes.style "margin-top" ".5rem" ]
            (tr []
                (th [] [ text "" ]
                    :: List.map (\t -> th [] [ text (shortName (nameOf names t)) ]) teams
                    ++ [ th [] [ text "V" ], th [] [ text "D" ], th [] [ text "Pts" ], th [] [ text "Diff" ] ]
                )
                :: List.map (matrixRow names matches teams) teams
            )


{-| Pool team ids ordered by the server-computed standings (BWF tiebreakers);
falls back to pool order for teams missing from the standings. -}
rankedPoolTeams : Dict String String -> List PoolStandings -> PoolV -> List String
rankedPoolTeams names standings p =
    case List.head (List.filter (\ps -> ps.poolId == p.id) standings) of
        Just ps ->
            let
                idFor nm =
                    List.head (List.filter (\tid -> nameOf names tid == nm) p.teams)

                ordered =
                    List.filterMap (\row -> idFor row.name) ps.rows

                rest =
                    List.filter (\tid -> not (List.member tid ordered)) p.teams
            in
            ordered ++ rest

        Nothing ->
            p.teams


matrixRow : Dict String String -> List MatchV -> List String -> String -> Html Msg
matrixRow names matches teams ti =
    let
        cell n =
            td [ Html.Attributes.style "text-align" "center", Html.Attributes.style "font-weight" "600" ]
                [ text (String.fromInt n) ]

        stat =
            teamStats matches teams ti
    in
    tr []
        (td [ Html.Attributes.style "font-weight" "600" ] [ text (nameOf names ti) ]
            :: List.map
                (\tj ->
                    td [ Html.Attributes.style "text-align" "center" ]
                        [ text
                            (if ti == tj then
                                "—"

                             else
                                scoreBetween matches ti tj
                            )
                        ]
                )
                teams
            ++ [ cell stat.w
               , cell stat.l
               , cell stat.pf
               , td [ Html.Attributes.style "text-align" "center", Html.Attributes.style "font-weight" "600" ]
                    [ text (signed (stat.pf - stat.pa)) ]
               ]
        )


type alias TeamStat =
    { w : Int, l : Int, pf : Int, pa : Int }


{-| Wins / losses / points-for / points-against of `ti` over its played pool matches. -}
teamStats : List MatchV -> List String -> String -> TeamStat
teamStats matches teams ti =
    List.foldl
        (\tj acc ->
            case playedScore matches ti tj of
                Just ( mine, opp ) ->
                    { w =
                        acc.w
                            + (if mine > opp then
                                1

                               else
                                0
                              )
                    , l =
                        acc.l
                            + (if mine < opp then
                                1

                               else
                                0
                              )
                    , pf = acc.pf + mine
                    , pa = acc.pa + opp
                    }

                Nothing ->
                    acc
        )
        { w = 0, l = 0, pf = 0, pa = 0 }
        teams


{-| Points of `i` vs `j` (own, opponent) when the match is actually played. -}
playedScore : List MatchV -> String -> String -> Maybe ( Int, Int )
playedScore matches i j =
    if i == j then
        Nothing

    else
        case List.head (List.filter (\m -> ( m.teamA, m.teamB ) == ( i, j ) || ( m.teamA, m.teamB ) == ( j, i )) matches) of
            Just m ->
                if m.pointsA == 0 && m.pointsB == 0 && m.status /= "Done" then
                    Nothing

                else if m.teamA == i then
                    Just ( m.pointsA, m.pointsB )

                else
                    Just ( m.pointsB, m.pointsA )

            Nothing ->
                Nothing


signed : Int -> String
signed n =
    if n > 0 then
        "+" ++ String.fromInt n

    else
        String.fromInt n


scoreBetween : List MatchV -> String -> String -> String
scoreBetween matches i j =
    case List.head (List.filter (\m -> ( m.teamA, m.teamB ) == ( i, j ) || ( m.teamA, m.teamB ) == ( j, i )) matches) of
        Just m ->
            if m.pointsA == 0 && m.pointsB == 0 && m.status /= "Done" then
                ""

            else if m.teamA == i then
                String.fromInt m.pointsA ++ "-" ++ String.fromInt m.pointsB

            else
                String.fromInt m.pointsB ++ "-" ++ String.fromInt m.pointsA

        Nothing ->
            ""


{-| Parse a score input: empty means 0 (e.g. a 21-0 win). -}
parseScore : String -> Maybe Int
parseScore s =
    if String.trim s == "" then
        Just 0

    else
        String.toInt (String.trim s)


shortName : String -> String
shortName n =
    if String.length n <= 10 then
        n

    else
        String.left 9 n ++ "…"


courtSelect : List String -> Maybe String -> String -> Html Msg
courtSelect courts assigned poolId =
    Html.select [ onInput (AssignPoolCourt poolId) ]
        (option [ value "" ] [ text "— terrain —" ]
            :: List.indexedMap
                (\i c ->
                    option [ value c, Html.Attributes.selected (assigned == Just c) ]
                        [ text ("Terrain " ++ String.fromInt (i + 1)) ]
                )
                courts
        )


viewBoard : Bool -> Sel -> Dict String String -> Html Msg
viewBoard showPast s names =
    div [ class "panel" ]
        [ div [ class "row" ]
            [ h2 [] [ text "Terrains" ]
            , button [ onClick Dispatch ] [ text "⟳ Dispatch auto" ]
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
                |> Maybe.map (\( m, sg ) -> suggestNode freeCourts names cp.court m sg)
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
                        [ text (setsLabel m) ]
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
        ]


suggestNode : List ( Int, String ) -> Dict String String -> String -> MatchV -> Sugg -> Html Msg
suggestNode freeCourts names court m sg =
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



-- HELPERS


teamNames : List Team -> Dict String String
teamNames teams =
    Dict.fromList (List.map (\t -> ( t.id, t.name )) teams)


nameOf : Dict String String -> String -> String
nameOf names id =
    Dict.get id names |> Maybe.withDefault (String.left 4 id)


matchLabel : Dict String String -> MatchV -> String
matchLabel names m =
    nameOf names m.teamA ++ " vs " ++ nameOf names m.teamB


findMatch : List MatchV -> String -> Maybe MatchV
findMatch matches id =
    List.filter (\m -> m.id == id) matches |> List.head


httpErr : Http.Error -> String
httpErr e =
    case e of
        Http.BadStatus code ->
            "Erreur serveur (" ++ String.fromInt code ++ ")"

        Http.BadBody b ->
            "Réponse invalide : " ++ b

        Http.NetworkError ->
            "Erreur réseau (backend démarré ?)"

        Http.Timeout ->
            "Délai dépassé"

        Http.BadUrl u ->
            "URL invalide : " ++ u



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
