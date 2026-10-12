use async_graphql_axum::GraphQL;
use axum::{routing::get, Router};
use tower_http::cors::CorsLayer;

use crate::schema::MinosSchema;

pub fn router(schema: MinosSchema) -> Router {
    Router::new()
        .route(
            "/graphql",
            get(graphql_playground).post_service(GraphQL::new(schema)),
        )
        .layer(CorsLayer::permissive())
}

async fn graphql_playground() -> impl axum::response::IntoResponse {
    axum::response::Html(async_graphql::http::playground_source(
        async_graphql::http::GraphQLPlaygroundConfig::new("/graphql"),
    ))
}
