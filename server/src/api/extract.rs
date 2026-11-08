//! Request extractors whose rejections render as the API's JSON error shape
//! (`{"error":"bad_request","message":…}`) instead of axum's plain-text defaults.

use axum::{
    body::Bytes,
    extract::{FromRequest, FromRequestParts, Path, Query, Request},
    http::request::Parts,
};
use serde::de::DeserializeOwned;

use crate::error::AppError;

/// JSON request body (requires `Content-Type: application/json`).
pub struct JsonBody<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for JsonBody<T> {
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(v)) => Ok(JsonBody(v)),
            Err(e) => Err(AppError::BadRequest(e.body_text())),
        }
    }
}

/// Optional JSON body: an empty body yields `T::default()`. Content-Type is not checked,
/// so `fetch(url, {method: "POST"})` with no body works for endpoints with only optional fields.
pub struct JsonOrEmpty<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Default> FromRequest<S> for JsonOrEmpty<T> {
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req, state).await.map_err(|e| AppError::BadRequest(e.body_text()))?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(JsonOrEmpty(T::default()));
        }
        serde_json::from_slice(&bytes)
            .map(JsonOrEmpty)
            .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))
    }
}

/// Path parameters.
pub struct PathParam<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for PathParam<T> {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(v)) => Ok(PathParam(v)),
            Err(e) => Err(AppError::BadRequest(e.body_text())),
        }
    }
}

/// Query string.
pub struct QueryParams<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for QueryParams<T> {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(v)) => Ok(QueryParams(v)),
            Err(e) => Err(AppError::BadRequest(e.body_text())),
        }
    }
}

/// `?flag=1` / `true` / `yes` / `on`.
pub fn flag(v: &Option<String>) -> bool {
    matches!(v.as_deref().map(str::trim), Some("1" | "true" | "yes" | "on"))
}

/// Treats `?x=` (empty) like an absent parameter.
pub fn non_empty(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}
