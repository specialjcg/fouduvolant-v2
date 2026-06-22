module Types exposing (..)

import Dict exposing (Dict)
import Http
import Time


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
    , confirmForfeit : Maybe String
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
    { id : String, name : String, player1 : String, player2 : String, forfeited : Bool }


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
    , conceded : Bool
    }


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
    | AskForfeit String
    | CancelForfeit
    | ConfirmForfeit String
    | GoStep Step
    | SetNumPools String
    | AutoPools
    | ProposeIdealPools
    | StartPools
    | StartFinals
    | ResetTournament
    | RedoPools
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
    | ConcedeMatch String String
    | Mutated (Result Http.Error ())
    | Tick Time.Posix
    | GotZone Time.Zone
    | ToggleShowPast
