module Api exposing (..)

import Http
import Json.Decode as D
import Json.Encode as E
import Decoders exposing (..)
import Types exposing (..)


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


forfeitTeam : String -> String -> String -> Cmd Msg
forfeitTeam api tid teamId =
    postEmpty api ("/tournaments/" ++ tid ++ "/teams/" ++ teamId ++ "/forfeit") (E.object [])


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


concedeMatch : String -> String -> String -> Cmd Msg
concedeMatch api matchId winnerId =
    postEmpty api ("/matches/" ++ matchId ++ "/concede") (E.object [ ( "winner", E.string winnerId ) ])


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
