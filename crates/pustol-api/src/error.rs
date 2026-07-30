//! Turning a failure into something the app can act on.
//!
//! Every response carries a stable `code` as well as a message. The app switches on the code to
//! choose what to say, because the words a guest reads belong to the screen they read them on — the
//! backend would otherwise be shipping Russian copy, and changing a comma would mean a deploy.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pustol_db::Error as DbError;
use pustol_telegram::VerifyError;
use serde::Serialize;

/// A failure on its way out.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    detail: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct Body<'a> {
    error: Payload<'a>,
}

#[derive(Serialize)]
struct Payload<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: &'a Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            detail: None,
        }
    }

    #[must_use]
    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn unauthorised(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, code, message)
    }

    pub fn not_found(entity: &'static str) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", format!("no such {entity}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Server faults are logged in full and reported as a bare code: a stack of database detail
        // in a response body tells an attacker about the schema and tells the guest nothing.
        if self.status.is_server_error() {
            tracing::error!(code = self.code, message = %self.message, "request failed");
        }
        let body = Body {
            error: Payload {
                code: self.code,
                message: &self.message,
                detail: &self.detail,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

impl From<VerifyError> for ApiError {
    fn from(error: VerifyError) -> Self {
        // The distinction the app needs is "sign in again" versus "something is wrong with you":
        // a stale payload is routine and self-healing, a bad signature never is.
        let code = match error {
            VerifyError::Stale { .. } | VerifyError::SignedInTheFuture => "session_expired",
            _ => "not_telegram",
        };
        Self::unauthorised(code, error.to_string())
    }
}

impl From<DbError> for ApiError {
    fn from(error: DbError) -> Self {
        use StatusCode as Code;
        match &error {
            DbError::NotFound { entity } => Self::not_found(entity),
            DbError::NotAnArrivalTime { minutes } => Self::new(
                Code::CONFLICT,
                "not_an_arrival_time",
                error.to_string(),
            )
            .with_detail(serde_json::json!({ "start_minutes": minutes })),
            DbError::InThePast => Self::new(Code::CONFLICT, "in_the_past", error.to_string()),
            DbError::NoTableFree { party_size } => {
                Self::new(Code::CONFLICT, "no_table_free", error.to_string())
                    .with_detail(serde_json::json!({ "party_size": party_size }))
            }
            // Losing the race is the same thing to a guest as the slot having been taken: somebody
            // else got there first, look at another time.
            DbError::TableTakenConcurrently => {
                Self::new(Code::CONFLICT, "no_table_free", error.to_string())
            }
            DbError::PartyTooLarge {
                party_size,
                max_party,
            } => Self::new(Code::CONFLICT, "party_too_large", error.to_string()).with_detail(
                serde_json::json!({ "party_size": party_size, "max_party": max_party }),
            ),
            DbError::ShiftNotBookable { .. } => {
                Self::new(Code::CONFLICT, "shift_not_bookable", error.to_string())
            }
            DbError::MissingBlockReason => {
                Self::bad_request("missing_block_reason", error.to_string())
            }
            DbError::UnknownCancelReason => {
                Self::bad_request("unknown_cancel_reason", error.to_string())
            }
            DbError::UnknownMessage => Self::bad_request("unknown_message", error.to_string()),
            DbError::ProposedConfigInvalid(errors) => Self::new(
                Code::UNPROCESSABLE_ENTITY,
                "settings_invalid",
                error.to_string(),
            )
            .with_detail(serde_json::json!({
                "reasons": errors.iter().map(ToString::to_string).collect::<Vec<_>>(),
            })),
            DbError::WouldStrandBookings(conflicts) => Self::new(
                Code::CONFLICT,
                "would_strand_bookings",
                error.to_string(),
            )
            .with_detail(serde_json::json!({
                "conflicts": conflicts
                    .iter()
                    .map(|stranded| serde_json::json!({
                        "booking_id": stranded.conflict.booking().0,
                        "guest_name": stranded.guest_name,
                        "service_date": stranded.conflict.service_day().date(),
                        "start_minutes": stranded.conflict.start_minutes(),
                    }))
                    .collect::<Vec<_>>(),
            })),
            DbError::UnusableProposal(detail) => {
                Self::bad_request("settings_unreadable", detail.clone())
            }
            DbError::Time(_) => Self::bad_request("impossible_time", error.to_string()),
            DbError::UnknownTimezone(_) => Self::bad_request("unknown_timezone", error.to_string()),
            DbError::StoredConfigInvalid(_)
            | DbError::CorruptRow { .. }
            | DbError::Database(_)
            | DbError::Migration(_) => Self::new(
                Code::INTERNAL_SERVER_ERROR,
                "internal",
                error.to_string(),
            ),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
