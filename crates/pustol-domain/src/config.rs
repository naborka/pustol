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

/// The longest each text the bar writes may be, in characters.
///
/// A guest message longer than Telegram carries is refused on every send; a name, reason or zone
/// that long breaks every screen and message it is drawn into.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct TextLimits {
    pub name: usize,
    pub address: usize,
    pub message: usize,
    pub reason: usize,
    pub zone: usize,
}

/// The most entries each list the bar keeps may hold.
///
/// Every list travels whole in one settings save. Unbounded, a legal configuration could outgrow any
/// request the API reads, and could then never be saved again.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct ListLimits {
    pub message_templates: usize,
    pub cancel_reasons: usize,
    pub zones: usize,
    pub staff: usize,
    /// Tables in the live room. Retired tables are the room's history, kept for ever, and never
    /// count: a bar that replaced its furniture often enough would otherwise be locked out.
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
    /// Length of the shift in minutes.
    ///
    /// Wide, because these are a proposal's hours until validated, and the difference of any two
    /// integers a body can carry does not fit in the width they arrived in.
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
    /// How guests reach a person at the bar — a phone number or a Telegram username — when the bar
    /// has given one. Kept as written; [`Contact::parse`] says what it is.
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

    /// The contact guests are given, or `None` when the bar gives none or what it gives is neither
    /// kind [`Contact::parse`] reads.
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

    // Everything below does arithmetic on the hours, the turn and the room, and so lives here rather
    // than on the proposal: it is correct only for values validation has bounded.

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
    ///
    /// Decided on instants, never on the wall clock: yesterday runs until [`Self::shift_end`]. The
    /// wall clock repeats an hour in autumn, and reading it made a shift that had stopped start
    /// running again, an hour after every table was free.
    #[must_use]
    pub fn current_service_day(&self, now: DateTime<Utc>) -> ServiceDay {
        let today = ServiceDay::new(now.with_timezone(&self.timezone).date_naive());
        match today.checked_sub_days(1) {
            Some(yesterday) if self.shift_end(yesterday).is_some_and(|end| now < end) => yesterday,
            _ => today,
        }
    }

    /// The window a party sitting down at `now` holds on `day`, or `None` when `day` seats nobody
    /// now: a day off, a shift that has not opened, or one that has closed.
    ///
    /// One turn, cut short at closing, so a party never sits past the hours that seated it, on the wall
    /// or in real time. Closing is never later than [`Self::shift_end`], so the party is gone before
    /// the next shift runs.
    ///
    /// Seated only while `day` [is open](Self::is_open).
    #[must_use]
    pub fn walk_in_window(&self, day: ServiceDay, now: DateTime<Utc>) -> Option<Interval> {
        if !self.is_open(day, now) {
            return None;
        }
        let closes = self.closing(day)?;
        let turn = Interval::from_duration(now, self.turn_minutes).ok()?;
        Interval::new(now, turn.end().min(closes)).ok()
    }

    /// The moment `day` opens, while `now` is before it; `None` once it has opened, and on a day off.
    #[must_use]
    pub fn opening_ahead(&self, day: ServiceDay, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.opening(day).filter(|opens| now < *opens)
    }

    /// Whether `day` is open at `now`: it has opened, and closing has not come.
    ///
    /// **The one rule for "open".** Whether a party at the door is seated and whether the guest's screen
    /// says the bar is open are this question, asked of instants rather than of the wall. Closing is
    /// the last moment the wall comes up to it from the minute before, so on the night the clocks go
    /// back a bar closing inside the repeated hour stays open through it.
    #[must_use]
    pub fn is_open(&self, day: ServiceDay, now: DateTime<Utc>) -> bool {
        self.opening(day).is_some_and(|opens| opens <= now)
            && self.closing(day).is_some_and(|closes| now < closes)
    }

    /// The moment `day` stops running, or `None` on a day off.
    ///
    /// The later of closing and the end of the last sitting the grid has. Every sitting ends by closing
    /// on the wall, but not always in real time: on the night the clocks go forward the last one holds
    /// its table an hour past closing, and the shift runs until it is over. On the night they go back
    /// the wall can come up to closing twice, and the shift runs until the second.
    fn shift_end(&self, day: ServiceDay) -> Option<DateTime<Utc>> {
        let closes = self.closing(day)?;
        Some(last_sitting(self, day).map_or(closes, |sitting| sitting.end().max(closes)))
    }

    /// The moment `day` opens, or `None` on a day off.
    fn opening(&self, day: ServiceDay) -> Option<DateTime<Utc>> {
        let hours = self.week.for_service_day(day);
        if hours.closed {
            return None;
        }
        resolve_boundary(day, hours.open_minutes, self.timezone).ok()
    }

    /// The moment `day` closes, the last the wall comes up to its closing time from the minute before,
    /// or `None` on a day off.
    fn closing(&self, day: ServiceDay) -> Option<DateTime<Utc>> {
        let hours = self.week.for_service_day(day);
        if hours.closed {
            return None;
        }
        resolve_end(day, hours.close_minutes, self.timezone).ok()
    }

    /// The latest a booking lasting `minutes` may end on `day` and still sit within its hours, or
    /// `None` on a day off.
    ///
    /// The later of two moments. One is closing, which can be the second time the wall reaches it: a
    /// party seated on the first pass through the hour the clocks repeat holds its table longer than
    /// the wall says, and sits within the hours all the same. The other is the end of a sitting as long, arriving at
    /// the latest minute closing allows it: on the night the clocks go forward the grid's last arrival
    /// holds its table an hour past closing, under the very hours that sold it.
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

/// A list the bar keeps, named so an error about its length can say which one.
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

/// Judged on instants, against the opening and the latest end [`ValidConfig::latest_end`] allows: on
/// the wall the two clock changes each put a booking the hours themselves seated outside them.
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

/// A way for a guest to reach a person at the bar.
///
/// The bot's chat is read by nobody, so a party larger than the app takes needs somewhere a human
/// answers. Only two kinds are accepted, because only those two turn into a link a phone can open.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Contact {
    Phone { shown: String, dial: String },
    Telegram { username: String },
}

/// The longest phone number as written, spaces and brackets included: one line of a screen. A
/// username needs no cap of its own, because Telegram's rule already bounds it.
const PHONE_MAX_CHARS: usize = 32;

impl Contact {
    /// Reads a contact as a manager typed it, or `None` when it is neither kind.
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
        // E.164 allows at most fifteen digits; fewer than seven is not a number anyone can dial.
        (shaped && (7..=15).contains(&digits.len())).then(|| Self::Phone {
            shown: text.to_owned(),
            dial: if text.starts_with('+') {
                format!("+{digits}")
            } else {
                digits
            },
        })
    }

    /// How the contact is written on a screen.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Phone { shown, .. } => shown.clone(),
            Self::Telegram { username } => format!("@{username}"),
        }
    }

    /// What a phone opens to reach it.
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
