//! Request bodies.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
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
/// A body not said to be JSON, one that is not JSON, and one of the wrong shape are refused with the
/// status `axum::Json` gives each, as `body_invalid` in the shape of every other refusal: the app reads
/// a code out of a JSON body, and a line of plain text reached it as a failure to parse.
#[derive(Debug)]
pub struct JsonBody<T>(pub T);

impl<T, S> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Json(raw) = axum::Json::<Box<RawValue>>::from_request(request, state).await?;
        let axum::Json(value) = axum::Json::<Value>::from_bytes(raw.get().as_bytes())?;
        refuse_nul(&value, "a string in the body")?;
        let axum::Json(body) = axum::Json::<T>::from_bytes(raw.get().as_bytes())?;
        Ok(Self(body))
    }
}

/// axum's refusal of a body, under axum's own status, as `body_invalid`.
impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self::new(rejection.status(), "body_invalid", rejection.body_text())
    }
}

/// Refuses `value` as `text_invalid` when any string or key anywhere in it holds U+0000, saying
/// `what` held it.
pub(crate) fn refuse_nul(value: &Value, what: &str) -> Result<(), ApiError> {
    if holds_nul(value) {
        return Err(ApiError::bad_request(
            "text_invalid",
            format!("{what} holds a NUL character, which no text can be stored with"),
        ));
    }
    Ok(())
}

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
