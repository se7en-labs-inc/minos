port module Tests exposing (main)

import Http
import Json.Decode as D
import Main exposing (Msg(..), WebData(..))
import Platform
import Time


port results : List { name : String, passed : Bool } -> Cmd msg


check : String -> Bool -> { name : String, passed : Bool }
check name passed =
    { name = name, passed = passed }


main : Program () () Never
main =
    Platform.worker
        { init = \_ -> ( (), results tests )
        , update = \never _ -> never |> neverIdentity
        , subscriptions = \_ -> Sub.none
        }


neverIdentity : Never -> a
neverIdentity impossible =
    never impossible


tests : List { name : String, passed : Bool }
tests =
    let
        ( initial, _ ) =
            Main.init { portalUrl = "https://portal.example/graphql" }

        ( failed, _ ) =
            Main.update (GotInstances (Err Http.Timeout)) initial

        ( empty, _ ) =
            Main.update (GotInstances (Ok [])) failed

        ( refresh, _ ) =
            Main.update Refresh empty

        pending =
            "{\"data\":{\"apiVersion\":2,\"instances\":[{\"identifier\":\"one\",\"label\":\"test\",\"description\":\"\",\"expiration\":{\"unixMilli\":\"1000\"},\"projects\":[],\"confirmed\":false,\"referencesReady\":false}]}}"
    in
    [ check "deployment endpoint retained" (initial.portalUrl == "https://portal.example/graphql")
    , check "initial loading"
        (case initial.data of
            Loading ->
                True

            _ ->
                False
        )
    , check "timeout visible"
        (case failed.data of
            Failed message ->
                String.contains "timed out" message

            _ ->
                False
        )
    , check "empty response recovers error"
        (case empty.data of
            Loaded [] ->
                True

            _ ->
                False
        )
    , check "refresh loads"
        (case refresh.data of
            Loading ->
                True

            _ ->
                False
        )
    , check "empty decoder" (D.decodeString Main.instancesDecoder "{\"data\":{\"apiVersion\":2,\"instances\":[]}}" == Ok [])
    , check "pending is explicit"
        (case D.decodeString Main.instancesDecoder pending of
            Ok [ instance ] ->
                not instance.confirmed

            _ ->
                False
        )
    , check "GraphQL errors reject" (Result.toMaybe (D.decodeString Main.instancesDecoder "{\"errors\":[{\"message\":\"failed\"}],\"data\":null}") == Nothing)
    , check "confirmed registry keeps missing references visible"
        (case D.decodeString Main.instancesDecoder (String.replace "\"confirmed\":false" "\"confirmed\":true" pending) of
            Ok [ instance ] ->
                Main.deploymentStatus instance == "references pending"

            _ ->
                False
        )
    , check "complete deployment shows confirmed"
        (case D.decodeString Main.instancesDecoder (pending |> String.replace "\"confirmed\":false" "\"confirmed\":true" |> String.replace "\"referencesReady\":false" "\"referencesReady\":true") of
            Ok [ instance ] ->
                Main.deploymentStatus instance == "confirmed"

            _ ->
                False
        )
    , check "old API rejects" (Result.toMaybe (D.decodeString Main.instancesDecoder "{\"data\":{\"apiVersion\":1,\"instances\":[]}}") == Nothing)
    , check "unsafe status rejects" (Result.toMaybe (D.decodeString Main.statusDecoder "\"FutureStatus\"") == Nothing)
    , check "all supported statuses" (List.all (\status -> D.decodeString Main.statusDecoder ("\"" ++ status ++ "\"") == Ok status) [ "Active", "Matured", "Paused", "Withdrawn", "Unresolved", "Unavailable" ])
    , check "native asset large integer remains exact"
        (case D.decodeString Main.assetAmountDecoder "{\"assetId\":\"abcd\",\"quantity\":\"18446744073709551615\"}" of
            Ok asset ->
                asset.assetId == "abcd" && asset.quantity == "18446744073709551615"

            _ ->
                False
        )
    ]
