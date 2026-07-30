//! What staff can do: read the shift, move it, and change the rules.
//!
//! Every handler here takes [`Staff`], so a guest cannot reach any of them by guessing a URL: the
//! check is in the signature rather than in a line of code somebody could omit.

use axum::extract::{Path, Query, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use pustol_db::bookings::{Channel, NewBooking};
use pustol_db::records::BookingRecord;
use pustol_domain::config::ValidConfig;
use pustol_domain::draft::Draft;
use pustol_domain::schedule::next_table_number;
use pustol_domain::{BookingId, ServiceDay, TableId, minutes_within};
use pustol_telegram::messages;
use uuid::Uuid;

use crate::auth::Staff;
use crate::dto::{
    AttendanceRequest, Availability, AvailabilityQuery, BlockRequest, CancelRequest, Hours,
    LimitsView, MessageRequest, ReconcileRequest, ReconciliationView, SavedSettingsView,
    SettingsTable, SettingsView, ShiftBooking, ShiftQuery, ShiftStats, ShiftTable, ShiftView,
    StaffBookingRequest, StaffView, UnblockRequest,
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
        .route("/bookings/{id}/cancel", post(cancel_booking))
        .route("/bookings/{id}/message", post(send_message))
        .route("/blocks", post(block).delete(unblock))
        .route("/settings", get(settings).put(save_settings))
}

async fn shift(
    State(state): State<AppState>,
    _staff: Staff,
    Query(query): Query<ShiftQuery>,
) -> ApiResult<Json<ShiftView>> {
    let now = state.now();
    let day = ServiceDay::new(query.service_date);
    let config = state.store.config(state.bar).await?;
    let shift = state.store.shift(state.bar, day).await?;
    let hours = config.week.for_service_day(day);

    let tables: Vec<ShiftTable> = config
        .active_tables()
        .map(|table| ShiftTable {
            id: table.id.0,
            number: table.number,
            seats: table.seats,
            zone: table.zone.as_str().to_owned(),
            blocked_because: shift
                .blocks
                .iter()
                .find(|block| {
                    block.block.table_id == table.id && block.block.service_day == day
                })
                .map(|block| block.reason.clone()),
        })
        .collect();

    // "Free now" and the now-line are only meaningful on the shift that is actually running. On any
    // other day an invented number would be worse than a blank.
    let is_running = config.current_service_day(now) == day;
    let free_now = is_running.then(|| {
        tables
            .iter()
            .filter(|table| table.blocked_because.is_none())
            .filter(|table| {
                !shift.bookings.iter().any(|record| {
                    record.booking.table_id == Some(TableId(table.id))
                        && record.booking.window.start() <= now
                        && now < record.booking.window.end()
                })
            })
            .count()
    });
    let now_minutes = is_running.then(|| minutes_within(day, now, config.timezone));

    Ok(Json(ShiftView {
        service_date: day.date(),
        hours: hours.into(),
        tables,
        bookings: shift
            .bookings
            .iter()
            .map(|record| ShiftBooking::of(record, &config))
            .collect(),
        stats: ShiftStats {
            bookings: shift.bookings.len(),
            guests: shift
                .bookings
                .iter()
                .map(|record| record.booking.party_size)
                .sum(),
            free_now,
        },
        now_minutes,
        cancel_reasons: config.cancel_reasons.clone(),
        message_templates: config.message_templates.clone(),
    }))
}

async fn availability(
    State(state): State<AppState>,
    _staff: Staff,
    Query(query): Query<AvailabilityQuery>,
) -> ApiResult<Json<Availability>> {
    let day = ServiceDay::new(query.service_date);
    let reading = state
        .store
        .availability(state.bar, day, query.party_size, state.now(), None)
        .await?;
    Ok(Json(Availability::of(
        day,
        query.party_size,
        &reading.config,
        &reading.slots,
    )))
}

async fn create_booking(
    State(state): State<AppState>,
    _staff: Staff,
    Json(request): Json<StaffBookingRequest>,
) -> ApiResult<Json<ShiftBooking>> {
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
                service_day: ServiceDay::new(request.service_date),
                start_minutes: request.start_minutes,
                party_size: request.party_size,
                channel: Channel::Staff {
                    guest_name: request.guest_name.trim().to_owned(),
                },
                // A booking taken at the door has no account behind it, so there is nobody to
                // remind. The absence is structural, not a setting.
                reminder: None,
            },
            state.now(),
        )
        .await?;
    Ok(Json(ShiftBooking::of(&created.record, &created.config)))
}

async fn set_attendance(
    State(state): State<AppState>,
    _staff: Staff,
    Path(id): Path<Uuid>,
    Json(request): Json<AttendanceRequest>,
) -> ApiResult<Json<ShiftBooking>> {
    let config = state.store.config(state.bar).await?;
    let record = state
        .store
        .set_attendance(state.bar, BookingId(id), request.attendance)
        .await?;
    Ok(Json(ShiftBooking::of(&record, &config)))
}

#[derive(Debug, serde::Serialize)]
pub struct CancelledView {
    pub booking: ShiftBooking,
    /// Whatever the freed table let the room put right.
    pub reconciliation: ReconciliationView,
    /// Whether the guest will be told. False for a booking with no account behind it, which is
    /// exactly when staff have to pick up the telephone themselves.
    pub guest_notified: bool,
}

/// Staff release a table and say why.
///
/// The reason must be one the bar configured; that is checked by the store, in the transaction that
/// reads the configuration, so no handler can forget it.
async fn cancel_booking(
    State(state): State<AppState>,
    _staff: Staff,
    Path(id): Path<Uuid>,
    Json(request): Json<CancelRequest>,
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
        booking: ShiftBooking::of(&cancelled.record, &cancelled.config),
        reconciliation: ReconciliationView::of(&cancelled.reconciliation),
        guest_notified: cancelled.guest_notified,
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
    Json(request): Json<MessageRequest>,
) -> ApiResult<Json<MessageSent>> {
    let records = state
        .store
        .bookings_by_id(state.bar, &[BookingId(id)])
        .await?;
    let record = records.first().ok_or_else(|| ApiError::not_found("booking"))?;
    let Some(recipient) = record.telegram_user_id else {
        return Err(ApiError::bad_request(
            "no_bot_chat",
            "this booking was taken at the door, so the bot has no chat with the guest",
        ));
    };
    state
        .store
        .send_template(state.bar, BookingId(id), recipient, &request.text, state.now())
        .await?;
    Ok(Json(MessageSent { queued: true }))
}

async fn block(
    State(state): State<AppState>,
    staff: Staff,
    Json(request): Json<BlockRequest>,
) -> ApiResult<Json<ReconciliationView>> {
    if request.reason.trim().is_empty() {
        return Err(ApiError::bad_request(
            "missing_block_reason",
            "closing a table needs a reason staff can read later",
        ));
    }
    let tables: Vec<TableId> = request.table_ids.into_iter().map(TableId).collect();
    let outcome = state
        .store
        .block_tables(
            state.bar,
            ServiceDay::new(request.service_date),
            &tables,
            request.reason.trim(),
            Some(staff.viewer.account.id),
            state.now(),
        )
        .await?;
    Ok(Json(ReconciliationView::of(&outcome)))
}

async fn unblock(
    State(state): State<AppState>,
    _staff: Staff,
    Json(request): Json<UnblockRequest>,
) -> ApiResult<Json<ReconciliationView>> {
    let tables: Vec<TableId> = request.table_ids.into_iter().map(TableId).collect();
    let outcome = state
        .store
        .unblock_tables(
            state.bar,
            ServiceDay::new(request.service_date),
            &tables,
            state.now(),
        )
        .await?;
    Ok(Json(ReconciliationView::of(&outcome)))
}

/// The "find a table" action, for a booking the room could not seat.
async fn reconcile_shift(
    State(state): State<AppState>,
    _staff: Staff,
    Json(request): Json<ReconcileRequest>,
) -> ApiResult<Json<ReconciliationView>> {
    let outcome = state
        .store
        .reconcile_shift(
            state.bar,
            ServiceDay::new(request.service_date),
            state.now(),
        )
        .await?;
    Ok(Json(ReconciliationView::of(&outcome)))
}

async fn settings(
    State(state): State<AppState>,
    _staff: Staff,
    Query(query): Query<ShiftQuery>,
) -> ApiResult<Json<SettingsView>> {
    let config = state.store.config(state.bar).await?;
    let day = ServiceDay::new(query.service_date);
    let shift = state.store.shift(state.bar, day).await?;
    // Derived from the tables just loaded — including retired ones, which is what makes a number
    // never reused — rather than re-read in a second transaction that could disagree with this one.
    let next = next_table_number(&config.tables);
    Ok(Json(view_of(&config, &shift.bookings, next)))
}

async fn save_settings(
    State(state): State<AppState>,
    _staff: Staff,
    Query(query): Query<ShiftQuery>,
    Json(draft): Json<Draft>,
) -> ApiResult<Json<SavedSettingsView>> {
    let saved = state
        .store
        .save_settings(state.bar, &draft, state.now())
        .await?;
    let day = ServiceDay::new(query.service_date);
    let shift = state.store.shift(state.bar, day).await?;
    Ok(Json(SavedSettingsView {
        settings: view_of(&saved.config, &shift.bookings, saved.next_table_number),
        reconciliation: ReconciliationView::of(&saved.reconciliation),
        above_cap: saved.above_cap,
    }))
}

fn view_of(
    config: &pustol_domain::config::ValidConfig,
    shift: &[pustol_db::records::BookingRecord],
    next_table_number: i32,
) -> SettingsView {
    SettingsView {
        name: config.name.clone(),
        address: config.address.clone(),
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
                bookings_today: shift
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
        limits: LimitsView::current(),
    }
}
