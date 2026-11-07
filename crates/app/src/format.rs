use rust_i18n::t;

/// `m:ss`, or `h:mm:ss` from an hour up.
pub fn clock(ms: u64) -> String {
    let total = ms / 1000;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{}:{seconds:02}", total / 60)
    }
}

const WEEKDAYS: [&str; 7] = [
    "date.wd.mon",
    "date.wd.tue",
    "date.wd.wed",
    "date.wd.thu",
    "date.wd.fri",
    "date.wd.sat",
    "date.wd.sun",
];
const MONTHS: [&str; 12] = [
    "date.mo.jan",
    "date.mo.feb",
    "date.mo.mar",
    "date.mo.apr",
    "date.mo.may",
    "date.mo.jun",
    "date.mo.jul",
    "date.mo.aug",
    "date.mo.sep",
    "date.mo.oct",
    "date.mo.nov",
    "date.mo.dec",
];

/// `2 hr 18 min` style length for a whole playlist.
pub fn long_duration(ms: u64) -> String {
    long_duration_in(ms, &rust_i18n::locale())
}

fn long_duration_in(ms: u64, locale: &str) -> String {
    let minutes = (ms + 30_000) / 60_000;

    match (minutes / 60, minutes % 60) {
        (0, minutes) => t!("duration.minutes", locale = locale, minutes = minutes),
        (hours, 0) => t!("duration.hours", locale = locale, hours = hours),
        (hours, minutes) => t!(
            "duration.hours_minutes",
            locale = locale,
            hours = hours,
            minutes = minutes
        ),
    }
    .into_owned()
}

/// `Fri, Nov 6 · 20:41` in local time, for the history list.
pub fn played_at(ms: u64) -> String {
    use chrono::TimeZone as _;

    chrono::Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map_or_else(String::new, |time| {
            played_at_in(&time, &rust_i18n::locale())
        })
}

fn played_at_in<T: chrono::Datelike + chrono::Timelike>(time: &T, locale: &str) -> String {
    let weekday = WEEKDAYS[time.weekday().num_days_from_monday() as usize];
    let month = MONTHS[time.month0() as usize];

    t!(
        "date.played_at",
        locale = locale,
        weekday = t!(weekday, locale = locale),
        month = t!(month, locale = locale),
        day = time.day(),
        time = format!("{:02}:{:02}", time.hour(), time.minute())
    )
    .into_owned()
}

/// Spotify descriptions carry HTML tags and entities; show them as plain text.
pub fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }

    out.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_owned()
}

pub fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

pub fn greeting(hour: u32) -> String {
    greeting_in(hour, &rust_i18n::locale())
}

fn greeting_in(hour: u32, locale: &str) -> String {
    match hour {
        5..=11 => t!("greeting.morning", locale = locale),
        12..=17 => t!("greeting.afternoon", locale = locale),
        _ => t!("greeting.evening", locale = locale),
    }
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_clock() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(222_000), "3:42");
        assert_eq!(clock(3_725_000), "1:02:05");
    }

    #[test]
    fn formats_long_durations() {
        assert_eq!(long_duration(8_272_000), "2 hr 18 min");
        assert_eq!(long_duration(3_600_000), "1 hr");
        assert_eq!(long_duration(600_000), "10 min");
    }

    #[test]
    fn formats_long_durations_in_spanish() {
        assert_eq!(long_duration_in(8_272_000, "es"), "2 h 18 min");
        assert_eq!(long_duration_in(3_600_000, "es"), "1 h");
        assert_eq!(long_duration_in(600_000, "es"), "10 min");
    }

    #[test]
    fn formats_play_times_by_language() {
        use chrono::TimeZone as _;

        let time = chrono::NaiveDate::from_ymd_opt(2026, 11, 6)
            .and_then(|date| date.and_hms_opt(20, 41, 0))
            .unwrap();

        assert_eq!(played_at_in(&time, "en"), "Fri, Nov 6 \u{b7} 20:41");
        assert_eq!(played_at_in(&time, "es"), "vie, 6 nov \u{b7} 20:41");

        let local = chrono::Local.with_ymd_and_hms(2026, 3, 2, 8, 5, 0).unwrap();

        assert_eq!(played_at_in(&local, "en"), "Mon, Mar 2 \u{b7} 08:05");
        assert_eq!(played_at_in(&local, "es"), "lun, 2 mar \u{b7} 08:05");
    }

    #[test]
    fn greets_in_spanish() {
        assert_eq!(greeting_in(8, "es"), "Buenos d\u{ed}as.");
        assert_eq!(greeting_in(14, "es"), "Buenas tardes.");
        assert_eq!(greeting_in(2, "es"), "Buenas noches.");
    }

    #[test]
    fn descriptions_lose_markup() {
        assert_eq!(
            plain_text("Rock &amp; roll <a href=\"x\">hits</a> &#x27;now&#x27;"),
            "Rock & roll hits 'now'"
        );
    }

    #[test]
    fn greets_by_hour() {
        assert_eq!(greeting(8), "Good morning.");
        assert_eq!(greeting(14), "Good afternoon.");
        assert_eq!(greeting(21), "Good evening.");
        assert_eq!(greeting(2), "Good evening.");
    }
}
