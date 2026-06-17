module View.Common exposing (shortName)


shortName : String -> String
shortName n =
    if String.length n <= 10 then
        n

    else
        String.left 9 n ++ "…"
