module Main exposing (main)

{-| Fou du Volant — Elm MVU frontend over the axum HTTP API.

Two screens: the tournament list (with a create form) and a selected
tournament's setup + live board. The board polls every few seconds and after
every mutation, so it reflects the event-sourced backend.
-}

import Browser
import Dict exposing (Dict)
import Html exposing (..)
import Html.Attributes exposing (class, disabled, placeholder, type_, value)
import Html.Events exposing (onClick, onInput)
import Http
import Json.Decode as D
import Json.Encode as E
import Time



-- MODEL


type alias Flags =
    { apiBase : String, open : String }


type alias Model =
    { api : String
    , tournaments : List Summary
    , sel : Maybe Sel
    , newName : String
    , err : Maybe String
    }


{-| State of the currently selected tournament. -}
type alias Sel =
    { id : String
    , view : TView
    , board : Board
    , newTeam : String
    , courts : String
    , teamA : String
    , teamB : String
    , scores : Dict String ( String, String )
    , standings : List PoolStandings
    , bracket : List BracketNode
    , perPool : String
    , step : Step
    , numPools : String
    }


type Step
    = StepTeams
    | StepPools
    | StepBoard
    | StepFinals
    | StepRanking


type alias BracketNode =
    { kind : String
    , round : Int
    , index : Int
    , teamA : Maybe String
    , teamB : Maybe String
    , winner : Maybe String
    }


type alias PoolStandings =
    { poolId : String, name : String, rows : List StandingRow }


type alias StandingRow =
    { name : String, rank : Int, played : Int, wins : Int, pf : Int, pa : Int, diff : Int }


type alias Summary =
    { id : String, name : String, phase : String }


type alias Team =
    { id : String, name : String }


type alias TView =
    { id : String
    , name : String
    , phase : String
    , teams : List Team
    , pools : List PoolV
    , courts : List String
    , poolCourts : List PoolCourt
    }


type alias PoolV =
    { id : String, name : String, teams : List String }


type alias PoolCourt =
    { pool : String, court : String }


type alias Board =
    { courts : List CourtPlan, matches : List MatchV }


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
    }


init : Flags -> ( Model, Cmd Msg )
init flags =
    ( { api = flags.apiBase
      , tournaments = []
      , sel = Nothing
      , newName = ""
      , err = Nothing
      }
    , Cmd.batch
        [ loadTournaments flags.apiBase
        , if flags.open == "" then
            Cmd.none

          else
            openCmds flags.apiBase flags.open
        ]
    )



-- UPDATE


type Msg
    = GotTournaments (Result Http.Error (List Summary))
    | SetNewName String
    | CreateTournament
    | Created (Result Http.Error String)
    | OpenT String
    | CloseT
    | GotView (Result Http.Error TView)
    | GotBoard (Result Http.Error Board)
    | GotStandings (Result Http.Error (List PoolStandings))
    | GotBracket (Result Http.Error (List BracketNode))
    | SetPerPool String
    | GenBracket
    | AdvanceBracket
    | SetNewTeam String
    | AddTeam
    | DeleteTeam String
    | GoStep Step
    | SetNumPools String
    | AutoPools
    | StartPools
    | StartFinals
    | IncScore String Int Int
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
    | Mutated (Result Http.Error ())
    | Tick Time.Posix


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

        CloseT ->
            ( { model | sel = Nothing }, loadTournaments model.api )

        GotView (Ok v) ->
            ( { model | sel = Just (mergeView model.sel v), err = Nothing }, Cmd.none )

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

        SetNewTeam s ->
            ( mapSel (\s_ -> { s_ | newTeam = s }) model, Cmd.none )

        AddTeam ->
            withSel model
                (\s ->
                    if String.trim s.newTeam == "" then
                        ( model, Cmd.none )

                    else
                        ( mapSel (\x -> { x | newTeam = "" }) model
                        , addTeam model.api s.id s.newTeam
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
            ( mapSel (\s -> { s | step = st }) model, Cmd.none )

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
            withSel model (\s -> ( model, postEmpty model.api ("/tournaments/" ++ s.id ++ "/start-pools") (E.object []) ))

        StartFinals ->
            withSel model (\s -> ( model, postEmpty model.api ("/tournaments/" ++ s.id ++ "/start-bracket") (E.object []) ))

        IncScore matchId which delta ->
            ( mapSel
                (\s ->
                    let
                        ( a, b ) =
                            Maybe.withDefault ( "", "" ) (Dict.get matchId s.scores)

                        clamp v =
                            String.fromInt (Basics.max 0 (Basics.min 30 (Maybe.withDefault 0 (String.toInt v) + delta)))

                        pair =
                            if which == 0 then
                                ( clamp a, b )

                            else
                                ( a, clamp b )
                    in
                    { s | scores = Dict.insert matchId pair s.scores }
                )
                model
            , Cmd.none
            )

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
                            case ( String.toInt a, String.toInt b ) of
                                ( Just na, Just nb ) ->
                                    ( model, recordSet model.api matchId na nb )

                                _ ->
                                    ( model, Cmd.none )

                        Nothing ->
                            ( model, Cmd.none )
                )

        Mutated (Ok _) ->
            ( model, refresh model )

        Mutated (Err e) ->
            ( { model | err = Just (httpErr e) }, refresh model )

        Tick _ ->
            ( model, refreshBoard model )


{-| Keep transient input fields when a fresh TView arrives. -}
mergeView : Maybe Sel -> TView -> Sel
mergeView prev v =
    case prev of
        Just s ->
            { s | view = v }

        Nothing ->
            { id = v.id
            , view = v
            , board = { courts = [], matches = [] }
            , newTeam = ""
            , courts = String.fromInt (List.length v.courts)
            , teamA = ""
            , teamB = ""
            , scores = Dict.empty
            , standings = []
            , bracket = []
            , perPool = "2"
            , step = StepTeams
            , numPools = "2"
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
    Cmd.batch [ loadView api id, loadBoard api id, loadStandings api id, loadBracket api id ]


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
                ]

        Nothing ->
            Cmd.none



-- HTTP


loadTournaments : String -> Cmd Msg
loadTournaments api =
    Http.get { url = api ++ "/tournaments", expect = Http.expectJson GotTournaments (D.list summaryDec) }


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


createTournament : String -> String -> Cmd Msg
createTournament api name =
    Http.post
        { url = api ++ "/tournaments"
        , body =
            Http.jsonBody
                (E.object
                    [ ( "name", E.string name )
                    , ( "pool_format", E.string "BestOf1" )
                    , ( "bracket_format", E.string "BestOf3" )
                    ]
                )
        , expect = Http.expectJson Created (D.field "id" D.string)
        }


addTeam : String -> String -> String -> Cmd Msg
addTeam api tid name =
    postEmpty api ("/tournaments/" ++ tid ++ "/teams") (E.object [ ( "name", E.string name ) ])


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
    D.map2 Team (D.field "id" D.string) (D.field "name" D.string)


tviewDec : D.Decoder TView
tviewDec =
    D.map7 TView
        (D.field "id" D.string)
        (D.field "name" D.string)
        (D.field "phase" D.string)
        (D.field "teams" (D.list teamDec))
        (D.field "pools" (D.list poolDec))
        (D.field "courts" (D.list D.string))
        (D.field "pool_courts" (D.list (D.map2 PoolCourt (D.field "pool" D.string) (D.field "court" D.string))))


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
    D.map6 BracketNode
        (D.field "kind" D.string)
        (D.field "round" D.int)
        (D.field "index" D.int)
        (D.field "team_a" (D.nullable D.string))
        (D.field "team_b" (D.nullable D.string))
        (D.field "winner" (D.nullable D.string))


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


matchVDec : D.Decoder MatchV
matchVDec =
    D.map6 MatchV
        (D.field "id" D.string)
        (D.field "team_a" D.string)
        (D.field "team_b" D.string)
        (D.field "status" D.string)
        (D.field "court" (D.nullable D.string))
        (D.field "done_order" (D.nullable D.int))



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
                    viewTournament s

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
    div [ class "match row" ]
        [ a [ onClick (OpenT t.id) ] [ text t.name ]
        , span [ class "pill" ] [ text t.phase ]
        ]


viewTournament : Sel -> Html Msg
viewTournament s =
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
                    viewBoard s names

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
        , item StepFinals "4 · Finales"
        , item StepRanking "5 · Classement"
        ]


viewTeams : Sel -> Html Msg
viewTeams s =
    div [ class "panel" ]
        [ h2 [] [ text "Équipes" ]
        , div [ class "row" ]
            [ input [ placeholder "Nom d'équipe", value s.newTeam, onInput SetNewTeam ] []
            , button [ onClick AddTeam ] [ text "+ Équipe" ]
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
    div [ class "match row", Html.Attributes.style "justify-content" "space-between" ]
        [ span [] [ text t.name ]
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
                (List.map (\p -> poolRow names s.view.courts (assignedOf p.id) p) s.view.pools)
        , div [ class "row", Html.Attributes.style "margin-top" "1rem" ]
            [ button
                [ onClick StartPools
                , disabled (List.isEmpty s.view.pools || List.isEmpty s.view.courts)
                ]
                [ text "Lancer les poules" ]
            , button [ class "secondary", onClick (GoStep StepBoard) ] [ text "Terrains →" ]
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
            ]
        , if List.isEmpty s.bracket then
            p [ class "muted" ] [ text "Bracket non tiré." ]

          else
            div []
                [ bracketColumn "Principal" (List.filter (\n -> n.kind == "Main") s.bracket)
                , bracketColumn "Consolante" (List.filter (\n -> n.kind == "Consolation") s.bracket)
                ]
        ]


bracketColumn : String -> List BracketNode -> Html Msg
bracketColumn title nodes =
    if List.isEmpty nodes then
        text ""

    else
        div []
            (h3 [ class "muted" ] [ text title ]
                :: List.map bracketNodeRow (List.sortBy (\n -> ( n.round, n.index )) nodes)
            )


bracketNodeRow : BracketNode -> Html Msg
bracketNodeRow n =
    let
        side m =
            Maybe.withDefault "—" m

        result =
            case n.winner of
                Just w ->
                    " → " ++ w

                Nothing ->
                    ""
    in
    let
        label =
            if n.round == 0 then
                "Prélim "

            else if n.round == 255 then
                "3e place "

            else
                "T" ++ String.fromInt n.round ++ " "
    in
    div [ class "match" ]
        [ span [ class "muted" ] [ text label ]
        , text (side n.teamA ++ " vs " ++ side n.teamB ++ result)
        ]


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


poolRow : Dict String String -> List String -> Maybe String -> PoolV -> Html Msg
poolRow names courts assigned p =
    div [ class "match" ]
        [ div [ class "row", Html.Attributes.style "justify-content" "space-between" ]
            [ span [ Html.Attributes.style "font-weight" "600" ] [ text p.name ]
            , div [ class "row" ]
                [ courtSelect courts assigned p.id
                , button [ onClick (GenPoolMatches p.id) ] [ text "Générer matchs" ]
                ]
            ]
        , div [ class "muted", Html.Attributes.style "font-size" ".85rem" ]
            [ text (String.join " · " (List.map (nameOf names) p.teams)) ]
        ]


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


viewBoard : Sel -> Dict String String -> Html Msg
viewBoard s names =
    div [ class "panel" ]
        [ div [ class "row" ]
            [ h2 [] [ text "Terrains" ]
            , button [ onClick Dispatch ] [ text "⟳ Dispatch auto" ]
            ]
        , if List.isEmpty s.view.courts then
            p [ class "muted" ] [ text "Aucun terrain configuré." ]

          else
            div [ class "lanes" ] (List.indexedMap (viewLane s names) s.board.courts)
        , viewPending s names
        ]


{-| One court as a horizontal timeline: completed (left) → current → next →
previews (right). -}
viewLane : Sel -> Dict String String -> Int -> CourtPlan -> Html Msg
viewLane s names idx cp =
    let
        completed =
            s.board.matches
                |> List.filter (\m -> m.status == "Done" && m.court == Just cp.court)
                |> List.sortBy (\m -> Maybe.withDefault 0 m.doneOrder)

        currentNode =
            cp.current
                |> Maybe.andThen (findMatch s.board.matches)
                |> Maybe.map (liveNode s names)
                |> maybeList

        nextNode =
            cp.next
                |> Maybe.andThen (\sg -> Maybe.map (\m -> ( m, sg )) (findMatch s.board.matches sg.matchId))
                |> Maybe.map (\( m, sg ) -> suggestNode names cp.court m sg)
                |> maybeList

        previewNodes =
            List.filterMap
                (\sg -> Maybe.map (previewNode names) (findMatch s.board.matches sg.matchId))
                cp.previews

        ( badgeClass, badgeText ) =
            if cp.current /= Nothing then
                ( "badge live", "En cours" )

            else if cp.next /= Nothing then
                ( "badge free", "Libre" )

            else
                ( "badge idle", "Inactif" )

        nodes =
            List.map (doneNode names) completed ++ currentNode ++ nextNode ++ previewNodes
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


doneNode : Dict String String -> MatchV -> Html Msg
doneNode names m =
    div [ class "node done" ]
        [ nodeHead "Terminé", div [ class "node-teams" ] [ text (matchLabel names m) ] ]


liveNode : Sel -> Dict String String -> MatchV -> Html Msg
liveNode s names m =
    div [ class "node live" ]
        [ nodeHead "● En cours"
        , div [ class "node-teams" ] [ text (matchLabel names m) ]
        , scoreEntry s m.id
        ]


suggestNode : Dict String String -> String -> MatchV -> Sugg -> Html Msg
suggestNode names court m sg =
    div [ class "node suggest" ]
        [ nodeHead
            (if sg.needsRest then
                "Suivant · repos"

             else
                "Suivant"
            )
        , div [ class "node-teams" ] [ text (matchLabel names m) ]
        , button [ onClick (StartMatch m.id court) ] [ text "▶ Démarrer" ]
        ]


previewNode : Dict String String -> MatchV -> Html Msg
previewNode names m =
    div [ class "node preview" ]
        [ nodeHead "À venir", div [ class "node-teams" ] [ text (matchLabel names m) ] ]


scoreEntry : Sel -> String -> Html Msg
scoreEntry s matchId =
    let
        ( a, b ) =
            Maybe.withDefault ( "", "" ) (Dict.get matchId s.scores)
    in
    div [ class "score-entry" ]
        [ scoreLine matchId 0 a
        , scoreLine matchId 1 b
        , button [ class "secondary", onClick (SubmitScore matchId) ] [ text "Valider" ]
        ]


scoreLine : String -> Int -> String -> Html Msg
scoreLine matchId which v =
    div [ class "row stepper-line" ]
        [ button [ class "step-btn", onClick (IncScore matchId which -1) ] [ text "−" ]
        , input [ class "score", type_ "number", placeholder "0", value v, onInput (SetScore matchId which) ] []
        , button [ class "step-btn", onClick (IncScore matchId which 1) ] [ text "+" ]
        ]


viewPending : Sel -> Dict String String -> Html Msg
viewPending s names =
    let
        pending =
            List.filter (\m -> m.status == "Pending") s.board.matches
    in
    if List.isEmpty pending then
        text ""

    else
        div []
            [ h3 [ class "muted" ] [ text "En attente" ]
            , table []
                (tr [] [ th [] [ text "Match" ], th [] [ text "" ] ]
                    :: List.map (pendingRow s names) pending
                )
            ]


pendingRow : Sel -> Dict String String -> MatchV -> Html Msg
pendingRow s names m =
    tr []
        [ td [] [ text (matchLabel names m) ]
        , td [] [ span [ class "pill" ] [ text "Pending" ] ]
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
