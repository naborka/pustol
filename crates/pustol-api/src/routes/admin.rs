//! What staff can do: read the shift, move it, and change the rules.
//!
//! Every handler here takes [`Staff`], so a guest cannot reach any of them by guessing a URL: the
//! check is in the signature rather than in a line of code somebody could omit.
//!
//! Every write that can change the room answers with the evening as that write left it: read by the
//! store inside the write's own transaction, before it commits, and drawn by [`ShiftView::of`], the
//! same drawing `GET /shift` uses. A screen that reloaded the shift itself would draw whatever a
//! colleague did in between as if this write had done it, and a read after the commit could fail and
//! report a write that went through as one that did not.

use axum::extract::{Path, Query, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use pustol_db::bookings::{Attendance, Channel, MoveTo, MoveWords, NewBooking};
use pustol_db::evening::Evening;
use pustol_db::records::BookingRecord;
use pustol_domain::config::ValidConfig;
use pustol_domain::draft::Draft;
use pustol_domain::{BookingId, ServiceDay, TableId};
use pustol_telegram::messages;
use uuid::Uuid;

use crate::auth::Staff;
use crate::body::JsonBody;
use crate::dto::{
    AttendanceRequest, Availability, AvailabilityQuery, BlockRequest, CancelRequest, Hours,
    LimitsView, MessageRequest, MoveRequest, NoteRequest, ReconcileRequest, ReconciliationView,
    SavedSettingsView, SettingsTable, SettingsView, ShiftBooking, ShiftQuery, ShiftView,
    StaffBookingRequest, StaffView, UnblockRequest, WalkInRequest, in_calendar,
};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/shift", get(shift))
        .route("/shift/reconcile", post(reconcile_shift))
        .route("/availability", get(availability))
        .route("/bookings", post(create_booking))
        .route("/bookings/{id}/attendance", patch(set_attendance))
        .route("/bookings/{id}/note", patch(set_note))
        .route("/bookings/{id}/move", patch(move_booking))
        .route("/bookings/{id}/cancel", post(cancel_booking))
        .route("/bookings/{id}/message", post(send_message))
        .route("/walkins", post(seat_walk_in))
        .route("/blocks", post(block).delete(unblock))
        .route("/settings", get(settings).put(save_settings))
}

async fn shift(
    State(state): State<AppState>,
    _staff: Staff,
    Query(query): Query<ShiftQuery>,
) -> ApiResult<Json<ShiftView>> {
    let evening = state
        .store
        .evening(state.bar, query.service_date.day()?, state.now())
        .await?;
    Ok(Json(ShiftView::of(&evening)))
}

/// A booking as the evening a write left draws it, by that evening's configuration and clock.
fn drawn_in(record: &BookingRecord, evening: &Evening) -> ShiftBooking {
    ShiftBooking::of(record, &evening.config, evening.now)
}

async fn availability(
    State(state): State<AppState>,
    _staff: Staff,
    Query(query): Query<AvailabilityQuery>,
) -> ApiResult<Json<Availability>> {
    let day = query.service_date.day()?;
    let moving: Vec<BookingId> = query.ignoring.map(BookingId).into_iter().collect();
    let reading = state
        .store
        .availability(state.bar, day, query.party_size, state.now(), &moving)
        .await?;
    Ok(Json(Availability::of(
        day,
        query.party_size,
        &reading.config,
        &reading.slots,
    )))
}

/// A booking staff just wrote, and the evening it is on.
#[derive(Debug, serde::Serialize)]
pub struct BookedView {
    pub booking: ShiftBooking,
    pub shift: ShiftView,
}

async fn create_booking(
    State(state): State<AppState>,
    _staff: Staff,
    JsonBody(request): JsonBody<StaffBookingRequest>,
) -> ApiResult<Json<BookedView>> {
    let service_day = request.service_date.day()?;
    if request.guest_name.trim().is_empty() {
        return Err(ApiError::bad_request(
            "blank_guest_name",
            "a booking needs a name to call out",
        ));
    }
    let created = state
        .store
        .create_booking(
            &NewBooking {
                bar: state.bar,
                service_day,
                start_minutes: request.start_minutes,
                party_size: request.party_size,
                channel: Channel::Staff {
                    guest_name: request.guest_name.trim().to_owned(),
                    table: request.table_id.map(TableId),
                },
                // A booking taken at the door has no account behind it, so there is nobody to
                // remind. The absence is structural, not a setting.
                reminder: None,
            },
            state.now(),
        )
        .await?;
    Ok(Json(BookedView {
        booking: drawn_in(&created.record, &created.evening),
        shift: ShiftView::of(&created.evening),
    }))
}

/// Whether a party turned up, sat, or went home.
///
/// The room is re-seated inside the same transaction, so a table given back by a party that left
/// is offered straight to anybody the room could not seat. Nothing is reported about it here: the
/// evening in the answer simply has the `Без стола` group shorter, which is the honest amount of
/// noise for something that fixed itself.
async fn set_attendance(
    State(state): State<AppState>,
    _staff: Staff,
    Path(id): Path<Uuid>,
    JsonBody(request): JsonBody<AttendanceRequest>,
) -> ApiResult<Json<AttendanceView>> {
    let recorded = state
        .store
        .set_attendance(state.bar, BookingId(id), request.attendance, state.now())
        .await?;
    Ok(Json(AttendanceView {
        booking: drawn_in(&recorded.record, &recorded.evening),
        previous: recorded.previous,
        shift: ShiftView::of(&recorded.evening),
    }))
}

#[derive(Debug, serde::Serialize)]
pub struct AttendanceView {
    pub booking: ShiftBooking,
    /// What the booking recorded just before this change, which is what undo puts back. Read by
    /// the server in the same transaction, because the screen's own copy may be one a colleague has
    /// changed since.
    pub previous: Attendance,
    pub shift: ShiftView,
}

/// What staff want to remember about a booking.
///
/// Staff-facing by construction: the guest projection has no field to put a note in, so no future
/// handler can send one by accident.
async fn set_note(
    State(state): State<AppState>,
    _staff: Staff,
    Path(id): Path<Uuid>,
    JsonBody(request): JsonBody<NoteRequest>,
) -> ApiResult<Json<BookedView>> {
    let written = state
        .store
        .set_note(
            state.bar,
            BookingId(id),
            request.note.as_deref(),
            state.now(),
        )
        .await?;
    Ok(Json(BookedView {
        booking: drawn_in(&written.record, &written.evening),
        shift: ShiftView::of(&written.evening),
    }))
}

#[derive(Debug, serde::Serialize)]
pub struct MovedView {
    pub booking: ShiftBooking,
    /// Whatever the table they left let the room settle.
    pub reconciliation: ReconciliationView,
    /// Whether the guest will hear about it. Only a time change is theirs to hear about, and only a
    /// guest the bot can reach will.
    pub guest_notified: bool,
    pub shift: ShiftView,
}

/// Staff put a booking at another table, another time, or both.
async fn move_booking(
    State(state): State<AppState>,
    _staff: Staff,
    Path(id): Path<Uuid>,
    JsonBody(request): JsonBody<MoveRequest>,
) -> ApiResult<Json<MovedView>> {
    let moved = state
        .store
        .move_booking(
            state.bar,
            BookingId(id),
            MoveTo {
                start_minutes: request.start_minutes,
                table: request.table_id.map(TableId),
                party_size: request.party_size,
            },
            Some(MoveWords {
                notice: word_move,
                reminder: crate::routes::guest::word_reminder,
            }),
            state.now(),
        )
        .await?;
    Ok(Json(MovedView {
        booking: drawn_in(&moved.record, &moved.evening),
        reconciliation: ReconciliationView::of(&moved.reconciliation),
        guest_notified: moved.guest_notified,
        shift: ShiftView::of(&moved.evening),
    }))
}

/// The notice a guest gets when their time changes, worded here where the bot's voice lives.
fn word_move(config: &ValidConfig, was: &BookingRecord, now: &BookingRecord) -> String {
    messages::moved(
        &config.name,
        was.booking.window.start(),
        now.booking.window.start(),
        config.timezone,
        now.booking.party_size,
    )
}

/// Seats a party that walked in, at the minute they sat down, at the table staff chose.
async fn seat_walk_in(
    State(state): State<AppState>,
    _staff: Staff,
    JsonBody(request): JsonBody<WalkInRequest>,
) -> ApiResult<Json<BookedView>> {
    let seated = state
        .store
        .seat_walk_in(
            state.bar,
            request.service_date.day()?,
            request.party_size,
            request.table_id.map(TableId),
            state.now(),
        )
        .await?;
    Ok(Json(BookedView {
        booking: drawn_in(&seated.record, &seated.evening),
        shift: ShiftView::of(&seated.evening),
    }))
}

#[derive(Debug, serde::Serialize)]
pub struct CancelledView {
    pub booking: ShiftBooking,
    /// Whatever the freed table let the room put right.
    pub reconciliation: ReconciliationView,
    /// Whether the guest will be told. False for a booking with no account behind it, and for a
    /// guest the bot has found it cannot reach: exactly when staff have to pick up the telephone.
    pub guest_notified: bool,
    pub shift: ShiftView,
}

/// Staff release a table and say why.
///
/// The reason must be one the bar configured; that is checked by the store, in the transaction that
/// reads the configuration, so no handler can forget it.
async fn cancel_booking(
    State(state): State<AppState>,
    _staff: Staff,
    Path(id): Path<Uuid>,
    JsonBody(request): JsonBody<CancelRequest>,
) -> ApiResult<Json<CancelledView>> {
    let cancelled = state
        .store
        .cancel_booking(
            state.bar,
            BookingId(id),
            request.reason.as_deref(),
            Some(word_cancellation),
            state.now(),
        )
        .await?;
    Ok(Json(CancelledView {
        booking: drawn_in(&cancelled.record, &cancelled.evening),
        reconciliation: ReconciliationView::of(&cancelled.reconciliation),
        guest_notified: cancelled.guest_notified,
        shift: ShiftView::of(&cancelled.evening),
    }))
}

/// The notice a guest gets, worded here where the bot's voice lives and queued by the store in the
/// transaction that cancelled.
fn word_cancellation(config: &ValidConfig, record: &BookingRecord, reason: &str) -> String {
    messages::cancellation(
        &config.name,
        record.booking.window.start(),
        config.timezone,
        reason,
    )
}

#[derive(Debug, serde::Serialize)]
pub struct MessageSent {
    pub queued: bool,
}

async fn send_message(
    State(state): State<AppState>,
    _staff: Staff,
    Path(id): Path<Uuid>,
    JsonBody(request): JsonBody<MessageRequest>,
) -> ApiResult<Json<MessageSent>> {
    state
        .store
        .send_template(state.bar, BookingId(id), &request.text, state.now())
        .await?;
    Ok(Json(MessageSent { queued: true }))
}

#[derive(Debug, serde::Serialize)]
pub struct ClosedView {
    pub reconciliation: ReconciliationView,
    /// The tables this request closed. One already shut is not among them.
    pub closed: Vec<Uuid>,
    pub shift: ShiftView,
}

async fn block(
    State(state): State<AppState>,
    staff: Staff,
    JsonBody(request): JsonBody<BlockRequest>,
) -> ApiResult<Json<ClosedView>> {
    let day = request.service_date.day()?;
    if request.reason.trim().is_empty() {
        return Err(ApiError::bad_request(
            "missing_block_reason",
            "closing a table needs a reason staff can read later",
        ));
    }
    let tables: Vec<TableId> = request.table_ids.into_iter().map(TableId).collect();
    let closed = state
        .store
        .block_tables(
            state.bar,
            day,
            &tables,
            request.reason.trim(),
            Some(staff.viewer.account.id),
            state.now(),
        )
        .await?;
    Ok(Json(ClosedView {
        reconciliation: ReconciliationView::of(&closed.reconciliation),
        closed: closed.closed.iter().map(|table| table.0).collect(),
        shift: ShiftView::of(&closed.evening),
    }))
}

#[derive(Debug, serde::Serialize)]
pub struct ReopenedView {
    pub reconciliation: ReconciliationView,
    /// The closures this request removed, with the reason each had. A table that was not shut is
    /// not among them.
    pub reopened: Vec<ReopenedTableView>,
    pub shift: ShiftView,
}

#[derive(Debug, serde::Serialize)]
pub struct ReopenedTableView {
    pub table_id: Uuid,
    pub reason: String,
}

async fn unblock(
    State(state): State<AppState>,
    _staff: Staff,
    JsonBody(request): JsonBody<UnblockRequest>,
) -> ApiResult<Json<ReopenedView>> {
    let tables: Vec<TableId> = request.table_ids.into_iter().map(TableId).collect();
    let reopened = state
        .store
        .unblock_tables(
            state.bar,
            request.service_date.day()?,
            &tables,
            state.now(),
        )
        .await?;
    Ok(Json(ReopenedView {
        reconciliation: ReconciliationView::of(&reopened.reconciliation),
        reopened: reopened
            .reopened
            .into_iter()
            .map(|table| ReopenedTableView {
                table_id: table.table_id.0,
                reason: table.reason,
            })
            .collect(),
        shift: ShiftView::of(&reopened.evening),
    }))
}

#[derive(Debug, serde::Serialize)]
pub struct ReconciledView {
    pub reconciliation: ReconciliationView,
    pub shift: ShiftView,
}

/// The "find a table" action, for a booking the room could not seat.
async fn reconcile_shift(
    State(state): State<AppState>,
    _staff: Staff,
    JsonBody(request): JsonBody<ReconcileRequest>,
) -> ApiResult<Json<ReconciledView>> {
    let reconciled = state
        .store
        .reconcile_shift(state.bar, request.service_date.day()?, state.now())
        .await?;
    Ok(Json(ReconciledView {
        reconciliation: ReconciliationView::of(&reconciled.reconciliation),
        shift: ShiftView::of(&reconciled.evening),
    }))
}

async fn settings(
    State(state): State<AppState>,
    _staff: Staff,
    Query(query): Query<ShiftQuery>,
) -> ApiResult<Json<SettingsView>> {
    let reading = state
        .store
        .settings_on(state.bar, query.service_date.day()?)
        .await?;
    Ok(Json(view_of(
        &reading.config,
        reading.version,
        reading.day,
        &reading.bookings,
        reading.next_table_number,
    )))
}

async fn save_settings(
    State(state): State<AppState>,
    _staff: Staff,
    Query(query): Query<ShiftQuery>,
    JsonBody(draft): JsonBody<Draft>,
) -> ApiResult<Json<SavedSettingsView>> {
    let day = query.service_date.day()?;
    // The version is an instant rather than a date, and it is a date in a request all the same.
    if !in_calendar(draft.version.date_naive()) {
        return Err(ApiError::bad_request(
            "invalid_date",
            "the settings version is not a moment in the years 1 to 9999",
        ));
    }
    let saved = state
        .store
        .save_settings(state.bar, &draft, day, state.now())
        .await?;
    Ok(Json(SavedSettingsView {
        settings: view_of(
            &saved.config,
            saved.version,
            saved.day,
            &saved.bookings,
            saved.next_table_number,
        ),
        reconciliation: ReconciliationView::of(&saved.reconciliation),
        above_cap: saved.above_cap,
    }))
}

/// The settings screen, with each table's count of the bookings on `day`.
fn view_of(
    config: &ValidConfig,
    version: chrono::DateTime<chrono::Utc>,
    day: ServiceDay,
    bookings: &[BookingRecord],
    next_table_number: i32,
) -> SettingsView {
    SettingsView {
        version,
        name: config.name.clone(),
        address: config.address.clone(),
        contact: config.contact.clone().unwrap_or_default(),
        timezone: config.timezone.name().to_owned(),
        week: config.week.all().iter().copied().map(Hours::from).collect(),
        zones: config
            .zones
            .iter()
            .map(|zone| zone.as_str().to_owned())
            .collect(),
        tables: config
            .active_tables()
            .map(|table| SettingsTable {
                id: table.id.0,
                number: table.number,
                seats: table.seats,
                zone: table.zone.as_str().to_owned(),
                bookings_today: bookings
                    .iter()
                    .filter(|record| record.booking.table_id == Some(table.id))
                    .count(),
            })
            .collect(),
        turn_minutes: config.turn_minutes,
        slot_step_minutes: config.slot_step_minutes,
        max_party: config.max_party,
        horizon_days: config.horizon_days,
        remind_hours: config.remind_hours,
        grace_minutes: config.grace_minutes,
        message_templates: config.message_templates.clone(),
        cancel_reasons: config.cancel_reasons.clone(),
        staff: config
            .staff
            .iter()
            .map(|member| StaffView {
                username: member.username.clone(),
                bound: member.telegram_user_id.is_some(),
            })
            .collect(),
        next_table_number,
        service_date: day.date(),
        limits: LimitsView::current(),
    }
}
