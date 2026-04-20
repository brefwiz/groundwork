//! ETag / conditional-request helpers.

pub use api_bones::etag::{ETag, IfMatch, IfNoneMatch, ParseETagError};

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};

use crate::ApiError;

/// Derive a weak [`ETag`] from an `updated_at` timestamp.
pub fn etag_from_updated_at(updated_at: DateTime<Utc>) -> ETag {
    let millis = updated_at.timestamp_millis();
    ETag::weak(format!("{millis:x}"))
}

/// Validate the `If-Match` request header against a current [`ETag`].
pub fn check_if_match(headers: &HeaderMap, current_etag: &ETag) -> Result<(), ApiError> {
    let raw = match headers.get(axum::http::header::IF_MATCH) {
        None => {
            let mut err = ApiError::new(
                api_bones::error::ErrorCode::BadRequest,
                "If-Match header is required",
            );
            err.status = 428;
            err.title = "Precondition Required".to_owned();
            return Err(err);
        }
        Some(v) => v
            .to_str()
            .map_err(|_| ApiError::bad_request("If-Match header is not valid ASCII"))?,
    };

    let trimmed = raw.trim();
    let matched = if trimmed == "*" {
        true
    } else {
        let tags = ETag::parse_list(trimmed)
            .map_err(|e| ApiError::bad_request(format!("If-Match header is malformed: {e}")))?;
        tags.iter().any(|t| t.matches_weak(current_etag))
    };

    if matched {
        Ok(())
    } else {
        let mut err = ApiError::new(
            api_bones::error::ErrorCode::BadRequest,
            "ETag does not match; the resource has been modified",
        );
        err.status = 412;
        err.title = "Precondition Failed".to_owned();
        Err(err)
    }
}
