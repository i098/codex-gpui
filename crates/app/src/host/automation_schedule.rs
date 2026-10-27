use chrono::{DateTime, Datelike, Duration, NaiveDateTime, Offset, TimeZone, Timelike, Utc};
use rrule::{Frequency, RRule, RRuleSet, Tz};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub(super) const NO_FUTURE_RUN: &str = "Automation schedule has no future runs. Check that its start time, interval, and BYHOUR/BYMINUTE/BYSECOND values can reach a future occurrence.";

pub(super) fn valid(rule: &str, now: i64) -> bool {
    Schedule::parse(rule, now).is_some()
}

// Original GF/KF/JF/XF: recurrence iteration stays in the RFC library.
pub(super) fn next_run(
    rule: &str,
    kind: &str,
    id: &str,
    now: i64,
    zone: Tz,
    salt: &str,
) -> Option<(i64, i64)> {
    next_run_after(rule, kind, id, now, None, zone, salt)
}

/// `next_run` with KF's `afterNominalRunAt`: the search starts at the later of it and `now`.
pub(super) fn next_run_after(
    rule: &str,
    kind: &str,
    id: &str,
    now: i64,
    after: Option<i64>,
    zone: Tz,
    salt: &str,
) -> Option<(i64, i64)> {
    // Like the original's JavaScript Date: `TZ` wins, and an unknown or missing zone is UTC.
    let zone = if zone.is_local() {
        std::env::var("TZ")
            .ok()
            .or_else(|| iana_time_zone::get_timezone().ok())
            .and_then(|name| name.parse::<chrono_tz::Tz>().ok())
            .map_or(Tz::UTC, Tz::from)
    } else {
        zone
    };
    let search = after.map_or(now, |after| after.max(now));
    let schedule = Schedule::parse(rule, search)?;
    let occurrence = schedule.next(search, zone)?;
    let nominal = if kind == "heartbeat" {
        schedule
            .heartbeat_interval()
            .and_then(|interval| now.checked_add(interval))
            .unwrap_or(occurrence)
    } else {
        occurrence
    };
    let jitter = if schedule.jitter() {
        let digest = Sha256::digest(format!("{salt}:{id}:{nominal}"));
        (i64::from(u32::from_be_bytes(digest[..4].try_into().ok()?)) % 601 - 300) * 1000
    } else {
        0
    };
    Some((now.max(nominal.checked_add(jitter)?), nominal))
}

pub(super) struct Schedule<'a> {
    text: String,
    set: RRuleSet,
    keys: HashSet<&'a str>,
    explicit_start: bool,
    tzid: Option<Tz>,
}

impl<'a> Schedule<'a> {
    pub(super) fn parse(text: &'a str, now: i64) -> Option<Self> {
        let text = text.trim();
        let explicit_start = text.lines().any(|line| line.starts_with("DTSTART"));
        let keys = text
            .lines()
            .find(|line| line.starts_with("RRULE:") || line.starts_with("FREQ="))
            .unwrap_or("")
            .trim_start_matches("RRULE:")
            .split(';')
            .filter_map(|field| field.split_once('=').map(|(key, _)| key))
            .collect();
        let tzid = text.lines().find_map(|line| {
            line.strip_prefix("DTSTART;TZID=")
                .and_then(|line| line.split_once(':').map(|(zone, _)| zone))
        });
        let tzid = tzid
            .map(str::parse::<chrono_tz::Tz>)
            .transpose()
            .ok()?
            .map(Tz::from);
        let normalized = if text.starts_with("FREQ=") {
            format!("RRULE:{text}")
        } else {
            text.lines()
                .map(|line| {
                    if let Some((property, date)) = line.split_once(':') {
                        let name = property.split(';').next().unwrap_or(property);
                        if matches!(name, "DTSTART" | "RDATE" | "EXDATE") {
                            return format!("{name};TZID=UTC:{date}");
                        }
                    }
                    line.to_owned()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let start = Utc
            .timestamp_millis_opt(now)
            .single()?
            .with_second(0)?
            .with_nanosecond(0)?;
        let set = RRuleSet::new(start.with_timezone(&Tz::UTC))
            .set_from_string(&normalized)
            .ok()?;
        Some(Self {
            text: normalized,
            set,
            keys,
            explicit_start,
            tzid,
        })
    }

    fn single(&self) -> bool {
        self.set.get_rrule().len() == 1
            && self.set.get_exrule().is_empty()
            && self.set.get_rdate().is_empty()
            && self.set.get_exdate().is_empty()
    }

    fn only(&self, keys: &[&str]) -> bool {
        self.keys.iter().all(|key| keys.contains(key))
    }

    pub(super) fn heartbeat_interval(&self) -> Option<i64> {
        if !self.single() || self.explicit_start {
            return None;
        }
        let rule = self.set.get_rrule().first()?;
        // A rule with a next occurrence before year 10000 keeps this product within `i64`.
        let interval = i64::try_from(rule.get_interval()).ok()?;
        match rule.get_freq() {
            Frequency::Minutely if self.only(&["FREQ", "INTERVAL", "TZID"]) => {
                interval.checked_mul(60_000)
            }
            Frequency::Hourly
                if self.only(&["FREQ", "INTERVAL", "TZID", "BYDAY", "BYMINUTE"])
                    && (!self.keys.contains("BYMINUTE") || rule.get_by_minute() == [0])
                    && (!self.keys.contains("BYDAY") || rule.get_by_weekday().len() == 7) =>
            {
                interval.checked_mul(3_600_000)
            }
            _ => None,
        }
    }

    fn jitter(&self) -> bool {
        if !self.single() {
            return false;
        }
        let rule = &self.set.get_rrule()[0];
        if rule.get_count() == Some(1) {
            return false;
        }
        if rule.get_freq() == Frequency::Minutely {
            return rule.get_interval() >= 60
                && self.only(&["FREQ", "INTERVAL", "TZID", "COUNT", "UNTIL"]);
        }
        rule.get_by_minute()
            .windows(2)
            .all(|pair| pair[0] == pair[1])
            && rule
                .get_by_second()
                .windows(2)
                .all(|pair| pair[0] == pair[1])
            && matches!(
                rule.get_freq(),
                Frequency::Hourly | Frequency::Daily | Frequency::Weekly
            )
    }
    fn next(&self, now: i64, zone: Tz) -> Option<i64> {
        let Some(rule) = self.set.get_rrule().first() else {
            return (&self.set)
                .into_iter()
                .map(|date| self.rezoned(date, zone))
                .find(|&date| date > now);
        };
        let simple = self.single() && !self.explicit_start;
        let explicit_time = self.keys.contains("BYHOUR") && self.keys.contains("BYMINUTE");
        if simple && rule.get_count() == Some(1) && !explicit_time {
            return None;
        }
        let daily_keys = [
            "FREQ", "INTERVAL", "TZID", "BYDAY", "BYMINUTE", "BYHOUR", "BYSECOND",
        ];
        let local_daily = simple
            && rule.get_interval() == 1
            && self.keys.iter().all(|key| {
                daily_keys.contains(key) || *key == "COUNT" && rule.get_count() == Some(1)
            })
            && (rule.get_count() != Some(1) || explicit_time)
            && (explicit_time || !self.keys.contains("BYSECOND") || rule.get_by_second() == [0]);
        if local_daily && matches!(rule.get_freq(), Frequency::Daily | Frequency::Weekly) {
            return self.daily(now, zone);
        }
        let mut start = Utc
            .timestamp_millis_opt(now)
            .single()?
            .with_second(0)?
            .with_nanosecond(0)?;
        let local_hourly = simple
            && rule.get_freq() == Frequency::Hourly
            && self.only(&daily_keys)
            && self.keys.contains("BYHOUR")
            && !self.keys.contains("TZID");
        let local_setpos = simple
            && matches!(
                rule.get_freq(),
                Frequency::Yearly | Frequency::Monthly | Frequency::Weekly | Frequency::Daily
            )
            && self.keys.contains("BYSETPOS")
            && !self.keys.contains("TZID")
            && !self.keys.contains("UNTIL")
            && explicit_time;
        let local = local_hourly || local_setpos;
        let comparison = if local {
            Utc.from_utc_datetime(
                &Utc.timestamp_millis_opt(now)
                    .single()?
                    .with_timezone(&zone)
                    .naive_local(),
            )
            .timestamp_millis()
        } else {
            now
        };
        if local {
            start = Utc
                .timestamp_millis_opt(comparison)
                .single()?
                .with_second(0)?
                .with_nanosecond(0)?;
        }
        if local_hourly
            && !rule.get_by_hour().iter().any(|hour| {
                (0..24).any(|step| {
                    (u64::from(start.hour()) + step * (rule.get_interval() % 24)) % 24
                        == u64::from(*hour)
                })
            })
        {
            start = start.with_hour(u32::from(*rule.get_by_hour().iter().min()?))?;
            if self.keys.contains("BYMINUTE") {
                start = start.with_minute(u32::from(*rule.get_by_minute().iter().min()?))?;
            }
        }
        let set = RRuleSet::new(start.with_timezone(&Tz::UTC))
            .set_from_string(&self.text)
            .ok()?;
        if set
            .get_rrule()
            .iter()
            .chain(set.get_exrule())
            .any(|rule| !reachable(rule, *set.get_dt_start()))
        {
            return None;
        }
        let date = set
            .into_iter()
            .find(|date| self.rezoned(*date, zone) > comparison)?;
        if local {
            let converted = local_date(date.naive_utc(), zone)?;
            let mut result = converted.timestamp_millis();
            if result <= now {
                let current_offset = Utc
                    .timestamp_millis_opt(now)
                    .single()?
                    .with_timezone(&zone)
                    .offset()
                    .fix()
                    .local_minus_utc();
                result +=
                    i64::from(converted.offset().fix().local_minus_utc() - current_offset) * 1000;
            }
            Some(result)
        } else {
            Some(self.rezoned(date, zone))
        }
    }

    // Original pN shifts UTC recurrence fields by the target offset minus the host offset.
    fn rezoned(&self, date: DateTime<Tz>, local: Tz) -> i64 {
        let Some(target) = self.tzid.filter(|zone| zone.name() != "UTC") else {
            return date.timestamp_millis();
        };
        let target_offset = date.with_timezone(&target).offset().fix().local_minus_utc();
        let local_offset = date.with_timezone(&local).offset().fix().local_minus_utc();
        date.timestamp_millis() - i64::from(target_offset - local_offset) * 1000
    }

    fn daily(&self, now: i64, zone: Tz) -> Option<i64> {
        let rule = &self.set.get_rrule()[0];
        let current = Utc.timestamp_millis_opt(now).single()?.with_timezone(&zone);
        let seconds = if self.keys.contains("BYSECOND") {
            rule.get_by_second()
        } else {
            &[0]
        };
        let mut next = None;
        for day in 0..=7 {
            let date = current
                .date_naive()
                .checked_add_signed(Duration::days(day))?;
            if !rule.get_by_weekday().is_empty()
                && !rule
                    .get_by_weekday()
                    .iter()
                    .any(|value| weekday(value) == date.weekday())
            {
                continue;
            }
            for &hour in rule.get_by_hour() {
                for &minute in rule.get_by_minute() {
                    for &second in seconds {
                        // Original fI builds a JavaScript Date, so second 60 moves into the next minute.
                        let time = date
                            .and_hms_opt(u32::from(hour), u32::from(minute), 0)?
                            .checked_add_signed(Duration::seconds(i64::from(second)))?;
                        let mut candidate = local_date(time, zone)?;
                        if rule.get_freq() == Frequency::Daily
                            && rule.get_by_weekday().is_empty()
                            && candidate.timestamp_millis() <= now
                        {
                            candidate = local_date(
                                candidate
                                    .naive_local()
                                    .checked_add_signed(Duration::days(1))?,
                                zone,
                            )?;
                        }
                        let candidate = candidate.timestamp_millis();
                        if candidate > now {
                            next = Some(next.map_or(candidate, |old: i64| old.min(candidate)));
                        }
                    }
                }
            }
            if next.is_some() {
                break;
            }
        }
        next
    }
}

// JavaScript Date picks the earlier overlap and moves a gap forward by its size.
fn local_date(date: NaiveDateTime, zone: Tz) -> Option<DateTime<Tz>> {
    if let Some(result) = zone.from_local_datetime(&date).earliest() {
        return Some(result);
    }
    let before = zone
        .from_local_datetime(&date.checked_sub_signed(Duration::days(1))?)
        .earliest()?;
    let after = zone
        .from_local_datetime(&date.checked_add_signed(Duration::days(1))?)
        .earliest()?;
    let gap = after.offset().fix().local_minus_utc() - before.offset().fix().local_minus_utc();
    zone.from_local_datetime(&date.checked_add_signed(Duration::seconds(i64::from(gap)))?)
        .earliest()
}

// Original qP rejects interval cycles that cannot reach their time-of-week filters.
fn reachable(rule: &RRule, start: DateTime<Tz>) -> bool {
    let step = match rule.get_freq() {
        Frequency::Hourly => rule.get_interval().checked_mul(3600),
        Frequency::Minutely => rule.get_interval().checked_mul(60),
        Frequency::Secondly => Some(rule.get_interval()),
        _ => return true,
    };
    // The step in seconds must stay a JavaScript safe integer.
    let Some(step) = step.filter(|step| *step <= 9_007_199_254_740_991) else {
        return false;
    };
    let step = u32::try_from(step % 604_800).expect("a week of seconds fits u32");
    let initial =
        start.weekday().num_days_from_monday() * 86400 + start.num_seconds_from_midnight();
    let mut time = initial;
    loop {
        let day = time / 86400;
        let hour = (time % 86400 / 3600) as u8;
        let minute = (time % 3600 / 60) as u8;
        let second = (time % 60) as u8;
        if (rule.get_by_weekday().is_empty()
            || rule
                .get_by_weekday()
                .iter()
                .any(|value| weekday(value).num_days_from_monday() == day))
            && (rule.get_by_hour().is_empty() || rule.get_by_hour().contains(&hour))
            && (rule.get_freq() == Frequency::Hourly
                || rule.get_by_minute().is_empty()
                || rule.get_by_minute().contains(&minute))
            && (rule.get_freq() != Frequency::Secondly
                || rule.get_by_second().is_empty()
                || rule.get_by_second().contains(&second))
        {
            return true;
        }
        time = (time + step) % 604800;
        if time == initial {
            return false;
        }
    }
}

fn weekday(value: &rrule::NWeekday) -> chrono::Weekday {
    match *value {
        rrule::NWeekday::Every(day) | rrule::NWeekday::Nth(_, day) => day,
    }
}
