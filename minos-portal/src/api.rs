use async_graphql_axum::GraphQL;
use axum::{routing::get, Router};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer};

use crate::schema::MinosSchema;

pub fn router(schema: MinosSchema) -> Router {
    router_with_permits(schema, Arc::new(Semaphore::new(32)))
}

fn router_with_permits(schema: MinosSchema, permits: Arc<Semaphore>) -> Router {
    Router::new()
        .route(
            "/graphql",
            get(graphql_playground).post_service(GraphQL::new(schema)),
        )
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            permits,
            concurrency_limit,
        ))
        .layer(TimeoutLayer::new(Duration::from_secs(30)))
        .layer(CorsLayer::permissive())
}

async fn concurrency_limit(
    axum::extract::State(semaphore): axum::extract::State<Arc<Semaphore>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let _permit = semaphore
        .try_acquire_owned()
        .map_err(|_| axum::http::StatusCode::TOO_MANY_REQUESTS)?;
    Ok(next.run(request).await)
}

async fn graphql_playground() -> impl axum::response::IntoResponse {
    axum::response::Html(async_graphql::http::playground_source(
        async_graphql::http::GraphQLPlaygroundConfig::new("/graphql"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn live_graphql_rejects_oversized_and_expensive_requests() {
        let schema = crate::schema::build(
            Arc::new(crate::db::Store::temporary()),
            None,
            None,
            toml::from_str("development_auth_bypass = true").unwrap(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/graphql", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router(schema)).await.unwrap();
        });
        let client = minos::http::client_builder().build().unwrap();
        let response = client
            .post(&url)
            .json(&serde_json::json!({"query": "{ instances { identifier } }"}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let value: serde_json::Value = response.json().await.unwrap();
        assert_eq!(value["data"]["instances"], serde_json::json!([]));

        let expensive = format!(
            "{{ {} }}",
            (0..1001)
                .map(|i| format!("a{i}: instances {{ identifier }}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let value: serde_json::Value = client
            .post(&url)
            .json(&serde_json::json!({"query": expensive}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(value["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()));
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(vec![b' '; 1024 * 1024 + 1])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
        server.abort();
    }

    #[tokio::test]
    async fn saturated_api_rejects_work_without_an_unbounded_wait_queue() {
        let schema = crate::schema::build(
            Arc::new(crate::db::Store::temporary()),
            None,
            None,
            toml::from_str("development_auth_bypass = true").unwrap(),
        );
        let permits = Arc::new(Semaphore::new(32));
        let held = permits.clone().acquire_many_owned(32).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/graphql", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router_with_permits(schema, permits))
                .await
                .unwrap();
        });
        let client = minos::http::client_builder().build().unwrap();
        let query = serde_json::json!({"query": "{ instances { identifier } }"});
        assert_eq!(
            client
                .post(&url)
                .json(&query)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::TOO_MANY_REQUESTS
        );
        drop(held);
        assert!(client
            .post(&url)
            .json(&query)
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        server.abort();
    }
}
