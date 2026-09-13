//! Path and query parameters.
//!
//! The one place `axum::extract::Path` and `axum::extract::Query` are read: `clippy.toml` refuses them
//! everywhere else, so no handler can answer a request it cannot read with a line of plain text.
#![allow(clippy::disallowed_types)]

use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{FromRequestParts, Path, Query};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::error::ApiError;

/// The parameters of a request's path, taken by every handler that reads one instead of `Path`.
///
/// A path that does not hold them in the shape asked for, such as an identifier that is not one, is
/// refused with the status `Path` gives it, as `request_invalid` in the shape of every other refusal:
/// the app reads a code out of a JSON body, and a line of plain text reached it as a failure to parse.
#[derive(Debug)]
pub struct RequestPath<T>(pub T);

impl<T, S> FromRequestParts<S> for RequestPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(params) = Path::<T>::from_request_parts(parts, state).await?;
        Ok(Self(params))
    }
}

/// The query string of a request, taken by every handler that reads one instead of `Query`.
///
/// A query missing a field, or holding a value of the wrong shape, is refused with the status `Query`
/// gives it, as `request_invalid`, for the reason [`RequestPath`] is.
#[derive(Debug)]
pub struct RequestQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for RequestQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(params) = Query::<T>::from_request_parts(parts, state).await?;
        Ok(Self(params))
    }
}

/// axum's refusal of a path, under axum's own status, as `request_invalid`.
impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::new(rejection.status(), "request_invalid", rejection.body_text())
    }
}

/// axum's refusal of a query, under axum's own status, as `request_invalid`.
impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::new(rejection.status(), "request_invalid", rejection.body_text())
    }
}
