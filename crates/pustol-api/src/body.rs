//! Request bodies.

use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::error::ApiError;

/// A JSON request body, taken by every handler that reads one instead of `axum::Json`.
///
/// `PostgreSQL` text cannot hold U+0000. A string carrying one reached the database from whichever
/// handler took it and came back as a server fault. Refused here, for every string and every key of
/// every body, no handler has a text field to remember it for.
///
/// Everything else is `axum::Json`'s own answer, unchanged: a body not said to be JSON, a body that is
/// not JSON, and one of the wrong shape are refused exactly as they were.
#[derive(Debug)]
pub struct JsonBody<T>(pub T);

impl<T, S> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Json(raw) = axum::Json::<Box<RawValue>>::from_request(request, state)
            .await
            .map_err(IntoResponse::into_response)?;
        let axum::Json(value) = axum::Json::<Value>::from_bytes(raw.get().as_bytes())
            .map_err(IntoResponse::into_response)?;
        if holds_nul(&value) {
            return Err(ApiError::bad_request(
                "text_invalid",
                "a string in the body holds a NUL character, which no text can be stored with",
            )
            .into_response());
        }
        axum::Json::<T>::from_bytes(raw.get().as_bytes())
            .map(|axum::Json(body)| Self(body))
            .map_err(IntoResponse::into_response)
    }
}

/// Whether any string or key anywhere in `value` holds U+0000.
fn holds_nul(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(holds_nul),
        Value::Object(fields) => fields
            .iter()
            .any(|(key, field)| key.contains('\0') || holds_nul(field)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}
