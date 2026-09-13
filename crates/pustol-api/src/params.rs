//! Only place `axum::extract::Path` and `axum::extract::Query` allowed; `clippy.toml` bans them
//! elsewhere.
#![allow(clippy::disallowed_types)]

use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{FromRequestParts, Path, Query};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::error::ApiError;

/// `Path` extractor refusing as JSON `request_invalid`, axum status; app cannot read plain text.
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

/// `Query` extractor refusing as JSON `request_invalid`, like [`RequestPath`].
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

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::new(rejection.status(), "request_invalid", rejection.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::new(rejection.status(), "request_invalid", rejection.body_text())
    }
}
