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
        let message = if self.status.is_server_error() {
            tracing::error!(code = self.code, message = %self.message, "request failed");
            "internal error"
        } else {
            &self.message
        };
        let body = Body {
            error: Payload {
                code: self.code,
                message,
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
            VerifyError::Stale { .. }
            | VerifyError::SignedInTheFuture
            | VerifyError::SessionEnded => "session_expired",
            _ => "not_telegram",
        };
        Self::unauthorised(code, error.to_string())
    }
}

impl From<DbError> for ApiError {
    fn from(error: DbError) -> Self {
        // A request that clashes with the room or the rules as they now stand, named by its code.
        let conflict = |code| Self::new(StatusCode::CONFLICT, code, error.to_string());
        let refused = |code| Self::bad_request(code, error.to_string());
        match &error {
            DbError::NotFound { entity } => Self::not_found(entity),
            DbError::NotAnArrivalTime { minutes } => conflict("not_an_arrival_time")
                .with_detail(serde_json::json!({ "start_minutes": minutes })),
            DbError::InThePast => conflict("in_the_past"),
            DbError::NoTableFree { party_size } => conflict("no_table_free")
                .with_detail(serde_json::json!({ "party_size": party_size })),
            // Losing the race is the same thing to a guest as the slot having been taken: somebody
            // else got there first, look at another time.
            DbError::TableTakenConcurrently => conflict("no_table_free"),
            // Its own code, because it asks for its own thing: the room may have plenty of tables,
            // it is *this* one that has gone, and the answer is to pick another.
            DbError::ChosenTableNotFree => conflict("chosen_table_not_free"),
            DbError::AlreadyBookedThisShift => conflict("already_booked_tonight"),
            DbError::GuestHasAnotherPlan => conflict("guest_has_another_plan"),
            DbError::BookingChanged => conflict("booking_changed"),
            DbError::TableTaken => conflict("table_taken"),
            DbError::BookingHasStarted => conflict("booking_started"),
            DbError::BookingHasFinished => conflict("booking_finished"),
            DbError::PartyTooLarge {
                party_size,
                max_party,
            } => conflict("party_too_large").with_detail(
                serde_json::json!({ "party_size": party_size, "max_party": max_party }),
            ),
            DbError::ShiftNotBookable { .. } => conflict("shift_not_bookable"),
            DbError::MissingBlockReason => refused("missing_block_reason"),
            DbError::NoteTooLong { limit } => {
                refused("note_too_long").with_detail(serde_json::json!({ "limit": limit }))
            }
            DbError::NotTheRunningShift { service_day } => conflict("not_the_running_shift")
                .with_detail(serde_json::json!({ "service_date": service_day })),
            DbError::UnknownCancelReason => refused("unknown_cancel_reason"),
            DbError::UnknownMessage => refused("unknown_message"),
            DbError::NoBotChat => refused("no_bot_chat"),
            DbError::ProposedConfigInvalid(errors) => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "settings_invalid",
                error.to_string(),
            )
            .with_detail(serde_json::json!({
                "reasons": errors.iter().map(ToString::to_string).collect::<Vec<_>>(),
            })),
            DbError::WouldStrandBookings(conflicts) => conflict("would_strand_bookings")
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
            DbError::SettingsChanged => conflict("settings_changed"),
            DbError::UnusableProposal(detail) => {
                Self::bad_request("settings_unreadable", detail.clone())
            }
            DbError::Time(_) => refused("impossible_time"),
            DbError::UnknownTimezone(_) => refused("unknown_timezone"),
            DbError::StoredConfigInvalid(_)
            | DbError::CorruptRow { .. }
            | DbError::Database(_)
            | DbError::Migration(_) => Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                error.to_string(),
            ),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
