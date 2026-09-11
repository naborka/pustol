//! What the bar has decided: the room, the week, and the rules a booking is taken under.
//!
//! [`BarConfig::validate`] is the single predicate for "is this configuration legal". The
//! settings screen greys out a control by asking whether the change it would make validates,
//! and the save endpoint refuses by asking the same question of the same value. There is no
//! second place where a rule is spelled out, so a control can never look live while the server
//! refuses it — nor the reverse, which is worse.
//!
//! Deliberately absent from this type is `last_arrival`: the latest time a party may arrive is
//! closing time minus one turn, and storing it would create a second source for one fact that
//! would eventually disagree with the first.

use chrono::{DateTime, Utc, Weekday};
use chrono_tz::Tz;

use crate::allocator::{Booking, BookingId};
use crate::schedule::{BarTable, Zone};
use crate::service_day::{ServiceDay, minutes_within};

/// An inclusive integer range a setting must fall in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bounds {
    pub min: i32,
    pub max: i32,
}

impl Bounds {
    #[must_use]
    pub const fn contains(self, value: i32) -> bool {
        value >= self.min && value <= self.max
    }
}

/// The outer bounds of every numeric setting.
///
/// These are policy, not representability: the database guards only against values that make no
/// sense at all (negative seats, a close before an open). Keeping policy in one place here means
/// widening a limit is a one-line change that the settings UI picks up for free.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub open_minutes: Bounds,
    pub close_minutes: Bounds,
    pub turn_minutes: Bounds,
    pub max_party: Bounds,
    pub horizon_days: Bounds,
    pub remind_hours: Bounds,
    pub grace_minutes: Bounds,
    pub seats: Bounds,
    pub slot_step_minutes: &'static [i32],
    pub staff_username_length: Bounds,
}

/// The limits this deployment runs under.
pub const LIMITS: Limits = Limits {
    open_minutes: Bounds { min: 480, max: 1080 },
    close_minutes: Bounds {
        min: 1200,
        max: 1680,
    },
    turn_minutes: Bounds { min: 60, max: 240 },
    max_party: Bounds { min: 2, max: 10 },
    horizon_days: Bounds { min: 1, max: 30 },
    remind_hours: Bounds { min: 1, max: 12 },
    grace_minutes: Bounds { min: 5, max: 60 },
    seats: Bounds { min: 1, max: 12 },
    slot_step_minutes: &[15, 30, 60],
    staff_username_length: Bounds { min: 5, max: 32 },
};

/// Wall-clock minute from which a slot counts as an evening slot rather than a daytime one.
///
/// The guest picker shows the evening by default and folds the daytime hours behind a
/// disclosure, because a bar sells most of its tables after this hour and the picker should not
/// open on four rows of times nobody wants.
pub const EVENING_FROM_MINUTES: i32 = 17 * 60;

/// One weekday's opening hours, in wall-clock minutes from midnight.
///
/// `close_minutes` may exceed 1440: a bar that shuts at 02:00 closes at minute 1560 of the
/// service day that opened the previous calendar evening.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DayHours {
    pub open_minutes: i32,
    pub close_minutes: i32,
    pub closed: bool,
}

impl DayHours {
    /// Length of the shift in minutes.
    #[must_use]
    pub const fn shift_minutes(self) -> i32 {
        self.close_minutes - self.open_minutes
    }
}

/// Opening hours for each weekday, indexed the way `chrono` counts from Sunday.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WeekSchedule {
    days: [DayHours; 7],
}

impl WeekSchedule {
    #[must_use]
    pub const fn new(days: [DayHours; 7]) -> Self {
        Self { days }
    }

    /// Every day the same.
    #[must_use]
    pub const fn uniform(hours: DayHours) -> Self {
        Self { days: [hours; 7] }
    }

    #[must_use]
    pub fn on(&self, weekday: Weekday) -> DayHours {
        self.days[weekday.num_days_from_sunday() as usize]
    }

    /// Hours of the shift `day` belongs to.
    #[must_use]
    pub fn for_service_day(&self, day: ServiceDay) -> DayHours {
        self.on(day.weekday())
    }

    #[must_use]
    pub const fn all(&self) -> &[DayHours; 7] {
        &self.days
    }

    /// The same week with one weekday replaced.
    #[must_use]
    pub fn with(&self, weekday: Weekday, hours: DayHours) -> Self {
        let mut days = self.days;
        days[weekday.num_days_from_sunday() as usize] = hours;
        Self { days }
    }
}

/// Somebody allowed into the admin side of the app.
///
/// Invitations are written as usernames because that is what one member of staff can tell
/// another, but authorisation is by `telegram_user_id` once it is known: a username can be
/// released and taken by somebody else, a numeric id cannot. The id is bound the first time the
/// invited person opens the app, and from then on the username is only a label.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StaffMember {
    pub username: String,
    pub telegram_user_id: Option<i64>,
}

/// Everything the bar has decided.
///
/// Intentionally not `Serialize`: this aggregate holds the staff roster, and a type that cannot
/// be serialised cannot be leaked into a guest-facing response by an absent-minded derive. The
/// API declares explicit projections for each audience instead.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BarConfig {
    pub name: String,
    pub address: String,
    pub timezone: Tz,
    pub week: WeekSchedule,
    pub zones: Vec<Zone>,
    pub tables: Vec<BarTable>,
    pub turn_minutes: i32,
    pub slot_step_minutes: i32,
    pub max_party: i32,
    pub horizon_days: i32,
    pub remind_hours: i32,
    pub grace_minutes: i32,
    pub message_templates: Vec<String>,
    pub cancel_reasons: Vec<String>,
    pub staff: Vec<StaffMember>,
}

impl BarConfig {
    /// Tables that are part of the live room.
    pub fn active_tables(&self) -> impl Iterator<Item = &BarTable> {
        self.tables.iter().filter(|table| table.is_active())
    }

    /// Seats at the largest live table, or zero for an empty room.
    #[must_use]
    pub fn largest_table_seats(&self) -> i32 {
        self.active_tables().map(|table| table.seats).max().unwrap_or(0)
    }

    /// Total live seats — the settings summary line.
    #[must_use]
    pub fn total_seats(&self) -> i32 {
        self.active_tables().map(|table| table.seats).sum()
    }

    /// Latest wall-clock minute a party may arrive on `weekday`, or `None` when closed.
    ///
    /// Derived, never stored: it is closing time minus one turn, and a stored copy would drift
    /// away from the two facts it is made of.
    #[must_use]
    pub fn last_arrival_minutes(&self, weekday: Weekday) -> Option<i32> {
        let hours = self.week.on(weekday);
        (!hours.closed).then(|| hours.close_minutes - self.turn_minutes)
    }

    /// The shift that is running, or about to run, at `now`.
    ///
    /// Not the calendar date. At one in the morning a bar that shuts at two is still working
    /// yesterday's shift, and a guest tapping "tonight" means the evening they are currently
    /// sitting in. Getting this wrong would move the whole day strip forward by one at midnight
    /// and show staff an empty room while the room is full.
    #[must_use]
    pub fn current_service_day(&self, now: DateTime<Utc>) -> ServiceDay {
        let today = ServiceDay::new(now.with_timezone(&self.timezone).date_naive());
        if let Some(yesterday) = today.checked_sub_days(1) {
            let hours = self.week.for_service_day(yesterday);
            if !hours.closed
                && minutes_within(yesterday, now, self.timezone) < hours.close_minutes
            {
                return yesterday;
            }
        }
        today
    }

    /// Every reason this configuration is illegal. Empty means legal.
    ///
    /// All reasons are reported, not just the first: a settings screen that fixes one problem
    /// only to be told about the next is a screen people stop trusting.
    #[must_use]
    pub fn validate(&self) -> Vec<ConfigError> {
        let mut errors = Vec::new();
        self.check_identity(&mut errors);
        self.check_week(&mut errors);
        self.check_rules(&mut errors);
        self.check_room(&mut errors);
        self.check_lists(&mut errors);
        self.check_staff(&mut errors);
        errors
    }

    /// Whether this configuration is legal.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }

    fn check_identity(&self, errors: &mut Vec<ConfigError>) {
        if self.name.trim().is_empty() {
            errors.push(ConfigError::BlankName);
        }
        if self.address.trim().is_empty() {
            errors.push(ConfigError::BlankAddress);
        }
    }

    fn check_week(&self, errors: &mut Vec<ConfigError>) {
        for weekday in weekdays() {
            let hours = self.week.on(weekday);
            if !LIMITS.open_minutes.contains(hours.open_minutes) {
                errors.push(ConfigError::OpenOutOfRange {
                    weekday,
                    minutes: hours.open_minutes,
                });
            }
            if !LIMITS.close_minutes.contains(hours.close_minutes) {
                errors.push(ConfigError::CloseOutOfRange {
                    weekday,
                    minutes: hours.close_minutes,
                });
            }
            if !hours.closed && hours.close_minutes - self.turn_minutes < hours.open_minutes {
                errors.push(ConfigError::ShiftShorterThanTurn {
                    weekday,
                    shift_minutes: hours.shift_minutes(),
                    turn_minutes: self.turn_minutes,
                });
            }
        }
        if self.week.all().iter().all(|hours| hours.closed) {
            errors.push(ConfigError::EveryDayClosed);
        }
    }

    fn check_rules(&self, errors: &mut Vec<ConfigError>) {
        for (value, bounds, setting) in [
            (self.turn_minutes, LIMITS.turn_minutes, Setting::TurnMinutes),
            (self.max_party, LIMITS.max_party, Setting::MaxParty),
            (self.horizon_days, LIMITS.horizon_days, Setting::HorizonDays),
            (self.remind_hours, LIMITS.remind_hours, Setting::RemindHours),
            (self.grace_minutes, LIMITS.grace_minutes, Setting::GraceMinutes),
        ] {
            if !bounds.contains(value) {
                errors.push(ConfigError::SettingOutOfRange {
                    setting,
                    value,
                    bounds,
                });
            }
        }
        if !LIMITS
            .slot_step_minutes
            .contains(&self.slot_step_minutes)
        {
            errors.push(ConfigError::SlotStepNotOffered {
                minutes: self.slot_step_minutes,
            });
        }
    }

    fn check_room(&self, errors: &mut Vec<ConfigError>) {
        if self.zones.is_empty() {
            errors.push(ConfigError::NoZones);
        }
        for (index, zone) in self.zones.iter().enumerate() {
            if self.zones[..index].contains(zone) {
                errors.push(ConfigError::DuplicateZone { zone: zone.clone() });
            }
        }

        if self.active_tables().next().is_none() {
            errors.push(ConfigError::NoTables);
        }
        for (index, table) in self.tables.iter().enumerate() {
            if self.tables[..index].iter().any(|other| other.number == table.number) {
                errors.push(ConfigError::DuplicateTableNumber {
                    number: table.number,
                });
            }
            if !table.is_active() {
                continue;
            }
            if !LIMITS.seats.contains(table.seats) {
                errors.push(ConfigError::SeatsOutOfRange {
                    number: table.number,
                    seats: table.seats,
                });
            }
            if !self.zones.contains(&table.zone) {
                errors.push(ConfigError::UnknownZone {
                    number: table.number,
                    zone: table.zone.clone(),
                });
            }
        }

        // A party size no table can seat is unserveable by construction: the guest would be
        // offered a size for which every slot is grey, with nothing on screen explaining why.
        let largest = self.largest_table_seats();
        if self.max_party > largest {
            errors.push(ConfigError::MaxPartyExceedsLargestTable {
                max_party: self.max_party,
                largest_table_seats: largest,
            });
        }
    }

    fn check_lists(&self, errors: &mut Vec<ConfigError>) {
        if self.message_templates.is_empty() {
            errors.push(ConfigError::NoMessageTemplates);
        }
        if self
            .message_templates
            .iter()
            .any(|text| text.trim().is_empty())
        {
            errors.push(ConfigError::BlankMessageTemplate);
        }
        if self.cancel_reasons.is_empty() {
            errors.push(ConfigError::NoCancelReasons);
        }
        if self.cancel_reasons.iter().any(|text| text.trim().is_empty()) {
            errors.push(ConfigError::BlankCancelReason);
        }
    }

    fn check_staff(&self, errors: &mut Vec<ConfigError>) {
        if self.staff.is_empty() {
            errors.push(ConfigError::NoStaff);
        }
        for (index, member) in self.staff.iter().enumerate() {
            if !is_telegram_username(&member.username) {
                errors.push(ConfigError::MalformedStaffUsername {
                    username: member.username.clone(),
                });
            }
            if self.staff[..index]
                .iter()
                .any(|other| other.username.eq_ignore_ascii_case(&member.username))
            {
                errors.push(ConfigError::DuplicateStaffUsername {
                    username: member.username.clone(),
                });
            }
        }
    }
}

/// A configuration that has been checked and is in force.
///
/// The settings screen edits a [`BarConfig`], which is a *proposal* and may be illegal at any
/// moment while somebody is typing. Everything that actually runs the bar — slot generation,
/// allocation, reminders — takes this type instead, so the question "was this config validated?"
/// is answered by the type system rather than by remembering to call a function. That removes a
/// whole class of bug at the root: a slot generator handed a zero-minute time step cannot loop
/// forever if a zero-minute time step cannot reach it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValidConfig(BarConfig);

impl ValidConfig {
    /// Checks a proposal and puts it beyond doubt, or reports every reason it cannot be used.
    pub fn new(config: BarConfig) -> Result<Self, Vec<ConfigError>> {
        let errors = config.validate();
        if errors.is_empty() {
            Ok(Self(config))
        } else {
            Err(errors)
        }
    }

    /// The proposal this was made from, for the settings screen to edit again.
    #[must_use]
    pub fn into_inner(self) -> BarConfig {
        self.0
    }
}

impl std::ops::Deref for ValidConfig {
    type Target = BarConfig;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TryFrom<BarConfig> for ValidConfig {
    type Error = Vec<ConfigError>;

    fn try_from(config: BarConfig) -> Result<Self, Self::Error> {
        Self::new(config)
    }
}

/// A numeric setting, named so an out-of-range error can say which one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Setting {
    TurnMinutes,
    MaxParty,
    HorizonDays,
    RemindHours,
    GraceMinutes,
}

/// A reason a configuration is illegal, specific enough to render next to the control at fault.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("the bar needs a name")]
    BlankName,
    #[error("the bar needs an address")]
    BlankAddress,
    #[error("{weekday:?} opens at minute {minutes}, outside the allowed opening times")]
    OpenOutOfRange { weekday: Weekday, minutes: i32 },
    #[error("{weekday:?} closes at minute {minutes}, outside the allowed closing times")]
    CloseOutOfRange { weekday: Weekday, minutes: i32 },
    #[error("the bar cannot be closed every day of the week")]
    EveryDayClosed,
    #[error(
        "{weekday:?} is only {shift_minutes} minutes long, which cannot hold one {turn_minutes} minute booking"
    )]
    ShiftShorterThanTurn {
        weekday: Weekday,
        shift_minutes: i32,
        turn_minutes: i32,
    },
    #[error("{setting:?} is {value}, outside {}..={}", bounds.min, bounds.max)]
    SettingOutOfRange {
        setting: Setting,
        value: i32,
        bounds: Bounds,
    },
    #[error("a {minutes} minute time step is not one of the offered steps")]
    SlotStepNotOffered { minutes: i32 },
    #[error("the bar needs at least one zone")]
    NoZones,
    #[error("zone {zone} is listed twice")]
    DuplicateZone { zone: Zone },
    #[error("the bar needs at least one table")]
    NoTables,
    #[error("table number {number} is used twice")]
    DuplicateTableNumber { number: i32 },
    #[error("table {number} seats {seats}, outside the allowed table sizes")]
    SeatsOutOfRange { number: i32, seats: i32 },
    #[error("table {number} is in zone {zone}, which the bar does not have")]
    UnknownZone { number: i32, zone: Zone },
    #[error(
        "parties of {max_party} cannot be seated: the largest table seats {largest_table_seats}"
    )]
    MaxPartyExceedsLargestTable {
        max_party: i32,
        largest_table_seats: i32,
    },
    #[error("the bar needs at least one message to send guests")]
    NoMessageTemplates,
    #[error("a guest message cannot be blank")]
    BlankMessageTemplate,
    #[error("the bar needs at least one cancellation reason")]
    NoCancelReasons,
    #[error("a cancellation reason cannot be blank")]
    BlankCancelReason,
    #[error("removing the last admin would lock everyone out")]
    NoStaff,
    #[error("{username} is not a Telegram username")]
    MalformedStaffUsername { username: String },
    #[error("{username} is on the admin list twice")]
    DuplicateStaffUsername { username: String },
}

/// A live booking the new configuration cannot honour.
///
/// Opening hours are a physical fact: a party cannot be served on a day the bar is shut, or an
/// hour after the lights go off. Such a change is refused with the bookings named, so staff move
/// or cancel them deliberately rather than discovering the clash on the night.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScheduleConflict {
    /// The shift the booking sits on would become a day off.
    DayBecameClosed {
        booking: BookingId,
        service_day: ServiceDay,
    },
    /// The booking would fall partly or wholly outside the new hours.
    OutsideOpeningHours {
        booking: BookingId,
        service_day: ServiceDay,
        start_minutes: i32,
        end_minutes: i32,
        open_minutes: i32,
        close_minutes: i32,
    },
}

impl ScheduleConflict {
    #[must_use]
    pub const fn booking(self) -> BookingId {
        match self {
            Self::DayBecameClosed { booking, .. } | Self::OutsideOpeningHours { booking, .. } => {
                booking
            }
        }
    }

    #[must_use]
    pub const fn service_day(self) -> ServiceDay {
        match self {
            Self::DayBecameClosed { service_day, .. }
            | Self::OutsideOpeningHours { service_day, .. } => service_day,
        }
    }

    /// When the booking arrives, where that is what makes it a conflict.
    #[must_use]
    pub const fn start_minutes(self) -> Option<i32> {
        match self {
            Self::DayBecameClosed { .. } => None,
            Self::OutsideOpeningHours { start_minutes, .. } => Some(start_minutes),
        }
    }
}

/// Bookings that `config` would strand, considering only those that have not finished yet.
///
/// Filtering by `now` here rather than at the call site is deliberate. Bookings that have
/// already happened cannot be moved or cancelled, so counting them as conflicts would make the
/// opening hours permanently unchangeable — the bar would be held hostage by its own history.
#[must_use]
pub fn schedule_conflicts(
    config: &ValidConfig,
    bookings: &[Booking],
    now: DateTime<Utc>,
) -> Vec<ScheduleConflict> {
    bookings
        .iter()
        .filter(|booking| booking.occupancy().is_some_and(|held| held.end() > now))
        .filter_map(|booking| conflict_for(config, booking))
        .collect()
}

fn conflict_for(config: &ValidConfig, booking: &Booking) -> Option<ScheduleConflict> {
    let hours = config.week.for_service_day(booking.service_day);
    if hours.closed {
        return Some(ScheduleConflict::DayBecameClosed {
            booking: booking.id,
            service_day: booking.service_day,
        });
    }
    let start_minutes = minutes_within(booking.service_day, booking.window.start(), config.timezone);
    let end_minutes = minutes_within(booking.service_day, booking.window.end(), config.timezone);
    if start_minutes < hours.open_minutes || end_minutes > hours.close_minutes {
        return Some(ScheduleConflict::OutsideOpeningHours {
            booking: booking.id,
            service_day: booking.service_day,
            start_minutes,
            end_minutes,
            open_minutes: hours.open_minutes,
            close_minutes: hours.close_minutes,
        });
    }
    None
}

/// Live bookings for parties larger than the new cap.
///
/// Not a conflict: these parties already have a table and will be served. Staff are told the
/// count so that a cap they lowered by accident is visible immediately.
#[must_use]
pub fn parties_above_cap(
    config: &ValidConfig,
    bookings: &[Booking],
    now: DateTime<Utc>,
) -> Vec<BookingId> {
    bookings
        .iter()
        .filter(|booking| booking.occupancy().is_some_and(|held| held.end() > now))
        .filter(|booking| booking.party_size > config.max_party)
        .map(|booking| booking.id)
        .collect()
}

/// Whether `candidate` could be a Telegram username.
///
/// Telegram publishes the rule: five to thirty-two characters, letters, digits and underscores,
/// starting with a letter. Staff whose username predates the rule are added by numeric id
/// instead, so being strict here locks nobody out.
#[must_use]
pub fn is_telegram_username(candidate: &str) -> bool {
    let length = i32::try_from(candidate.chars().count()).unwrap_or(i32::MAX);
    LIMITS.staff_username_length.contains(length)
        && candidate.starts_with(|first: char| first.is_ascii_alphabetic())
        && candidate
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn weekdays() -> impl Iterator<Item = Weekday> {
    [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ]
    .into_iter()
}
