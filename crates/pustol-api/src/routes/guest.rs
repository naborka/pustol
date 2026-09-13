//! What a guest can do: see the bar, see their bookings, take one, give one back.

use axum::extract::State;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use pustol_db::bookings::{Channel, NewBooking};
use pustol_db::identity::ReminderChoice;
use pustol_db::records::bookings_of;
use pustol_domain::config::ValidConfig;
use pustol_domain::rebooking::{refused_on, replaced_on};
use pustol_domain::slots::slot_list;
use pustol_domain::{Booking, BookingId, Interval, ServiceDay};
use pustol_telegram::messages;
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::body::JsonBody;
use crate::dto::{
    Availability, AvailabilityQuery, BarView, BookingRequest, DayOffer, DayRail, DayRailQuery,
    GuestAvailability, GuestBooking, RemindersView, Session, UserView,
};
use crate::error::ApiResult;
use crate::params::{RequestPath, RequestQuery};
use crate::state::AppState;

/// The party the home screen speaks for.
///
/// "Сегодня свободно с 21:30" is a promise, and a promise has to be about a definite party. Two is
/// the picker's own default and by far the commonest booking, so the sentence on the card is the
/// one the very next screen will keep.
const HOME_CARD_PARTY: i32 = 2;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/session", get(session))
        .route("/availability", get(availability))
        .route("/days", get(days))
        .route("/booking", post(book))
        .route("/bookings/{id}", delete(cancel))
        .route("/reminders/opt-in", post(opt_in))
        .route("/reminders/dismiss", post(dismiss))
}

/// Everything the first screen needs, in one request.
///
/// A Mini App opens on a phone over whatever connection the bar's basement has; three round trips
/// to draw one card is three chances to show a spinner.
async fn session(State(state): State<AppState>, caller: Authenticated) -> ApiResult<Json<Session>> {
    let now = state.now();
    let (viewer, config, mine) = tokio::try_join!(
        caller.viewer(&state),
        async { Ok(state.store.config(state.bar).await?) },
        async {
            Ok(state
                .store
                .bookings_of_guest(state.bar, caller.user_id(), now)
                .await?)
        },
    )?;
    let today = config.current_service_day(now);
    let held = bookings_of(&mine);
    let tonight = state
        .store
        .day_offers(state.bar, &config, &[today], HOME_CARD_PARTY, now, &held)
        .await?;

    Ok(Json(Session {
        session_token: caller.session(state.bot_token()),
        user: UserView {
            id: viewer.account.id.0,
            first_name: viewer.account.first_name.clone(),
            username: viewer.account.username.clone(),
        },
        is_staff: viewer.is_staff,
        reminders: RemindersView::of(&viewer),
        bar: BarView::of(&config, today, now),
        bookings: mine
            .iter()
            .map(|record| GuestBooking::of(record, &held, &config, now))
            .collect(),
        bookable_days: pustol_domain::bookable_days(&config, today)
            .into_iter()
            .map(ServiceDay::date)
            .collect(),
        today_free_from_minutes: tonight.first().and_then(|offer| offer.free_from_minutes),
        today_free_for_party: HOME_CARD_PARTY,
    }))
}

/// What every day of the booking horizon holds for a party of this size.
///
/// The middle of the guest's three taps. Kept apart from `/availability` because it answers a
/// different question — *which evening*, not *which time* — and because it changes only when the
/// party size does: folding it into availability would recompute thirty days every time somebody
/// tapped a different day on the rail it had just drawn.
async fn days(
    State(state): State<AppState>,
    caller: Authenticated,
    RequestQuery(query): RequestQuery<DayRailQuery>,
) -> ApiResult<Json<DayRail>> {
    let now = state.now();
    let (config, mine) = tokio::try_join!(
        async { Ok(state.store.config(state.bar).await?) },
        own_bookings(&state, &caller, now),
    )?;
    let horizon = pustol_domain::horizon_days(&config, config.current_service_day(now));
    let offers = state
        .store
        .day_offers(state.bar, &config, &horizon, query.party_size, now, &mine)
        .await?;
    Ok(Json(DayRail {
        party_size: query.party_size,
        days: offers
            .into_iter()
            .map(|offer| DayOffer {
                service_date: offer.day.date(),
                closed: offer.closed,
                free_from_minutes: offer.free_from_minutes,
                booked: offer.booked,
            })
            .collect(),
    }))
}

/// Arrival times for a party on one shift, and what booking on it would do to what the guest holds.
///
/// A guest changing an existing booking must still see their own time as free, or the only way to
/// move from 20:00 to 20:30 would be to give up 20:00 first and hope. What is set aside is exactly
/// what a booking on that shift would replace — never a table they are sitting at.
async fn availability(
    State(state): State<AppState>,
    caller: Authenticated,
    RequestQuery(query): RequestQuery<AvailabilityQuery>,
) -> ApiResult<Json<GuestAvailability>> {
    let now = state.now();
    let day = query.service_date.day()?;
    let (room, mine) = tokio::try_join!(
        async { Ok(state.store.room(state.bar, day).await?) },
        own_bookings(&state, &caller, now),
    )?;
    let replacing = replaced_on(&mine, day, now);
    let slots = slot_list(&room.query(query.party_size, now, &replacing));
    Ok(Json(GuestAvailability {
        offer: Availability::of(day, query.party_size, &room.config, &slots),
        replacing: replacing.iter().map(|id| id.0).collect(),
        booked: refused_on(&mine, day, now),
    }))
}

/// The caller's own running bookings, which every offer made to them weighs by the rebooking rule.
async fn own_bookings(
    state: &AppState,
    caller: &Authenticated,
    now: chrono::DateTime<chrono::Utc>,
) -> ApiResult<Vec<Booking>> {
    Ok(bookings_of(
        &state
            .store
            .bookings_of_guest(state.bar, caller.user_id(), now)
            .await?,
    ))
}

#[derive(Debug, serde::Serialize)]
pub struct BookingTaken {
    pub booking: GuestBooking,
    /// Every booking this one replaced, soonest first, so the app can say so rather than appear to
    /// have lost them.
    pub replaced: Vec<Uuid>,
}

async fn book(
    State(state): State<AppState>,
    caller: Authenticated,
    JsonBody(request): JsonBody<BookingRequest>,
) -> ApiResult<Json<BookingTaken>> {
    let now = state.now();
    let service_day = request.service_date.day()?;
    // The account has to exist before a booking can point at it, and the name the booking is filed
    // under is the one stored for it rather than whatever a session remembered.
    let viewer = caller.viewer(&state).await?;

    let created = state
        .store
        .create_booking(
            &NewBooking {
                bar: state.bar,
                service_day,
                start_minutes: request.start_minutes,
                party_size: request.party_size,
                channel: Channel::Guest {
                    user: viewer.account.id,
                    name: viewer.account.first_name,
                    username: viewer.account.username,
                    replacing: request.replacing.into_iter().map(BookingId).collect(),
                },
                reminder: Some(word_reminder),
            },
            now,
        )
        .await?;

    Ok(Json(BookingTaken {
        booking: GuestBooking::of(
            &created.record,
            &bookings_of(&created.guest_bookings),
            &created.evening.config,
            now,
        ),
        replaced: created.replaced.iter().map(|id| id.0).collect(),
    }))
}

/// The reminder's words, handed to the store so it can compose them from the window it granted.
///
/// The copy lives here, with the rest of the bot's voice; the window it describes is derived once,
/// inside the transaction that decided it.
pub(crate) fn word_reminder(config: &ValidConfig, window: Interval, party_size: i32) -> String {
    messages::reminder(&config.name, window.start(), config.timezone, party_size)
}

/// A guest gives one of their tables back.
///
/// No reason is recorded and nothing is sent: they know why, and a bar that messages somebody about
/// a cancellation they just made themselves looks broken.
async fn cancel(
    State(state): State<AppState>,
    caller: Authenticated,
    RequestPath(id): RequestPath<Uuid>,
) -> ApiResult<Json<GuestBooking>> {
    let now = state.now();
    let cancelled = state
        .store
        .cancel_booking_of_guest(state.bar, caller.user_id(), BookingId(id), now)
        .await?;
    // Nothing replaces a cancelled booking, whatever else the guest holds.
    Ok(Json(GuestBooking::of(
        &cancelled.record,
        &[],
        &cancelled.evening.config,
        now,
    )))
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
    let viewer = caller.viewer(state).await?;
    let standing = state
        .store
        .choose_reminders(&viewer.account, choice, state.now())
        .await?;
    Ok(Json(RemindersView::of_standing(standing)))
}
