//! Business time versus absolute time.
//!
//! A bar does not think in calendar days. It thinks in *shifts*: "Friday" is one continuous
//! service that opens at 18:00 and closes at 02:00, and a booking taken at 01:00 belongs to
//! Friday even though the calendar has already turned over. Two facts are therefore kept
//! about every booking and never derived from one another:
//!
//! * the [`ServiceDay`] — which shift it belongs to, a label fixed when the booking is taken;
//! * the absolute window it occupies — the only thing overlap may ever be computed on.
//!
//! Opening hours are wall-clock minutes counted from midnight at the start of the service day,
//! so a close at `1560` means 02:00 the next calendar morning. Everything that has to compare
//! against opening hours works in those minutes; everything that has to decide whether two
//! parties would sit at the same table at the same moment works in absolute instants. Mixing
//! the two is how a table gets sold twice on the night the clocks change.

use chrono::{DateTime, Datelike, Days, Duration, LocalResult, NaiveDate, TimeZone, Utc, Weekday};
use chrono_tz::Tz;

/// Highest minute offset a service day may address: 04:00 on the following calendar day.
///
/// This is a representability bound, not a policy: the business limits on opening and closing
/// times live in [`crate::config::Limits`].
pub const MAX_SERVICE_MINUTE: i32 = 28 * 60;

/// The shift a booking belongs to, identified by the calendar date the shift *opens* on.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ServiceDay(NaiveDate);

impl ServiceDay {
    #[must_use]
    pub const fn new(date: NaiveDate) -> Self {
        Self(date)
    }

    #[must_use]
    pub const fn date(self) -> NaiveDate {
        self.0
    }

    /// Weekday the shift opens on — the index into a [`crate::config::WeekSchedule`].
    #[must_use]
    pub fn weekday(self) -> Weekday {
        self.0.weekday()
    }

    /// The shift `days` later. Returns `None` only at the edges of the calendar range.
    #[must_use]
    pub fn checked_add_days(self, days: u64) -> Option<Self> {
        self.0.checked_add_days(Days::new(days)).map(Self)
    }

    /// The shift `days` earlier. Returns `None` only at the edges of the calendar range.
    #[must_use]
    pub fn checked_sub_days(self, days: u64) -> Option<Self> {
        self.0.checked_sub_days(Days::new(days)).map(Self)
    }
}

/// A half-open absolute interval `[start, end)`.
///
/// Half-open is not a detail: back-to-back bookings share an endpoint, and a closed interval
/// would report them as overlapping. It matches the `tstzrange(..., '[)')` the database stores
/// and the `&&` operator its exclusion constraint uses, so the same two bookings can never be
/// judged differently by the allocator and by the constraint that backs it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Interval {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

impl Interval {
    /// Builds an interval, rejecting the empty and inverted cases outright so that no code
    /// downstream has to wonder whether an interval can be degenerate.
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Self, TimeError> {
        if end <= start {
            return Err(TimeError::EmptyInterval { start, end });
        }
        Ok(Self { start, end })
    }

    /// Interval starting at `start` and lasting `minutes`.
    pub fn from_duration(start: DateTime<Utc>, minutes: i32) -> Result<Self, TimeError> {
        if minutes <= 0 {
            return Err(TimeError::NonPositiveDuration(minutes));
        }
        let end = start
            .checked_add_signed(Duration::minutes(i64::from(minutes)))
            .ok_or(TimeError::MinutesOutOfRange(minutes))?;
        Self::new(start, end)
    }

    #[must_use]
    pub const fn start(self) -> DateTime<Utc> {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> DateTime<Utc> {
        self.end
    }

    #[must_use]
    pub fn minutes(self) -> i64 {
        (self.end - self.start).num_minutes()
    }

    /// `true` when the two intervals share at least one instant.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Failures of the business-time to absolute-time mapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum TimeError {
    /// The local wall-clock time asked for never happens: the clocks jump over it.
    #[error("{minutes} minutes into the {day} shift is a local time that does not exist in {tz}")]
    LocalTimeSkipped {
        day: NaiveDate,
        minutes: i32,
        tz: Tz,
    },
    /// The minute offset is outside what a service day can address at all.
    #[error("minute offset {0} is outside the representable service day")]
    MinutesOutOfRange(i32),
    /// An interval was asked to be empty or inverted.
    #[error("interval end {end} is not after its start {start}")]
    EmptyInterval {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },
    /// A duration was asked to be zero or negative.
    #[error("duration of {0} minutes is not positive")]
    NonPositiveDuration(i32),
}

/// Resolves a wall-clock minute offset within a shift to the instant it happens.
///
/// When the clocks go back, the requested wall-clock time happens twice; the *first*
/// occurrence is chosen, because that is the one a guest reading a clock is referring to and
/// because a consistent choice is what keeps [`minutes_within`] a left inverse of this
/// function. When the clocks go forward the requested time never happens at all, and that is
/// an error rather than a silent shift — a bar cannot seat anyone at a time the day does not
/// contain, and quietly moving the guest an hour is worse than refusing the slot.
pub fn resolve(day: ServiceDay, minutes: i32, tz: Tz) -> Result<DateTime<Utc>, TimeError> {
    if !(0..=MAX_SERVICE_MINUTE).contains(&minutes) {
        return Err(TimeError::MinutesOutOfRange(minutes));
    }
    let naive = day
        .date()
        .and_hms_opt(0, 0, 0)
        .ok_or(TimeError::MinutesOutOfRange(minutes))?
        .checked_add_signed(Duration::minutes(i64::from(minutes)))
        .ok_or(TimeError::MinutesOutOfRange(minutes))?;
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Ok(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(earlier, _later) => Ok(earlier.with_timezone(&Utc)),
        LocalResult::None => Err(TimeError::LocalTimeSkipped {
            day: day.date(),
            minutes,
            tz,
        }),
    }
}

/// Wall-clock minutes from the start of `day` to `instant`, the inverse of [`resolve`].
///
/// The result may exceed 1440 for a shift that runs past midnight, and is negative for an
/// instant before the shift's calendar date began. Comparisons against opening and closing
/// times must go through this function, never through absolute arithmetic: opening hours are
/// wall-clock facts and survive a clock change unchanged.
#[must_use]
pub fn minutes_within(day: ServiceDay, instant: DateTime<Utc>, tz: Tz) -> i32 {
    let local = instant.with_timezone(&tz).naive_local();
    let midnight = day
        .date()
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time on every date");
    i32::try_from((local - midnight).num_minutes()).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BELGRADE: Tz = chrono_tz::Europe::Belgrade;

    fn day(y: i32, m: u32, d: u32) -> ServiceDay {
        ServiceDay::new(NaiveDate::from_ymd_opt(y, m, d).expect("valid date"))
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("valid utc instant")
    }

    #[test]
    fn resolves_an_ordinary_evening_slot() {
        // 20:00 on a July evening in Belgrade is CEST, two hours ahead of UTC.
        assert_eq!(
            resolve(day(2026, 7, 30), 20 * 60, BELGRADE),
            Ok(utc(2026, 7, 30, 18, 0))
        );
    }

    #[test]
    fn resolves_a_minute_offset_past_midnight_into_the_next_calendar_day() {
        // A shift closing at "26:00" closes at 02:00 the following morning.
        assert_eq!(
            resolve(day(2026, 7, 30), 26 * 60, BELGRADE),
            Ok(utc(2026, 7, 31, 0, 0))
        );
    }

    #[test]
    fn refuses_a_local_time_the_spring_clock_change_skips() {
        // Belgrade jumps 02:00 -> 03:00 on 2026-03-29, so 02:30 never happens.
        assert_eq!(
            resolve(day(2026, 3, 29), 2 * 60 + 30, BELGRADE),
            Err(TimeError::LocalTimeSkipped {
                day: NaiveDate::from_ymd_opt(2026, 3, 29).unwrap(),
                minutes: 150,
                tz: BELGRADE,
            })
        );
    }

    #[test]
    fn picks_the_first_occurrence_of_an_ambiguous_autumn_local_time() {
        // Belgrade repeats 02:00..03:00 on 2026-10-25: 02:30 happens at 00:30Z and again at
        // 01:30Z. The earlier one is chosen.
        assert_eq!(
            resolve(day(2026, 10, 25), 2 * 60 + 30, BELGRADE),
            Ok(utc(2026, 10, 25, 0, 30))
        );
    }

    #[test]
    fn rejects_minute_offsets_outside_a_service_day() {
        assert_eq!(
            resolve(day(2026, 7, 30), -1, BELGRADE),
            Err(TimeError::MinutesOutOfRange(-1))
        );
        assert_eq!(
            resolve(day(2026, 7, 30), MAX_SERVICE_MINUTE + 1, BELGRADE),
            Err(TimeError::MinutesOutOfRange(MAX_SERVICE_MINUTE + 1))
        );
    }

    #[test]
    fn minutes_within_inverts_resolve_for_ordinary_times() {
        for minutes in [0, 600, 1200, 1439, 1440, 1560] {
            let instant = resolve(day(2026, 7, 30), minutes, BELGRADE).expect("resolvable");
            assert_eq!(
                minutes_within(day(2026, 7, 30), instant, BELGRADE),
                minutes,
                "round trip failed for {minutes}"
            );
        }
    }

    #[test]
    fn minutes_within_reports_wall_clock_minutes_across_an_autumn_clock_change() {
        // 02:30 CEST, the first pass through the repeated hour.
        assert_eq!(
            minutes_within(day(2026, 10, 25), utc(2026, 10, 25, 0, 30), BELGRADE),
            150
        );
        // 02:30 CET, the second pass: the same wall clock, one hour of real time later.
        assert_eq!(
            minutes_within(day(2026, 10, 25), utc(2026, 10, 25, 1, 30), BELGRADE),
            150
        );
    }

    #[test]
    fn a_shift_spanning_the_spring_change_loses_an_hour_of_real_time() {
        // 01:00 local exists, 03:00 local exists, and only one real hour separates them.
        let one = resolve(day(2026, 3, 29), 60, BELGRADE).expect("resolvable");
        let three = resolve(day(2026, 3, 29), 180, BELGRADE).expect("resolvable");
        assert_eq!((three - one).num_minutes(), 60);
        // Wall-clock minutes still report the two-hour gap the opening hours are written in.
        assert_eq!(minutes_within(day(2026, 3, 29), three, BELGRADE), 180);
    }

    #[test]
    fn intervals_are_half_open_so_back_to_back_bookings_do_not_overlap() {
        let first = Interval::new(utc(2026, 7, 30, 18, 0), utc(2026, 7, 30, 20, 0)).unwrap();
        let second = Interval::new(utc(2026, 7, 30, 20, 0), utc(2026, 7, 30, 22, 0)).unwrap();
        assert!(!first.overlaps(second));
        assert!(!second.overlaps(first));
    }

    #[test]
    fn intervals_sharing_any_instant_overlap_in_both_directions() {
        let first = Interval::new(utc(2026, 7, 30, 18, 0), utc(2026, 7, 30, 20, 0)).unwrap();
        let straddling =
            Interval::new(utc(2026, 7, 30, 19, 59), utc(2026, 7, 30, 21, 0)).unwrap();
        assert!(first.overlaps(straddling));
        assert!(straddling.overlaps(first));
    }

    #[test]
    fn an_interval_cannot_be_empty_or_inverted() {
        let noon = utc(2026, 7, 30, 12, 0);
        assert!(matches!(
            Interval::new(noon, noon),
            Err(TimeError::EmptyInterval { .. })
        ));
        assert!(matches!(
            Interval::new(noon, utc(2026, 7, 30, 11, 0)),
            Err(TimeError::EmptyInterval { .. })
        ));
        assert!(matches!(
            Interval::from_duration(noon, 0),
            Err(TimeError::NonPositiveDuration(0))
        ));
    }

    #[test]
    fn an_interval_built_across_a_clock_change_keeps_its_real_duration() {
        // A two-hour booking arriving at 01:30 on the spring-forward night still occupies two
        // real hours; it simply ends at 04:30 on the wall clock rather than 03:30.
        let start = resolve(day(2026, 3, 29), 90, BELGRADE).expect("01:30 exists");
        let window = Interval::from_duration(start, 120).expect("positive duration");
        assert_eq!(window.minutes(), 120);
        assert_eq!(minutes_within(day(2026, 3, 29), window.end(), BELGRADE), 270);
    }

    #[test]
    fn service_days_step_by_whole_days() {
        let thursday = day(2026, 7, 30);
        assert_eq!(thursday.weekday(), Weekday::Thu);
        assert_eq!(thursday.checked_add_days(2), Some(day(2026, 8, 1)));
        assert_eq!(thursday.checked_sub_days(30), Some(day(2026, 6, 30)));
    }
}
