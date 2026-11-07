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

/// `2 hr 18 min` style length for a whole playlist.
pub fn long_duration(ms: u64) -> String {
    let minutes = (ms + 30_000) / 60_000;

    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} hr"),
        (hours, minutes) => format!("{hours} hr {minutes} min"),
    }
}

/// `Fri, Nov 6 · 20:41` in local time, for the history list.
pub fn played_at(ms: u64) -> String {
    use chrono::TimeZone as _;

    chrono::Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map_or_else(String::new, |time| {
            time.format("%a, %b %-d · %H:%M").to_string()
        })
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

pub fn greeting(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning.",
        12..=17 => "Good afternoon.",
        _ => "Good evening.",
    }
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
