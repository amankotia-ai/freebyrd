//! Days as the person lives them. A working day runs from 04:00 to 04:00
//! local time, so work past midnight counts toward the day it belongs to.

use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeDelta, TimeZone, Weekday};
use regex::Regex;

pub const DAY_START_HOUR: u32 = 4;

/// A span of time in milliseconds since the Unix epoch, end excluded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// The working day that `now` falls in.
pub fn working_day(now: DateTime<Local>) -> NaiveDate {
    (now - TimeDelta::hours(i64::from(DAY_START_HOUR))).date_naive()
}

pub fn day_span(day: NaiveDate) -> Span {
    Span {
        start_ms: day_start_ms(day),
        end_ms: day_start_ms(day.succ_opt().unwrap_or(day)),
    }
}

fn day_start_ms(day: NaiveDate) -> i64 {
    let start = day.and_time(NaiveTime::from_hms_opt(DAY_START_HOUR, 0, 0).unwrap_or_default());
    // A start skipped by a clock change resolves to the earliest valid time.
    Local.from_local_datetime(&start).earliest().map_or_else(
        || start.and_utc().timestamp_millis(),
        |t| t.timestamp_millis(),
    )
}

/// "Monday 12 October 2026".
pub fn day_label(day: NaiveDate) -> String {
    day.format("%A %-d %B %Y").to_string()
}

/// "Mon 12 Oct".
pub fn short_day_label(day: NaiveDate) -> String {
    day.format("%a %-d %b").to_string()
}

/// "14:05", local time.
pub fn clock(ms: i64) -> String {
    local(ms).map_or_else(String::new, |t| t.format("%H:%M").to_string())
}

/// "Mon 12 Oct 2026 14:05", local time.
pub fn moment_with_year(ms: i64) -> String {
    local(ms).map_or_else(String::new, |t| t.format("%a %-d %b %Y %H:%M").to_string())
}

/// "Mon 12 Oct 14:05", local time.
pub fn moment(ms: i64) -> String {
    local(ms).map_or_else(String::new, |t| t.format("%a %-d %b %H:%M").to_string())
}

/// The days a question names, as of `today`: "Sep 7", "7th September",
/// "2026-09-07", "today", "yesterday", "Monday" (the latest one, today
/// included). A date without a year is the latest one not after today. At
/// most three, in the order named.
pub fn named_days(text: &str, today: NaiveDate) -> Vec<NaiveDate> {
    static DAYS: LazyLock<Regex> = LazyLock::new(|| {
        let month = r"(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?";
        let day = r"(\d{1,2})(?:st|nd|rd|th)?";
        Regex::new(&format!(
            r"(?i)\b(?:(\d{{4}})-(\d{{2}})-(\d{{2}})|{month}\s+{day}(?:,?\s+(\d{{4}}))?|{day}(?:\s+of)?\s+{month}(?:,?\s+(\d{{4}}))?|(today|yesterday)|(mon|tues|wednes|thurs|fri|satur|sun)day)\b"
        ))
        .expect("valid pattern")
    });
    let mut days = Vec::new();
    for found in DAYS.captures_iter(text) {
        let number = |i: usize| found.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
        let day = if let (Some(y), Some(m), Some(d)) = (number(1), number(2), number(3)) {
            NaiveDate::from_ymd_opt(i32::try_from(y).unwrap_or_default(), m, d)
        } else if let (Some(m), Some(d)) = (found.get(4).and_then(|m| month(m.as_str())), number(5))
        {
            month_day(m, d, number(6), today)
        } else if let (Some(d), Some(m)) = (number(7), found.get(8).and_then(|m| month(m.as_str())))
        {
            month_day(m, d, number(9), today)
        } else if let Some(word) = found.get(10) {
            match word.as_str().to_lowercase().as_str() {
                "today" => Some(today),
                _ => today.pred_opt(),
            }
        } else {
            found
                .get(11)
                .and_then(|w| format!("{}day", w.as_str()).parse::<Weekday>().ok())
                .map(|weekday| {
                    let back = (7 + today.weekday().num_days_from_monday()
                        - weekday.num_days_from_monday())
                        % 7;
                    today - TimeDelta::days(i64::from(back))
                })
        };
        if let Some(day) = day.filter(|day| !days.contains(day)) {
            days.push(day);
        }
        if days.len() == 3 {
            break;
        }
    }
    days
}

fn month(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let prefix = name.get(..3)?.to_lowercase();
    MONTHS
        .iter()
        .position(|m| *m == prefix)
        .and_then(|i| u32::try_from(i + 1).ok())
}

/// A month and day, in `year` or else the latest year that keeps it from
/// being after `today`.
fn month_day(month: u32, day: u32, year: Option<u32>, today: NaiveDate) -> Option<NaiveDate> {
    if let Some(year) = year {
        return NaiveDate::from_ymd_opt(i32::try_from(year).ok()?, month, day);
    }
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if this_year <= today {
        Some(this_year)
    } else {
        NaiveDate::from_ymd_opt(today.year() - 1, month, day)
    }
}

/// Midnight to midnight of a calendar day, local time.
pub fn calendar_day(day: NaiveDate) -> Span {
    let midnight = |day: NaiveDate| {
        Local
            .from_local_datetime(&day.and_time(NaiveTime::MIN))
            .earliest()
            .map_or(0, |t| t.timestamp_millis())
    };
    Span {
        start_ms: midnight(day),
        end_ms: midnight(day.succ_opt().unwrap_or(day)),
    }
}

pub fn local(ms: i64) -> Option<DateTime<Local>> {
    Local.timestamp_millis_opt(ms).single()
}

pub fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(date: &str, hour: u32, minute: u32) -> DateTime<Local> {
        let day: NaiveDate = date.parse().unwrap();
        let naive = day.and_time(NaiveTime::from_hms_opt(hour, minute, 0).unwrap());
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    #[test]
    fn work_after_midnight_belongs_to_the_day_before() {
        let day: NaiveDate = "2026-10-12".parse().unwrap();
        assert_eq!(working_day(at("2026-10-13", 0, 48)), day);
        assert_eq!(working_day(at("2026-10-13", 3, 59)), day);
        assert_eq!(working_day(at("2026-10-13", 4, 0)), day.succ_opt().unwrap());
    }

    #[test]
    fn a_day_spans_from_four_to_four() {
        let span = day_span("2026-10-12".parse().unwrap());
        assert_eq!(span.start_ms, at("2026-10-12", 4, 0).timestamp_millis());
        assert_eq!(span.end_ms, at("2026-10-13", 4, 0).timestamp_millis());
    }

    #[test]
    fn finds_the_days_a_question_names() {
        let today: NaiveDate = "2026-10-13".parse().unwrap(); // a Tuesday
        let day = |s: &str| s.parse::<NaiveDate>().unwrap();
        for (text, expected) in [
            (
                "action items from my Sep 7 Website sync with Raj",
                vec![day("2026-09-07")],
            ),
            ("the 7th of September call", vec![day("2026-09-07")]),
            (
                "on 2026-09-07 and Sept. 9th",
                vec![day("2026-09-07"), day("2026-09-09")],
            ),
            ("yesterday's standup", vec![day("2026-10-12")]),
            ("Monday's call", vec![day("2026-10-12")]),
            ("Tuesday", vec![day("2026-10-13")]),
            ("Wednesday", vec![day("2026-10-07")]),
            // Not after today: December is last year's.
            ("the Dec 3 review", vec![day("2025-12-03")]),
            ("what did Ben say?", vec![]),
            ("in 2 weeks", vec![]),
        ] {
            assert_eq!(named_days(text, today), expected, "{text}");
        }
    }

    #[test]
    fn labels() {
        let day: NaiveDate = "2026-10-12".parse().unwrap();
        assert_eq!(day_label(day), "Monday 12 October 2026");
        assert_eq!(short_day_label(day), "Mon 12 Oct");
    }
}
