//! recurrence — a repeating event, as an RFC 5545 RRULE.
//!
//! # Why RRULE and not our own shape
//!
//! "Every Thursday" is a solved problem with a standard answer: RFC 5545 §3.3.10,
//! the rule iCalendar has used since 1998. Apple Calendar, Google Calendar, Outlook
//! and every .ics file already speak it. Inventing `{freq: "weekly", day: 4}` would
//! mean writing a converter at every boundary — the Calendar import already in the
//! app, an export nobody has asked for yet but will, and the RA/ArtRabbit feeds —
//! and each converter is a place to lose a Thursday.
//!
//! So the delta carries the RRULE string verbatim. What we implement here is the
//! SUBSET that matters for events people actually create, validated strictly:
//!
//! ```text
//! FREQ=DAILY|WEEKLY|MONTHLY|YEARLY   required
//! INTERVAL=<n>                       optional, default 1
//! BYDAY=MO,TU,…                      optional, WEEKLY only
//! COUNT=<n> | UNTIL=<unix ms>        optional, mutually exclusive
//! ```
//!
//! Anything outside the subset is REJECTED at reduce time rather than stored and
//! silently ignored. A rule we cannot expand is worse than no rule: the organiser
//! believes the event repeats, and it does not.
//!
//! # The expansion is a read, never state
//!
//! `occurrences` is a pure function of (rule, start). Occurrences are NOT minted as
//! objects and NOT stored — one Event, one log, one node in the graph, with a rule
//! attached. Materialising them would multiply the object by however far ahead
//! somebody scrolled, and every one of them would need its own delta history.
//!
//! UNTIL is unix ms here rather than RFC 5545's UTC timestamp form. The wire format
//! everywhere else in this codebase is epoch ms (`start_ms`, `at`), and a rule whose
//! dates are in a different unit from the event it governs is a bug waiting to be
//! written. `parse` accepts both forms; `to_rrule` emits ms.

use crate::object::DeltaRejection;

/// How often the event repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Freq {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "DAILY" => Some(Self::Daily),
            "WEEKLY" => Some(Self::Weekly),
            "MONTHLY" => Some(Self::Monthly),
            "YEARLY" => Some(Self::Yearly),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "DAILY",
            Self::Weekly => "WEEKLY",
            Self::Monthly => "MONTHLY",
            Self::Yearly => "YEARLY",
        }
    }
}

/// Where the series stops. `Never` is legitimate — a weekly night with no planned
/// end — and the expander bounds it by the caller's window instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Until {
    Never,
    /// Stop after N occurrences INCLUDING the first.
    Count(u32),
    /// Stop at this instant (unix ms), inclusive.
    Date(i64),
}

/// A parsed, validated repeat rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recurrence {
    pub freq: Freq,
    /// Every `interval` periods. Always >= 1.
    pub interval: u32,
    /// Weekdays, 0=Sunday … 6=Saturday, sorted and deduped. WEEKLY only; empty
    /// means "the weekday the event starts on".
    pub by_day: Vec<u8>,
    pub until: Until,
}

const DAYS: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];

/// A hard ceiling on what one call may expand, so a malicious or fat-fingered rule
/// (`FREQ=DAILY;COUNT=999999999`) cannot hang the caller. Callers ask for a window
/// and a limit; this is the limit's limit.
pub const MAX_OCCURRENCES: usize = 512;

impl Recurrence {
    /// Parse an RRULE. Strict: an unknown part, an unparseable value, or a
    /// combination we cannot expand is an error, never a silently-dropped clause.
    pub fn parse(rule: &str) -> Result<Self, DeltaRejection> {
        let body = rule.trim();
        // Tolerate the "RRULE:" prefix an .ics line carries.
        let body = body.strip_prefix("RRULE:").unwrap_or(body);
        if body.is_empty() {
            return Err(DeltaRejection::MalformedArgs);
        }

        let mut freq = None;
        let mut interval: u32 = 1;
        let mut by_day: Vec<u8> = Vec::new();
        let mut count: Option<u32> = None;
        let mut until_at: Option<i64> = None;

        for part in body.split(';') {
            if part.is_empty() {
                continue;
            }
            let (key, value) = part.split_once('=').ok_or(DeltaRejection::MalformedArgs)?;
            match key.to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(
                        Freq::parse(&value.to_ascii_uppercase())
                            .ok_or(DeltaRejection::MalformedArgs)?,
                    )
                }
                "INTERVAL" => {
                    interval = value.parse().map_err(|_| DeltaRejection::MalformedArgs)?;
                    if interval == 0 {
                        // "every 0 weeks" never advances — an infinite loop, not a rule.
                        return Err(DeltaRejection::MalformedArgs);
                    }
                }
                "BYDAY" => {
                    for d in value.split(',') {
                        let d = d.trim().to_ascii_uppercase();
                        let idx = DAYS
                            .iter()
                            .position(|x| *x == d)
                            .ok_or(DeltaRejection::MalformedArgs)?;
                        by_day.push(idx as u8);
                    }
                }
                "COUNT" => {
                    let c: u32 = value.parse().map_err(|_| DeltaRejection::MalformedArgs)?;
                    if c == 0 {
                        return Err(DeltaRejection::MalformedArgs);
                    }
                    count = Some(c);
                }
                "UNTIL" => {
                    until_at = Some(parse_until(value).ok_or(DeltaRejection::MalformedArgs)?)
                }
                // Everything else in RFC 5545 (BYMONTHDAY, BYSETPOS, WKST, …) is
                // refused rather than ignored — see the module header.
                _ => return Err(DeltaRejection::MalformedArgs),
            }
        }

        let freq = freq.ok_or(DeltaRejection::MalformedArgs)?;
        if count.is_some() && until_at.is_some() {
            // RFC 5545: "UNTIL and COUNT MUST NOT occur in the same recur."
            return Err(DeltaRejection::MalformedArgs);
        }
        if !by_day.is_empty() && freq != Freq::Weekly {
            // We only expand BYDAY weekly; accepting it on MONTHLY would store a
            // rule whose expansion silently ignores half of what it says.
            return Err(DeltaRejection::MalformedArgs);
        }
        by_day.sort_unstable();
        by_day.dedup();

        Ok(Self {
            freq,
            interval,
            by_day,
            until: match (count, until_at) {
                (Some(c), _) => Until::Count(c),
                (_, Some(d)) => Until::Date(d),
                _ => Until::Never,
            },
        })
    }

    /// Canonical RRULE text — what the delta stores, so a round-trip is stable.
    pub fn to_rrule(&self) -> String {
        let mut out = format!("FREQ={}", self.freq.as_str());
        if self.interval != 1 {
            out.push_str(&format!(";INTERVAL={}", self.interval));
        }
        if !self.by_day.is_empty() {
            let days: Vec<&str> = self.by_day.iter().map(|d| DAYS[*d as usize]).collect();
            out.push_str(&format!(";BYDAY={}", days.join(",")));
        }
        match self.until {
            Until::Never => {}
            Until::Count(c) => out.push_str(&format!(";COUNT={c}")),
            Until::Date(d) => out.push_str(&format!(";UNTIL={d}")),
        }
        out
    }

    /// A short human line for the UI — "Every Thursday", "Every 2 weeks".
    /// Lives here so the label cannot drift from the rule that produced it.
    pub fn summary(&self) -> String {
        let every = if self.interval == 1 {
            String::new()
        } else {
            format!("{} ", self.interval)
        };
        let unit: String = match (self.freq, self.interval) {
            (Freq::Daily, 1) => "day".into(),
            (Freq::Daily, _) => "days".into(),
            (Freq::Weekly, 1) => {
                if self.by_day.len() == 1 {
                    return format!("Every {}", weekday_name(self.by_day[0]));
                }
                "week".into()
            }
            (Freq::Weekly, _) => "weeks".into(),
            (Freq::Monthly, 1) => "month".into(),
            (Freq::Monthly, _) => "months".into(),
            (Freq::Yearly, 1) => "year".into(),
            (Freq::Yearly, _) => "years".into(),
        };
        format!("Every {every}{unit}")
    }

    /// The series' start instants (unix ms), from `start_ms`, in order.
    ///
    /// `from`/`to` bound the WINDOW the caller cares about; `limit` bounds the work.
    /// The first occurrence is always `start_ms` itself (RFC 5545: DTSTART is the
    /// first instance), included when the window admits it.
    ///
    /// Pure: no clock, no allocation beyond the result, and the same answer on every
    /// replica — which is what lets this be a read rather than stored state.
    pub fn occurrences(&self, start_ms: i64, from: i64, to: i64, limit: usize) -> Vec<i64> {
        let limit = limit.min(MAX_OCCURRENCES);
        let mut out = Vec::new();
        if limit == 0 || to < from {
            return out;
        }

        // WEEKLY with BYDAY steps through the named weekdays; everything else steps
        // one period at a time from the start.
        let mut emitted: u32 = 0;
        let mut cursor = start_ms;
        // A generous walk bound: we may step over many instants outside the window
        // before reaching it, but never more than this many, so the loop terminates
        // even for a rule whose window is far in the future.
        let mut steps = 0usize;
        let max_steps = MAX_OCCURRENCES * 8;

        while steps < max_steps && out.len() < limit {
            steps += 1;

            if let Until::Count(c) = self.until {
                if emitted >= c {
                    break;
                }
            }
            if let Until::Date(d) = self.until {
                if cursor > d {
                    break;
                }
            }
            if cursor > to {
                break;
            }
            if cursor >= from {
                out.push(cursor);
            }
            emitted += 1;

            cursor = match self.next(cursor, start_ms) {
                Some(n) => n,
                None => break,
            };
        }
        out
    }

    /// The instant after `cursor`. Separated so the stepping rules are readable and
    /// the loop above stays about bounds.
    fn next(&self, cursor: i64, start_ms: i64) -> Option<i64> {
        const DAY: i64 = 86_400_000;
        match self.freq {
            Freq::Daily => cursor.checked_add(DAY * self.interval as i64),
            Freq::Weekly if self.by_day.is_empty() => {
                cursor.checked_add(DAY * 7 * self.interval as i64)
            }
            Freq::Weekly => {
                // Walk day by day to the next named weekday. When that wraps past the
                // start-of-week we also advance by (interval - 1) whole weeks, so
                // "every other Tuesday and Thursday" keeps its fortnightly rhythm.
                let mut c = cursor.checked_add(DAY)?;
                for _ in 0..7 {
                    if self.by_day.contains(&weekday(c)) {
                        if self.interval > 1 && weekday(c) <= weekday(start_ms) {
                            c = c.checked_add(DAY * 7 * (self.interval as i64 - 1))?;
                        }
                        return Some(c);
                    }
                    c = c.checked_add(DAY)?;
                }
                None
            }
            // Calendar months and years, not fixed spans: "the 3rd of every month"
            // must not drift by three days a quarter, which is what adding 30 days
            // would do.
            Freq::Monthly => add_months(cursor, self.interval as i32),
            Freq::Yearly => add_months(cursor, 12 * self.interval as i32),
        }
    }
}

/// Weekday of a unix-ms instant, 0=Sunday. 1 Jan 1970 was a Thursday (4).
fn weekday(ms: i64) -> u8 {
    let days = ms.div_euclid(86_400_000);
    (((days + 4) % 7 + 7) % 7) as u8
}

fn weekday_name(d: u8) -> &'static str {
    [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ][d as usize % 7]
}

/// Add whole calendar months, clamping the day into the target month — 31 Jan + 1
/// month is 28 Feb, not 3 March. Civil-date arithmetic on the proleptic Gregorian
/// calendar (Howard Hinnant's days-from-civil), so it needs no date library.
fn add_months(ms: i64, months: i32) -> Option<i64> {
    const DAY: i64 = 86_400_000;
    let days = ms.div_euclid(DAY);
    let time_of_day = ms - days * DAY;
    let (y, m, d) = civil_from_days(days);

    let total = y * 12 + (m as i32 - 1) + months;
    let ny = total.div_euclid(12);
    let nm = (total.rem_euclid(12) + 1) as u32;
    let nd = d.min(days_in_month(ny, nm));

    Some(days_from_civil(ny, nm, nd).checked_mul(DAY)? + time_of_day)
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// UNIX ms, or RFC 5545's `YYYYMMDD` / `YYYYMMDDTHHMMSSZ` — accepted so a rule
/// pasted from an .ics file parses, and re-emitted as ms by `to_rrule`.
fn parse_until(v: &str) -> Option<i64> {
    let v = v.trim();
    if let Ok(ms) = v.parse::<i64>() {
        // A bare 8-digit number is ambiguous: 20260813 is both a plausible ms value
        // (Jan 1970) and an obvious calendar date. Treat 8 digits as the date form.
        if v.len() != 8 {
            return Some(ms);
        }
    }
    let date = v.split('T').next()?;
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i32 = date[0..4].parse().ok()?;
    let m: u32 = date[4..6].parse().ok()?;
    let d: u32 = date[6..8].parse().ok()?;
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(days_from_civil(y, m, d) * 86_400_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-07-30 is a Thursday, 12:00 UTC.
    const THU: i64 = 1_785_412_800_000;
    const DAY: i64 = 86_400_000;
    const WEEK: i64 = DAY * 7;

    fn r(s: &str) -> Recurrence {
        Recurrence::parse(s).expect("valid rule")
    }

    #[test]
    fn weekday_helper_is_right() {
        assert_eq!(weekday(THU), 4, "2026-07-30 is a Thursday");
        assert_eq!(weekday(0), 4, "1 Jan 1970 was a Thursday");
    }

    #[test]
    fn weekly_repeats_on_the_start_weekday() {
        let occ = r("FREQ=WEEKLY").occurrences(THU, THU, THU + WEEK * 3, 10);
        assert_eq!(occ, vec![THU, THU + WEEK, THU + WEEK * 2, THU + WEEK * 3]);
    }

    #[test]
    fn the_first_occurrence_is_the_start_itself() {
        let occ = r("FREQ=DAILY").occurrences(THU, 0, THU + DAY, 10);
        assert_eq!(occ.first(), Some(&THU), "DTSTART is the first instance");
    }

    #[test]
    fn count_bounds_the_series() {
        let occ = r("FREQ=WEEKLY;COUNT=3").occurrences(THU, 0, THU + WEEK * 50, 100);
        assert_eq!(occ.len(), 3, "COUNT includes the first instance");
        assert_eq!(occ.last(), Some(&(THU + WEEK * 2)));
    }

    #[test]
    fn until_bounds_the_series() {
        let stop = THU + WEEK * 2;
        let occ = r(&format!("FREQ=WEEKLY;UNTIL={stop}")).occurrences(THU, 0, THU + WEEK * 50, 100);
        assert_eq!(occ, vec![THU, THU + WEEK, stop], "UNTIL is inclusive");
    }

    #[test]
    fn interval_skips_periods() {
        let occ = r("FREQ=WEEKLY;INTERVAL=2").occurrences(THU, THU, THU + WEEK * 4, 10);
        assert_eq!(occ, vec![THU, THU + WEEK * 2, THU + WEEK * 4]);
    }

    #[test]
    fn byday_hits_each_named_weekday() {
        // Thursdays and Saturdays, from a Thursday.
        let occ = r("FREQ=WEEKLY;BYDAY=TH,SA").occurrences(THU, THU, THU + WEEK, 10);
        assert_eq!(occ, vec![THU, THU + DAY * 2, THU + WEEK]);
    }

    /// Calendar months, not 30-day spans — otherwise a monthly event drifts.
    #[test]
    fn monthly_keeps_the_day_of_month() {
        let occ = r("FREQ=MONTHLY;COUNT=3").occurrences(THU, 0, i64::MAX / 2, 10);
        let dates: Vec<(i32, u32, u32)> = occ
            .iter()
            .map(|ms| civil_from_days(ms.div_euclid(DAY)))
            .collect();
        assert_eq!(dates, vec![(2026, 7, 30), (2026, 8, 30), (2026, 9, 30)]);
    }

    /// The 31st of a month that has no 31st clamps rather than spilling forward.
    #[test]
    fn monthly_clamps_a_short_month() {
        let jan31 = days_from_civil(2026, 1, 31) * DAY;
        let occ = r("FREQ=MONTHLY;COUNT=2").occurrences(jan31, 0, i64::MAX / 2, 10);
        let d = civil_from_days(occ[1].div_euclid(DAY));
        assert_eq!(d, (2026, 2, 28), "31 Jan + 1 month is 28 Feb, not 3 Mar");
    }

    #[test]
    fn yearly_lands_on_the_same_date() {
        let occ = r("FREQ=YEARLY;COUNT=2").occurrences(THU, 0, i64::MAX / 2, 10);
        assert_eq!(civil_from_days(occ[1].div_euclid(DAY)), (2027, 7, 30));
    }

    #[test]
    fn the_window_filters_without_losing_the_rhythm() {
        // Ask only for weeks 2–3; the series still lands on the right instants.
        let occ = r("FREQ=WEEKLY").occurrences(THU, THU + WEEK * 2, THU + WEEK * 3, 10);
        assert_eq!(occ, vec![THU + WEEK * 2, THU + WEEK * 3]);
    }

    #[test]
    fn an_unbounded_rule_is_capped_by_the_caller() {
        let occ = r("FREQ=DAILY").occurrences(THU, THU, i64::MAX / 2, 5);
        assert_eq!(occ.len(), 5, "limit bounds an endless rule");
        let occ = r("FREQ=DAILY").occurrences(THU, THU, i64::MAX / 2, 100_000);
        assert!(
            occ.len() <= MAX_OCCURRENCES,
            "and MAX_OCCURRENCES bounds the limit"
        );
    }

    #[test]
    fn round_trips_through_canonical_text() {
        for rule in [
            "FREQ=DAILY",
            "FREQ=WEEKLY;INTERVAL=2",
            "FREQ=WEEKLY;BYDAY=MO,WE,FR",
            "FREQ=MONTHLY;COUNT=6",
            "FREQ=YEARLY",
        ] {
            assert_eq!(r(rule).to_rrule(), rule, "{rule} must round-trip");
        }
    }

    #[test]
    fn an_ics_prefix_and_date_until_parse() {
        let p = r("RRULE:FREQ=WEEKLY;UNTIL=20261231T000000Z");
        assert_eq!(p.freq, Freq::Weekly);
        assert_eq!(p.until, Until::Date(days_from_civil(2026, 12, 31) * DAY));
    }

    /// A rule we cannot expand must be REFUSED, not stored and half-honoured.
    #[test]
    fn unexpandable_rules_are_refused_rather_than_ignored() {
        for bad in [
            "",                             // empty
            "INTERVAL=2",                   // no FREQ
            "FREQ=HOURLY",                  // outside the subset
            "FREQ=WEEKLY;INTERVAL=0",       // never advances
            "FREQ=WEEKLY;COUNT=0",          // a series with no occurrences
            "FREQ=WEEKLY;BYMONTHDAY=3",     // unsupported part, silently lossy
            "FREQ=MONTHLY;BYDAY=MO",        // BYDAY only expands weekly
            "FREQ=WEEKLY;COUNT=3;UNTIL=99", // RFC 5545 forbids both
            "FREQ=WEEKLY;BYDAY=XX",         // not a weekday
        ] {
            assert!(
                Recurrence::parse(bad).is_err(),
                "`{bad}` must be refused, not stored"
            );
        }
    }

    #[test]
    fn summary_reads_like_a_person_wrote_it() {
        assert_eq!(r("FREQ=WEEKLY;BYDAY=TH").summary(), "Every Thursday");
        assert_eq!(r("FREQ=WEEKLY;INTERVAL=2").summary(), "Every 2 weeks");
        assert_eq!(r("FREQ=DAILY").summary(), "Every day");
        assert_eq!(r("FREQ=MONTHLY").summary(), "Every month");
    }
}
