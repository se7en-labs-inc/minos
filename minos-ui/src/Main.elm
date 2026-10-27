module Main exposing (Model, Msg(..), WebData(..), assetAmountDecoder, deploymentStatus, init, instancesDecoder, main, statusDecoder, update)

import Browser
import Html exposing (..)
import Html.Attributes exposing (..)
import Html.Events exposing (onClick)
import Http
import Json.Decode as D exposing (Decoder)
import Json.Encode as E
import Task
import Time



-- MAIN


main : Program Flags Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , subscriptions = subscriptions
        , view = view
        }



-- FLAGS


type alias Flags =
    { portalUrl : String }



-- MODEL


type alias Model =
    { portalUrl : String
    , data : WebData (List Instance)
    , now : Time.Posix
    , expandedProjects : List String
    }


type WebData a
    = Loading
    | Loaded a
    | Failed String


type alias Instance =
    { identifier : String
    , label : String
    , description : String
    , expirationMs : Maybe Int
    , projects : List Project
    , confirmed : Bool
    , referencesReady : Bool
    }


type alias Project =
    { identifier : String
    , label : String
    , vendorLabel : String
    , milestones : List Milestone
    , unspentOutput : Maybe Output
    }


type alias Milestone =
    { identifier : String
    , label : String
    , description : String
    , maturationMs : Int
    , status : String
    , requiresEvidence : Bool
    , completions : List Completion
    }


type alias Completion =
    { description : String
    , evidence : List Evidence
    }


type alias Evidence =
    { anchorUrl : String
    , hash : Maybe String
    }


type alias Output =
    { transaction : String
    , index : Int
    , value : List AssetAmount
    }


type alias AssetAmount =
    { assetId : String
    , quantity : String
    }



-- INIT


init : Flags -> ( Model, Cmd Msg )
init flags =
    ( { portalUrl = flags.portalUrl
      , data = Loading
      , now = Time.millisToPosix 0
      , expandedProjects = []
      }
    , Cmd.batch
        [ fetchInstances flags.portalUrl
        , Task.perform GotTime Time.now
        ]
    )


fetchInstances : String -> Cmd Msg
fetchInstances portalUrl =
    Http.request
        { method = "POST"
        , headers = []
        , url = portalUrl
        , timeout = Just 30000
        , tracker = Nothing
        , body =
            Http.jsonBody
                (E.object
                    [ ( "query"
                      , E.string """
{ apiVersion instances {
    identifier label description confirmed referencesReady
    expiration { unixMilli }
    projects {
      identifier label
      vendor { label }
      milestones {
        identifier label description
        maturation { unixMilli }
        status
        requiresEvidence
        completions {
          description
          evidence { anchorUrl hash }
        }
      }
      unspentOutput {
        transaction index
        value { assetId quantity }
      }
    }
  }
}"""
                      )
                    ]
                )
        , expect = Http.expectJson GotInstances instancesDecoder
        }



-- MSG


type Msg
    = GotInstances (Result Http.Error (List Instance))
    | GotTime Time.Posix
    | ToggleProject String
    | Refresh



-- UPDATE


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        GotInstances (Ok instances) ->
            ( { model | data = Loaded instances }, Cmd.none )

        GotInstances (Err err) ->
            ( { model | data = Failed (httpErrorToString err) }, Cmd.none )

        GotTime posix ->
            ( { model | now = posix }, Cmd.none )

        ToggleProject id ->
            let
                expanded =
                    if List.member id model.expandedProjects then
                        List.filter (\x -> x /= id) model.expandedProjects

                    else
                        id :: model.expandedProjects
            in
            ( { model | expandedProjects = expanded }, Cmd.none )

        Refresh ->
            ( { model | data = Loading }
            , Cmd.batch
                [ fetchInstances model.portalUrl
                , Task.perform GotTime Time.now
                ]
            )


httpErrorToString : Http.Error -> String
httpErrorToString err =
    case err of
        Http.Timeout ->
            "Request timed out"

        Http.NetworkError ->
            "Network error"

        Http.BadStatus code ->
            "HTTP " ++ String.fromInt code

        Http.BadBody body ->
            "Decode error: " ++ body

        Http.BadUrl url ->
            "Bad URL: " ++ url



-- SUBSCRIPTIONS


subscriptions : Model -> Sub Msg
subscriptions _ =
    Time.every 30000 GotTime



-- DECODERS


instancesDecoder : Decoder (List Instance)
instancesDecoder =
    D.at [ "data", "apiVersion" ] D.int
        |> D.andThen
            (\version ->
                if version == 2 then
                    D.at [ "data", "instances" ] (D.list instanceDecoder)

                else
                    D.fail "Unsupported portal API version; version 2 is required"
            )


instanceDecoder : Decoder Instance
instanceDecoder =
    D.map7 Instance
        (D.field "identifier" D.string)
        (D.field "label" D.string)
        (D.field "description" D.string)
        (D.maybe
            (D.at [ "expiration", "unixMilli" ] D.string
                |> D.map (String.toInt >> Maybe.withDefault 0)
            )
        )
        (D.field "projects" (D.list projectDecoder))
        (D.field "confirmed" D.bool)
        (D.field "referencesReady" D.bool)


projectDecoder : Decoder Project
projectDecoder =
    D.map5 Project
        (D.field "identifier" D.string)
        (D.field "label" D.string)
        (D.at [ "vendor", "label" ] D.string)
        (D.field "milestones" (D.list milestoneDecoder))
        (D.maybe (D.field "unspentOutput" outputDecoder))


milestoneDecoder : Decoder Milestone
milestoneDecoder =
    D.map7 Milestone
        (D.field "identifier" D.string)
        (D.field "label" D.string)
        (D.field "description" D.string)
        (D.at [ "maturation", "unixMilli" ] D.string
            |> D.map (String.toInt >> Maybe.withDefault 0)
        )
        (D.field "status" statusDecoder)
        (D.field "requiresEvidence" D.bool)
        (D.field "completions" (D.list completionDecoder))


completionDecoder : Decoder Completion
completionDecoder =
    D.map2 Completion
        (D.field "description" D.string)
        (D.field "evidence" (D.list evidenceDecoder))


evidenceDecoder : Decoder Evidence
evidenceDecoder =
    D.map2 Evidence
        (D.field "anchorUrl" D.string)
        (D.maybe (D.field "hash" D.string))


outputDecoder : Decoder Output
outputDecoder =
    D.map3 Output
        (D.field "transaction" D.string)
        (D.field "index" D.int)
        (D.field "value" (D.list assetAmountDecoder))


assetAmountDecoder : Decoder AssetAmount
assetAmountDecoder =
    D.map2 AssetAmount
        (D.field "assetId" D.string)
        (D.field "quantity" D.string)


statusDecoder : Decoder String
statusDecoder =
    D.string
        |> D.andThen
            (\status ->
                if List.member status [ "Active", "Matured", "Paused", "Withdrawn", "Unresolved", "Unavailable" ] then
                    D.succeed status

                else
                    D.fail ("Unsupported milestone status: " ++ status)
            )



-- VIEW


view : Model -> Html Msg
view model =
    div [ class "app" ]
        [ viewStyles
        , viewHeader
        , div [ class "content" ]
            [ case model.data of
                Loading ->
                    viewLoading

                Failed err ->
                    viewError err

                Loaded instances ->
                    viewInstances model instances
            ]
        ]


viewStyles : Html msg
viewStyles =
    node "style"
        []
        [ text css ]


css : String
css =
    """
:root {
  --bg: #0d0f1a;
  --surface: #141627;
  --surface2: #1c1f35;
  --border: #2a2d4a;
  --purple: #7c3aed;
  --purple-light: #a78bfa;
  --amber: #f59e0b;
  --green: #10b981;
  --red: #ef4444;
  --text: #e2e8f0;
  --text-muted: #94a3b8;
  --text-dim: #64748b;
  --radius: 12px;
  --radius-sm: 6px;
  --font: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
}
* { box-sizing: border-box; margin: 0; padding: 0; }
body {
  background: var(--bg);
  color: var(--text);
  font-family: var(--font);
  font-size: 14px;
  line-height: 1.5;
  min-height: 100vh;
}
.app { min-height: 100vh; }
.header {
  background: var(--surface);
  border-bottom: 1px solid var(--border);
  padding: 0 24px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 64px;
  position: sticky;
  top: 0;
  z-index: 100;
  backdrop-filter: blur(8px);
}
.header-logo {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 18px;
  font-weight: 700;
  color: var(--text);
}
.logo-mark {
  width: 32px;
  height: 32px;
  background: linear-gradient(135deg, #7c3aed, #c084fc);
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 15px;
  font-weight: 900;
  color: white;
}
.header-subtitle {
  font-size: 11px;
  color: var(--text-muted);
  font-weight: 400;
  margin-top: 1px;
}
.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  border-radius: var(--radius-sm);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  border: none;
  transition: opacity 0.15s, background 0.15s;
  font-family: var(--font);
}
.btn:hover { opacity: 0.85; }
.btn-primary { background: var(--purple); color: white; }
.btn-ghost {
  background: transparent;
  color: var(--text-muted);
  border: 1px solid var(--border);
}
.btn-ghost:hover { color: var(--text); border-color: var(--text-muted); }
.badge {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 99px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.badge-active {
  background: rgba(16,185,129,0.12);
  color: #34d399;
  border: 1px solid rgba(16,185,129,0.25);
}
.badge-paused {
  background: rgba(245,158,11,0.12);
  color: #fbbf24;
  border: 1px solid rgba(245,158,11,0.25);
}
.badge-ready {
  background: rgba(124,58,237,0.18);
  color: #c084fc;
  border: 1px solid rgba(124,58,237,0.35);
}
.badge-withdrawn {
  background: rgba(100,116,139,0.12);
  color: #94a3b8;
  border: 1px solid rgba(100,116,139,0.25);
}
.content {
  max-width: 1100px;
  margin: 0 auto;
  padding: 32px 24px;
}
.instance { margin-bottom: 40px; }
.instance-heading {
  display: flex;
  align-items: baseline;
  gap: 12px;
  margin-bottom: 6px;
}
.instance-title {
  font-size: 22px;
  font-weight: 700;
  color: var(--text);
}
.instance-id {
  font-size: 11px;
  color: var(--text-dim);
  font-family: 'SFMono-Regular', Consolas, monospace;
}
.instance-meta {
  display: flex;
  align-items: center;
  gap: 20px;
  font-size: 12px;
  color: var(--text-muted);
  margin-bottom: 20px;
}
.projects-grid { display: grid; gap: 12px; }
.project-card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  transition: border-color 0.15s;
}
.project-card:hover { border-color: rgba(124,58,237,0.35); }
.project-header {
  padding: 18px 24px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  user-select: none;
}
.project-header:hover { background: rgba(255,255,255,0.02); }
.project-name {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
  margin-bottom: 2px;
}
.project-id {
  font-size: 11px;
  color: var(--text-dim);
  font-family: 'SFMono-Regular', Consolas, monospace;
}
.project-right {
  display: flex;
  align-items: center;
  gap: 24px;
  flex-shrink: 0;
}
.stat { text-align: right; }
.stat-val {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
}
.stat-val-green { color: #34d399; }
.stat-val-muted { color: var(--text-dim); }
.stat-lbl { font-size: 11px; color: var(--text-dim); }
.chevron {
  color: var(--text-dim);
  transition: transform 0.2s;
  font-size: 14px;
}
.chevron-open { transform: rotate(180deg); }
.utxo-row {
  padding: 10px 24px;
  background: rgba(28,31,53,0.6);
  border-top: 1px solid var(--border);
  font-size: 11px;
  color: var(--text-dim);
  font-family: 'SFMono-Regular', Consolas, monospace;
  display: flex;
  align-items: center;
  gap: 6px;
}
.utxo-hash { color: #a78bfa; opacity: 0.85; }
.milestones-panel { border-top: 1px solid var(--border); }
.ms-table-header {
  display: grid;
  grid-template-columns: 36px 1fr 180px 90px;
  gap: 12px;
  padding: 10px 24px;
  background: rgba(28,31,53,0.5);
  border-bottom: 1px solid var(--border);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-dim);
}
.ms-row {
  display: grid;
  grid-template-columns: 36px 1fr 180px 90px;
  gap: 12px;
  align-items: center;
  padding: 13px 24px;
  border-bottom: 1px solid rgba(42,45,74,0.4);
  transition: background 0.1s;
}
.ms-row:last-child { border-bottom: none; }
.ms-row:hover { background: rgba(255,255,255,0.02); }
.ms-row-withdrawn { opacity: 0.45; }
.ms-num {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-dim);
  font-family: 'SFMono-Regular', Consolas, monospace;
}
.ms-label {
  font-size: 13px;
  font-weight: 500;
  color: var(--text);
  margin-bottom: 2px;
}
.ms-desc {
  font-size: 11px;
  color: var(--text-dim);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ms-date { font-size: 12px; color: var(--text-muted); }
.ms-date-ready { color: #34d399; font-weight: 500; }
.ms-status { display: flex; justify-content: flex-end; }
.ms-evidence { display: block; margin-top: 3px; }
.ms-evidence-missing { font-size: 10px; color: var(--text-dim); font-style: italic; }
.ms-evidence-link {
  font-size: 10px;
  color: #818cf8;
  text-decoration: none;
  border-bottom: 1px dashed rgba(129,140,248,0.4);
  margin-right: 8px;
}
.ms-evidence-link:hover { color: #a5b4fc; border-bottom-style: solid; }
.loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  padding: 80px 0;
  color: var(--text-muted);
  font-size: 13px;
}
.spinner {
  width: 28px;
  height: 28px;
  border: 3px solid var(--border);
  border-top-color: var(--purple);
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
.error-box {
  background: rgba(239,68,68,0.08);
  border: 1px solid rgba(239,68,68,0.25);
  border-radius: var(--radius);
  padding: 18px 24px;
  color: #fca5a5;
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 13px;
}
.error-detail {
  font-size: 11px;
  margin-top: 3px;
  color: rgba(252,165,165,0.7);
}
.empty {
  padding: 32px 24px;
  text-align: center;
  color: var(--text-dim);
  font-size: 13px;
}
"""


viewHeader : Html Msg
viewHeader =
    header [ class "header" ]
        [ div [ class "header-logo" ]
            [ div [ class "logo-mark" ] [ text "M" ]
            , div []
                [ div [] [ text "Minos Treasury" ]
                , div [ class "header-subtitle" ] [ text "Cardano milestone management" ]
                ]
            ]
        , div []
            [ button [ class "btn btn-ghost", onClick Refresh ] [ text "↻  Refresh" ]
            ]
        ]


viewLoading : Html msg
viewLoading =
    div [ class "loading" ]
        [ div [ class "spinner" ] []
        , text "Loading treasury data…"
        ]


viewError : String -> Html Msg
viewError err =
    div [ class "error-box" ]
        [ text "⚠"
        , div []
            [ div [ style "font-weight" "600" ] [ text "Failed to load data" ]
            , div [ class "error-detail" ] [ text err ]
            ]
        , button
            [ class "btn btn-ghost"
            , onClick Refresh
            , style "margin-left" "auto"
            , style "flex-shrink" "0"
            ]
            [ text "Retry" ]
        ]


viewInstances : Model -> List Instance -> Html Msg
viewInstances model instances =
    if List.isEmpty instances then
        div [ class "empty" ] [ text "No instances found." ]

    else
        div [] (List.map (viewInstance model) instances)


deploymentStatus : Instance -> String
deploymentStatus inst =
    if inst.confirmed && inst.referencesReady then
        "confirmed"

    else if inst.confirmed then
        "references pending"

    else
        "pending confirmation"


viewInstance : Model -> Instance -> Html Msg
viewInstance model inst =
    div [ class "instance" ]
        [ div [ class "instance-heading" ]
            [ span [ class "instance-title" ] [ text inst.label ]
            , span [ class "instance-id" ] [ text (shortHash inst.identifier) ]
            , span [ class "badge" ]
                [ text
                    (deploymentStatus inst)
                ]
            ]
        , div [ class "instance-meta" ]
            (case inst.expirationMs of
                Nothing ->
                    []

                Just ms ->
                    [ span [] [ text ("Expires " ++ formatDate ms) ] ]
            )
        , div [ class "projects-grid" ]
            (List.map (viewProject model) inst.projects)
        ]


viewProject : Model -> Project -> Html Msg
viewProject model proj =
    let
        isExpanded =
            List.member proj.identifier model.expandedProjects

        nowMs =
            Time.posixToMillis model.now

        totalMs =
            List.length proj.milestones

        withdrawnMs =
            proj.milestones
                |> List.filter (\m -> m.status == "Withdrawn")
                |> List.length

        readyMs =
            proj.milestones
                |> List.filter (\m -> m.status == "Active" && m.maturationMs > 0 && m.maturationMs <= nowMs)
                |> List.length
    in
    div [ class "project-card" ]
        [ div [ class "project-header", onClick (ToggleProject proj.identifier) ]
            [ div []
                [ div [ class "project-name" ] [ text proj.label ]
                , div [ class "project-id" ] [ text proj.identifier ]
                ]
            , div [ class "project-right" ]
                [ div [ class "stat" ]
                    [ div [ class "stat-val" ] [ text (String.fromInt totalMs) ]
                    , div [ class "stat-lbl" ] [ text "milestones" ]
                    ]
                , if withdrawnMs > 0 then
                    div [ class "stat" ]
                        [ div [ class "stat-val stat-val-muted" ] [ text (String.fromInt withdrawnMs) ]
                        , div [ class "stat-lbl" ] [ text "withdrawn" ]
                        ]

                  else
                    text ""
                , div [ class "stat" ]
                    [ div
                        [ class
                            (if readyMs > 0 then
                                "stat-val stat-val-green"

                             else
                                "stat-val"
                            )
                        ]
                        [ text (String.fromInt readyMs) ]
                    , div [ class "stat-lbl" ] [ text "ready" ]
                    ]
                , span
                    [ class
                        (if isExpanded then
                            "chevron chevron-open"

                         else
                            "chevron"
                        )
                    ]
                    [ text "▾" ]
                ]
            ]
        , case proj.unspentOutput of
            Nothing ->
                text ""

            Just out ->
                div [ class "utxo-row" ]
                    [ text "UTxO "
                    , span [ class "utxo-hash" ]
                        [ text (out.transaction ++ "#" ++ String.fromInt out.index) ]
                    ]
        , if isExpanded then
            viewMilestones model.now proj.milestones

          else
            text ""
        ]


msIndex : String -> Int
msIndex id =
    id
        |> String.split "-"
        |> List.reverse
        |> List.head
        |> Maybe.andThen String.toInt
        |> Maybe.withDefault 0


viewMilestones now milestones =
    let
        sorted =
            List.sortBy (\m -> msIndex m.identifier) milestones
    in
    if List.isEmpty sorted then
        div [ class "empty" ] [ text "No milestones." ]

    else
        div [ class "milestones-panel" ]
            [ div [ class "ms-table-header" ]
                [ text "#"
                , text "Milestone"
                , text "Unlocks"
                , text "Status"
                ]
            , div [] (List.indexedMap (viewMilestone now) sorted)
            ]


viewEvidence : String -> Bool -> List Completion -> Html msg
viewEvidence status requiresEvidence completions =
    let
        allEvidence =
            List.concatMap .evidence completions

        showEvidence =
            status == "Matured" || status == "Withdrawn" || not (List.isEmpty allEvidence)
    in
    if not showEvidence then
        text ""

    else if not requiresEvidence then
        span [ class "ms-evidence ms-evidence-missing" ] [ text "no evidence required" ]

    else if List.isEmpty allEvidence then
        span [ class "ms-evidence ms-evidence-missing" ] [ text "evidence not submitted" ]

    else
        span [ class "ms-evidence" ]
            (List.indexedMap
                (\i ev ->
                    a
                        [ href ev.anchorUrl
                        , Html.Attributes.target "_blank"
                        , Html.Attributes.rel "noopener noreferrer"
                        , class "ms-evidence-link"
                        ]
                        [ text
                            (if List.length allEvidence == 1 then
                                "evidence"

                             else
                                "evidence " ++ String.fromInt (i + 1)
                            )
                        ]
                )
                allEvidence
            )


viewMilestone : Time.Posix -> Int -> Milestone -> Html msg
viewMilestone now idx ms =
    let
        nowMs =
            Time.posixToMillis now

        isReady =
            ms.status == "Active" && ms.maturationMs > 0 && ms.maturationMs <= nowMs

        badge =
            case ms.status of
                "Active" ->
                    if isReady then
                        span [ class "badge badge-ready" ] [ text "ready" ]

                    else
                        span [ class "badge badge-active" ] [ text "active" ]

                "Paused" ->
                    span [ class "badge badge-paused" ] [ text "paused" ]

                "Matured" ->
                    span [ class "badge badge-ready" ] [ text "ready" ]

                "Withdrawn" ->
                    span [ class "badge badge-withdrawn" ] [ text "withdrawn" ]

                other ->
                    span [ class "badge" ] [ text (String.toLower other) ]

        rowClass =
            if ms.status == "Withdrawn" then
                "ms-row ms-row-withdrawn"

            else
                "ms-row"
    in
    div [ class rowClass ]
        [ span [ class "ms-num" ] [ text (String.fromInt (idx + 1)) ]
        , div []
            [ div [ class "ms-label" ] [ text ms.label ]
            , div [ class "ms-desc" ] [ text ms.description ]
            , viewEvidence ms.status ms.requiresEvidence ms.completions
            ]
        , div
            [ class
                (if isReady then
                    "ms-date ms-date-ready"

                 else
                    "ms-date"
                )
            ]
            [ text (formatDate ms.maturationMs) ]
        , div [ class "ms-status" ] [ badge ]
        ]



-- HELPERS


formatDate : Int -> String
formatDate ms =
    if ms == 0 then
        "—"

    else
        let
            posix =
                Time.millisToPosix ms

            year =
                String.fromInt (Time.toYear Time.utc posix)

            month =
                monthStr (Time.toMonth Time.utc posix)

            day =
                String.padLeft 2 '0' (String.fromInt (Time.toDay Time.utc posix))

            hour =
                String.padLeft 2 '0' (String.fromInt (Time.toHour Time.utc posix))

            minute =
                String.padLeft 2 '0' (String.fromInt (Time.toMinute Time.utc posix))
        in
        month ++ " " ++ day ++ " " ++ year ++ "  " ++ hour ++ ":" ++ minute ++ " UTC"


monthStr : Time.Month -> String
monthStr m =
    case m of
        Time.Jan ->
            "Jan"

        Time.Feb ->
            "Feb"

        Time.Mar ->
            "Mar"

        Time.Apr ->
            "Apr"

        Time.May ->
            "May"

        Time.Jun ->
            "Jun"

        Time.Jul ->
            "Jul"

        Time.Aug ->
            "Aug"

        Time.Sep ->
            "Sep"

        Time.Oct ->
            "Oct"

        Time.Nov ->
            "Nov"

        Time.Dec ->
            "Dec"


shortHash : String -> String
shortHash s =
    if String.length s <= 16 then
        s

    else
        String.left 8 s ++ "…" ++ String.right 6 s
