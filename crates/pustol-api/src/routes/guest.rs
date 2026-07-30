//! What a guest can do: see the bar, see their booking, take one, give it back.

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use pustol_db::bookings::{Channel, NewBooking};
use pustol_db::identity::ReminderChoice;
use pustol_domain::config::ValidConfig;
use pustol_domain::{Interval, ServiceDay};
use pustol_telegram::messages;

use crate::auth::Authenticated;
use crate::dto::{
    Availability, AvailabilityQuery, BarView, BookingRequest, GuestBooking, RemindersView, Session,
    UserView,
};
use crate::error::ApiResult;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/session", get(session))
        .route("/availability", get(availability))
        .route("/booking", post(book).delete(cancel))
        .route("/reminders/opt-in", post(opt_in))
        .route("/reminders/dismiss", post(dismiss))
}

/// Everything the first screen needs, in one request.
///
/// A Mini App opens on a phone over whatever connection the bar's basement has; three round trips
/// to draw one card is three chances to show a spinner.
async fn session(
    State(state): State<AppState>,
    caller: Authenticated,
) -> ApiResult<Json<Session>> {
    let now = state.now();
    let viewer = state
        .store
        .identify(state.bar, &caller.account(), now)
        .await?;
    let config = state.store.config(state.bar).await?;
    let today = config.current_service_day(now);
    let booking = state
        .store
        .booking_of_guest(state.bar, caller.user_id(), now)
        .await?;

    Ok(Json(Session {
        user: UserView {
            id: viewer.account.id.0,
            first_name: viewer.account.first_name.clone(),
            username: viewer.account.username.clone(),
        },
        is_staff: viewer.is_staff,
        reminders: RemindersView::of(&viewer),
        bar: BarView::of(&config, today),
        booking: booking
            .as_ref()
            .map(|record| GuestBooking::of(record, &config)),
        bookable_days: pustol_domain::bookable_days(&config, today)
            .into_iter()
            .map(ServiceDay::date)
            .collect(),
    }))
}

/// Arrival times for a party on one shift.
///
/// A guest changing an existing booking must still see their own time as free, or the only way to
/// move from 20:00 to 20:30 would be to give up 20:00 first and hope.
async fn availability(
    State(state): State<AppState>,
    caller: Authenticated,
    Query(query): Query<AvailabilityQuery>,
) -> ApiResult<Json<Availability>> {
    let now = state.now();
    let day = ServiceDay::new(query.service_date);
    let mine = state
        .store
        .booking_of_guest(state.bar, caller.user_id(), now)
        .await?;
    let reading = state
        .store
        .availability(
            state.bar,
            day,
            query.party_size,
            now,
            mine.as_ref().map(|record| record.booking.id),
        )
        .await?;
    Ok(Json(Availability::of(
        day,
        query.party_size,
        &reading.config,
        &reading.slots,
    )))
}

#[derive(Debug, serde::Serialize)]
pub struct BookingTaken {
    pub booking: GuestBooking,
    /// The booking this one replaced, so the app can say so rather than appear to have lost it.
    pub replaced: Option<uuid::Uuid>,
}

async fn book(
    State(state): State<AppState>,
    caller: Authenticated,
    Json(request): Json<BookingRequest>,
) -> ApiResult<Json<BookingTaken>> {
    let now = state.now();
    // The account has to exist before a booking can point at it, and this is also the moment a
    // freshly invited member of staff is recognised.
    state
        .store
        .identify(state.bar, &caller.account(), now)
        .await?;

    let created = state
        .store
        .create_booking(
            &NewBooking {
                bar: state.bar,
                service_day: ServiceDay::new(request.service_date),
                start_minutes: request.start_minutes,
                party_size: request.party_size,
                channel: Channel::Guest {
                    user: caller.user_id(),
                    name: caller.init_data.user.first_name.clone(),
                    username: caller.init_data.user.username.clone(),
                },
                reminder: Some(word_reminder),
            },
            now,
        )
        .await?;

    Ok(Json(BookingTaken {
        booking: GuestBooking::of(&created.record, &created.config),
        replaced: created.replaced.map(|id| id.0),
    }))
}

/// The reminder's words, handed to the store so it can compose them from the window it granted.
///
/// The copy lives here, with the rest of the bot's voice; the window it describes is derived once,
/// inside the transaction that decided it.
fn word_reminder(config: &ValidConfig, window: Interval, party_size: i32) -> String {
    messages::reminder(&config.name, window.start(), config.timezone, party_size)
}

/// A guest gives their table back.
///
/// No reason is recorded and nothing is sent: they know why, and a bar that messages somebody about
/// a cancellation they just made themselves looks broken.
async fn cancel(
    State(state): State<AppState>,
    caller: Authenticated,
) -> ApiResult<Json<GuestBooking>> {
    let cancelled = state
        .store
        .cancel_booking_of_guest(state.bar, caller.user_id(), state.now())
        .await?;
    Ok(Json(GuestBooking::of(&cancelled.record, &cancelled.config)))
}

async fn opt_in(
    State(state): State<AppState>,
    caller: Authenticated,
) -> ApiResult<Json<RemindersView>> {
    choose(&state, &caller, ReminderChoice::OptIn).await
}

async fn dismiss(
    State(state): State<AppState>,
    caller: Authenticated,
) -> ApiResult<Json<RemindersView>> {
    choose(&state, &caller, ReminderChoice::NotNow).await
}

/// Records the choice and answers with where it leaves the guest, in one round trip.
async fn choose(
    state: &AppState,
    caller: &Authenticated,
    choice: ReminderChoice,
) -> ApiResult<Json<RemindersView>> {
    let standing = state
        .store
        .choose_reminders(&caller.account(), choice, state.now())
        .await?;
    Ok(Json(RemindersView::of_standing(standing)))
}
