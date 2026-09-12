//! What a failure tells the caller.

mod common;

use axum::http::StatusCode;

use common::{Caller, harness};

#[tokio::test]
async fn a_server_fault_names_its_code_and_nothing_about_the_schema() {
    let app = harness().await;
    sqlx::query("drop table bar_hours cascade")
        .execute(app.store.pool())
        .await
        .expect("the fixture database can be broken");

    let answer = app.get("/api/session", &Caller::new("Вера")).await;

    assert_eq!(answer.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(answer.error_code(), Some("internal"));
    let body = answer.body.to_string();
    assert!(!body.contains("bar_hours"), "the body leaks the schema: {body}");
}
