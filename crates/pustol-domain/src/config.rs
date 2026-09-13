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
use crate::service_day::{Interval, ServiceDay, minutes_within, resolve_boundary, resolve_end};
use crate::slots::last_sitting;
use crate::text::longer_than;

/// An inclusive integer range a setting must fall in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
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
    pub text: TextLimits,
    pub lists: ListLimits,
}

/// Max length per text, in characters. Longer message Telegram refuses on every send; longer name,
/// reason or zone breaks screens.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct TextLimits {
    pub name: usize,
    pub address: usize,
    pub message: usize,
    pub reason: usize,
    pub zone: usize,
}

/// Max entries per list. Whole config travels in one save; unbounded, legal config could outgrow
/// request limit and never save again.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct ListLimits {
    pub message_templates: usize,
    pub cancel_reasons: usize,
    pub zones: usize,
    pub staff: usize,
    /// Live tables only. Retired ones are history kept forever; counting them would lock out bar
    /// that replaced furniture often.
    pub tables: usize,
}

/// The limits this deployment runs under.
pub const LIMITS: Limits = Limits {
    open_minutes: Bounds {
        min: 480,
        max: 1080,
    },
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
    text: TextLimits {
        name: 100,
        address: 200,
        message: 1000,
        reason: 200,
        zone: 40,
    },
    lists: ListLimits {
        message_templates: 20,
        cancel_reasons: 20,
        zones: 20,
        staff: 50,
        tables: 100,
    },
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
    /// `i64`: unvalidated `i32` hours overflow on subtraction.
    #[must_use]
    pub fn shift_minutes(self) -> i64 {
        i64::from(self.close_minutes) - i64::from(self.open_minutes)
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
    /// Phone or Telegram username, kept as typed; [`Contact::parse`] reads it.
    pub contact: Option<String>,
}

impl BarConfig {
    /// Tables that are part of the live room.
    pub fn active_tables(&self) -> impl Iterator<Item = &BarTable> {
        self.tables.iter().filter(|table| table.is_active())
    }

    /// Seats at the largest live table, or zero for an empty room.
    #[must_use]
    pub fn largest_table_seats(&self) -> i32 {
        self.active_tables()
            .map(|table| table.seats)
            .max()
            .unwrap_or(0)
    }

    /// `None` when absent or unparseable.
    #[must_use]
    pub fn contact(&self) -> Option<Contact> {
        self.contact.as_deref().and_then(Contact::parse)
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
        self.check_list_lengths(&mut errors);
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
        if longer_than(&self.name, LIMITS.text.name) {
            errors.push(ConfigError::NameTooLong {
                limit: LIMITS.text.name,
            });
        }
        if longer_than(&self.address, LIMITS.text.address) {
            errors.push(ConfigError::AddressTooLong {
                limit: LIMITS.text.address,
            });
        }
        if self.contact.is_some() && self.contact().is_none() {
            errors.push(ConfigError::MalformedContact);
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
            if !hours.closed
                && i64::from(hours.close_minutes) - i64::from(self.turn_minutes)
                    < i64::from(hours.open_minutes)
            {
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
            (
                self.grace_minutes,
                LIMITS.grace_minutes,
                Setting::GraceMinutes,
            ),
        ] {
            if !bounds.contains(value) {
                errors.push(ConfigError::SettingOutOfRange {
                    setting,
                    value,
                    bounds,
                });
            }
        }
        if !LIMITS.slot_step_minutes.contains(&self.slot_step_minutes) {
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
        if self
            .zones
            .iter()
            .any(|zone| longer_than(zone.as_str(), LIMITS.text.zone))
        {
            errors.push(ConfigError::ZoneNameTooLong {
                limit: LIMITS.text.zone,
            });
        }

        if self.active_tables().next().is_none() {
            errors.push(ConfigError::NoTables);
        }
        for (index, table) in self.tables.iter().enumerate() {
            if self.tables[..index]
                .iter()
                .any(|other| other.number == table.number)
            {
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
        if self
            .cancel_reasons
            .iter()
            .any(|text| text.trim().is_empty())
        {
            errors.push(ConfigError::BlankCancelReason);
        }
        if self
            .message_templates
            .iter()
            .any(|text| longer_than(text, LIMITS.text.message))
        {
            errors.push(ConfigError::MessageTemplateTooLong {
                limit: LIMITS.text.message,
            });
        }
        if self
            .cancel_reasons
            .iter()
            .any(|text| longer_than(text, LIMITS.text.reason))
        {
            errors.push(ConfigError::CancelReasonTooLong {
                limit: LIMITS.text.reason,
            });
        }
    }

    fn check_list_lengths(&self, errors: &mut Vec<ConfigError>) {
        let lists = LIMITS.lists;
        for (list, length, limit) in [
            (
                BarList::MessageTemplates,
                self.message_templates.len(),
                lists.message_templates,
            ),
            (
                BarList::CancelReasons,
                self.cancel_reasons.len(),
                lists.cancel_reasons,
            ),
            (BarList::Zones, self.zones.len(), lists.zones),
            (BarList::Staff, self.staff.len(), lists.staff),
            (BarList::Tables, self.active_tables().count(), lists.tables),
        ] {
            if length > limit {
                errors.push(ConfigError::ListTooLong { list, limit });
            }
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

    // Below: arithmetic on hours, turn and room; correct only for validated values.

    #[must_use]
    pub fn total_seats(&self) -> i32 {
        self.active_tables().map(|table| table.seats).sum()
    }

    /// Closing less one turn; `None` on day off. Derived, never stored, so cannot drift.
    #[must_use]
    pub fn last_arrival_minutes(&self, weekday: Weekday) -> Option<i32> {
        let hours = self.week.on(weekday);
        (!hours.closed).then(|| hours.close_minutes - self.turn_minutes)
    }

    /// Shift running, or about to run, at `now`. Not calendar date: at 01:00 bar closing 02:00 still
    /// works yesterday's shift.
    ///
    /// Decided on instants: yesterday runs until [`Self::shift_end`]. Wall repeats hour in autumn;
    /// reading it would restart finished shift.
    #[must_use]
    pub fn current_service_day(&self, now: DateTime<Utc>) -> ServiceDay {
        let today = ServiceDay::new(now.with_timezone(&self.timezone).date_naive());
        match today.checked_sub_days(1) {
            Some(yesterday) if self.shift_end(yesterday).is_some_and(|end| now < end) => yesterday,
            _ => today,
        }
    }

    /// Window for party seated at `now`: one turn, cut at closing. `None` unless `day`
    /// [is open](Self::is_open).
    ///
    /// Closing never later than [`Self::shift_end`], so party leaves before next shift runs.
    #[must_use]
    pub fn walk_in_window(&self, day: ServiceDay, now: DateTime<Utc>) -> Option<Interval> {
        if !self.is_open(day, now) {
            return None;
        }
        let closes = self.closing(day)?;
        let turn = Interval::from_duration(now, self.turn_minutes).ok()?;
        Interval::new(now, turn.end().min(closes)).ok()
    }

    /// Opening of `day` while still ahead of `now`; `None` once opened or on day off.
    #[must_use]
    pub fn opening_ahead(&self, day: ServiceDay, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.opening(day).filter(|opens| now < *opens)
    }

    /// **One rule for "open".** Door seating and guest screen both ask this, on instants. Closing is
    /// last time wall comes up to it, so on autumn night bar closing inside repeated hour stays open
    /// through it.
    #[must_use]
    pub fn is_open(&self, day: ServiceDay, now: DateTime<Utc>) -> bool {
        self.opening(day).is_some_and(|opens| opens <= now)
            && self.closing(day).is_some_and(|closes| now < closes)
    }

    /// Later of closing and last grid sitting end; `None` on day off.
    ///
    /// Spring night: last sitting holds table hour past closing. Autumn night: closing is second
    /// time wall reaches it.
    fn shift_end(&self, day: ServiceDay) -> Option<DateTime<Utc>> {
        let closes = self.closing(day)?;
        Some(last_sitting(self, day).map_or(closes, |sitting| sitting.end().max(closes)))
    }

    /// `None` on day off.
    fn opening(&self, day: ServiceDay) -> Option<DateTime<Utc>> {
        let hours = self.week.for_service_day(day);
        if hours.closed {
            return None;
        }
        resolve_boundary(day, hours.open_minutes, self.timezone).ok()
    }

    /// Last time wall comes up to closing from minute before; `None` on day off.
    fn closing(&self, day: ServiceDay) -> Option<DateTime<Utc>> {
        let hours = self.week.for_service_day(day);
        if hours.closed {
            return None;
        }
        resolve_end(day, hours.close_minutes, self.timezone).ok()
    }

    /// Latest end for booking of `minutes` on `day` still within hours; `None` on day off.
    ///
    /// Later of closing (autumn: maybe second wall reading, so first-pass party holds table longer
    /// than wall says) and end of equal sitting arriving at latest allowed minute (spring: holds
    /// table hour past closing).
    fn latest_end(&self, day: ServiceDay, minutes: i32) -> Option<DateTime<Utc>> {
        let hours = self.week.for_service_day(day);
        let closes = self.closing(day)?;
        let sitting = resolve_boundary(
            day,
            hours.close_minutes.saturating_sub(minutes),
            self.timezone,
        )
        .and_then(|arrives| Interval::from_duration(arrives, minutes))
        .ok();
        Some(sitting.map_or(closes, |sitting| sitting.end().max(closes)))
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarList {
    MessageTemplates,
    CancelReasons,
    Zones,
    Staff,
    Tables,
}

impl std::fmt::Display for BarList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MessageTemplates => "guest messages",
            Self::CancelReasons => "cancellation reasons",
            Self::Zones => "zones",
            Self::Staff => "admins",
            Self::Tables => "tables",
        })
    }
}

/// A reason a configuration is illegal, specific enough to render next to the control at fault.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("the bar needs a name")]
    BlankName,
    #[error("the bar needs an address")]
    BlankAddress,
    #[error("the bar's name is longer than {limit} characters")]
    NameTooLong { limit: usize },
    #[error("the bar's address is longer than {limit} characters")]
    AddressTooLong { limit: usize },
    #[error("a guest message is longer than {limit} characters")]
    MessageTemplateTooLong { limit: usize },
    #[error("a cancellation reason is longer than {limit} characters")]
    CancelReasonTooLong { limit: usize },
    #[error("a zone name is longer than {limit} characters")]
    ZoneNameTooLong { limit: usize },
    #[error("the bar keeps at most {limit} {list}")]
    ListTooLong { list: BarList, limit: usize },
    #[error("the contact is neither a phone number nor a Telegram username")]
    MalformedContact,
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
        shift_minutes: i64,
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

/// Judged on instants against opening and [`ValidConfig::latest_end`]: on wall, both clock changes put
/// bookings the hours seated outside them.
fn conflict_for(config: &ValidConfig, booking: &Booking) -> Option<ScheduleConflict> {
    let day = booking.service_day;
    let hours = config.week.for_service_day(day);
    if hours.closed {
        return Some(ScheduleConflict::DayBecameClosed {
            booking: booking.id,
            service_day: day,
        });
    }
    let minutes = i32::try_from(booking.window.minutes()).unwrap_or(i32::MAX);
    let within = config
        .opening(day)
        .is_some_and(|opens| opens <= booking.window.start())
        && config
            .latest_end(day, minutes)
            .is_some_and(|latest| booking.window.end() <= latest);
    let start_minutes = minutes_within(day, booking.window.start(), config.timezone);
    let end_minutes = start_minutes.saturating_add(minutes);
    if !within {
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

/// How guest reaches a person at bar. Bot chat unread, so party too large for app needs a human.
/// Only phone and Telegram: only those become link phone opens.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Contact {
    Phone { shown: String, dial: String },
    Telegram { username: String },
}

/// Phone as typed, spaces and brackets included: one screen line. Username needs no cap; Telegram
/// rule bounds it.
const PHONE_MAX_CHARS: usize = 32;

impl Contact {
    /// `None` when neither phone nor Telegram username.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let username = text.strip_prefix('@').unwrap_or(text);
        if is_telegram_username(username) {
            return Some(Self::Telegram {
                username: username.to_owned(),
            });
        }
        if text.chars().count() > PHONE_MAX_CHARS {
            return None;
        }
        let digits: String = text.chars().filter(char::is_ascii_digit).collect();
        let shaped = text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ' ' | '-' | '(' | ')' | '+'))
            && text.rfind('+').is_none_or(|at| at == 0);
        // E.164: max 15 digits; under 7 not dialable.
        (shaped && (7..=15).contains(&digits.len())).then(|| Self::Phone {
            shown: text.to_owned(),
            dial: if text.starts_with('+') {
                format!("+{digits}")
            } else {
                digits
            },
        })
    }

    /// Text shown on screen.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Phone { shown, .. } => shown.clone(),
            Self::Telegram { username } => format!("@{username}"),
        }
    }

    /// Link phone opens: `tel:` or `t.me`.
    #[must_use]
    pub fn url(&self) -> String {
        match self {
            Self::Phone { dial, .. } => format!("tel:{dial}"),
            Self::Telegram { username } => format!("https://t.me/{username}"),
        }
    }
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
