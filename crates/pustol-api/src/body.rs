use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::error::ApiError;

/// JSON body extractor; refuses U+0000 in any string or key, which `PostgreSQL` text cannot hold.
///
/// Other `axum::Json` refusals keep axum status as JSON `body_invalid`: app reads code from JSON.
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
        // Parse raw text, not `value`: map keeps only last repeated key.
        let axum::Json(body) = axum::Json::<T>::from_bytes(raw.get().as_bytes())?;
        Ok(Self(body))
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self::new(rejection.status(), "body_invalid", rejection.body_text())
    }
}

pub(crate) fn refuse_nul(value: &Value, what: &str) -> Result<(), ApiError> {
    if holds_nul(value) {
        return Err(nul_refused(what));
    }
    Ok(())
}

pub(crate) fn nul_refused(what: &str) -> ApiError {
    ApiError::bad_request(
        "text_invalid",
        format!("{what} holds a NUL character, which no text can be stored with"),
    )
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
