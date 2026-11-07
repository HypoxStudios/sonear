use std::time::Duration;

use async_trait::async_trait;

use super::catalog::catalog;
use crate::lyrics::{LyricLine, Lyrics, LyricsError, LyricsQuery, LyricsSource};

const PHRASES: [&str; 10] = [
    "Streetlights blur along the glass",
    "We keep the engine humming low",
    "Every mile a quiet promise",
    "Hold the horizon a little longer",
    "Nothing here is moving slow",
    "Count the signals as they fade",
    "Headlights paint the empty road",
    "Say it once and let it go",
    "The city hums beneath the rain",
    "Tonight we're only passing through",
];
const FIRST_LINE_MS: u64 = 8000;
const LINE_MS: u64 = 5500;
const TAIL_MS: u64 = 6000;
const LOOKUP_DELAY: Duration = Duration::from_millis(350);

/// Generated lyrics timed across the track, so the panel can be tried offline.
pub struct DemoLyrics;

#[async_trait]
impl LyricsSource for DemoLyrics {
    async fn fetch(&self, query: &LyricsQuery) -> Result<Lyrics, LyricsError> {
        tokio::time::sleep(LOOKUP_DELAY).await;

        let index = catalog().track(&query.uri).map(|track| track.index);

        match index {
            Some(3 | 4) => return Ok(Lyrics::NotFound),
            Some(5) => return Ok(Lyrics::Instrumental),
            Some(6) => {
                return Ok(Lyrics::Plain(
                    PHRASES
                        .iter()
                        .take(6)
                        .copied()
                        .collect::<Vec<_>>()
                        .join("\n"),
                ));
            }
            _ => {}
        }

        let end = query.duration_ms.unwrap_or(180_000).saturating_sub(TAIL_MS);
        let offset = index.unwrap_or(0);
        let mut lines = Vec::new();
        let mut at_ms = FIRST_LINE_MS;
        let mut count = 0;

        while at_ms < end {
            let text = if count % 9 == 8 {
                String::new()
            } else {
                PHRASES[(offset + count) % PHRASES.len()].to_owned()
            };

            lines.push(LyricLine { at_ms, text });
            at_ms += LINE_MS + (count as u64 % 3) * 700;
            count += 1;
        }

        Ok(Lyrics::Synced(lines))
    }
}
