//! Extractors whose rejections are v2 problems, and the id type of v2 paths.

use super::problem::ApiProblem;
use aide::{OperationInput, generate::GenContext, openapi::Operation};
use axum::{extract::FromRequestParts, http::request::Parts};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use std::{borrow::Cow, fmt};

/// Query parameters; a parameter may repeat to give a list.
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiProblem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum_extra::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|query| Self(query.0))
            .map_err(|rejection| ApiProblem::invalid(rejection.body_text()))
    }
}

impl<T: JsonSchema> OperationInput for Query<T> {
    fn operation_input(ctx: &mut GenContext, operation: &mut Operation) {
        axum::extract::Query::<T>::operation_input(ctx, operation);
    }
}

pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiProblem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|path| Self(path.0))
            .map_err(|rejection| ApiProblem::invalid(rejection.body_text()))
    }
}

impl<T: JsonSchema> OperationInput for Path<T> {
    fn operation_input(ctx: &mut GenContext, operation: &mut Operation) {
        axum::extract::Path::<T>::operation_input(ctx, operation);
    }
}

/// A numeric Twitch user id, which channels have too.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct TwitchId(String);

impl TwitchId {
    /// For ids from Twitch or the database, which need no checking.
    pub fn new_unchecked(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TwitchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TwitchId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(serde::de::Error::custom(format!(
                "`{id}` is not a numeric Twitch id"
            )));
        }
        Ok(Self(id))
    }
}

impl JsonSchema for TwitchId {
    fn schema_name() -> Cow<'static, str> {
        "TwitchId".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::TwitchId").into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^[0-9]+$",
            "description": "A numeric Twitch user id, which channels have too.",
            "examples": ["71092938"],
        })
    }

    fn inline_schema() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::TwitchId;

    #[test]
    fn twitch_ids_are_numeric() {
        let id: TwitchId = serde_json::from_str(r#""71092938""#).unwrap();
        assert_eq!(id.as_str(), "71092938");

        for invalid in [r#""""#, r#""xqc""#, r#""12a""#, r#""-1""#] {
            assert!(
                serde_json::from_str::<TwitchId>(invalid).is_err(),
                "{invalid}"
            );
        }
    }
}
