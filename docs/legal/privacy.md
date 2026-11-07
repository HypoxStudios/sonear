# Sonear privacy policy

Last updated: 6 November 2026

Sonear is maintained by Hypox Studios ("we"). This policy explains what Sonear collects, where it keeps it, and what it sends. Collecting and using your data in Sonear is subject to this policy.

The short version: Sonear runs entirely on your computer. We run no servers, and Sonear sends nothing to us.

## What Sonear collects

When you connect your Spotify account, Sonear reads from Spotify:

- your profile (display name, user ID and avatar);
- your library: saved tracks, albums and shows, your playlists, and the artists you follow;
- your top tracks and artists, and your recently played tracks;
- what is playing, and the playback queue.

You approve these permissions on Spotify's login page. Sonear asks only for what its features use. It changes your Spotify account only when you ask it to: controlling playback, and saving or removing a track with the heart.

## Where it is kept

Everything stays on your computer:

| What | Where | How long |
|---|---|---|
| Spotify login token, Soloist key | Windows Credential Manager | Until you log out or delete your data |
| Settings, including your Client ID | `%APPDATA%\Sonear\config.toml` | Until you delete your data |
| Listening history (time played, track and context links only) | `%LOCALAPPDATA%\Sonear\history.db` | 12 months by default, adjustable in Settings |
| Your Sonear queue (track links only) | `%LOCALAPPDATA%\Sonear\queue.json` | Until you log out |
| Track, album and playlist details | `%LOCALAPPDATA%\Sonear\cache` | Up to 7 days |
| Cover art | `%LOCALAPPDATA%\Sonear\cache\covers` | Up to 30 days, at most 200 MB |
| Soloist's own data, including its pairing | `%LOCALAPPDATA%\Sonear\soloist` | Until you delete your data |

Sonear builds no listening statistics or profile of you. The history is a plain list of plays.

## What Sonear sends, and to whom

- **Spotify.** Sonear talks to the Spotify Web API, and Soloist talks to Spotify to play music. Spotify's [privacy policy](https://www.spotify.com/legal/privacy-policy/) covers what Spotify does with this.
- **LRCLIB.** While the lyrics panel is open, Sonear sends the playing track's title, artist, album and length to [lrclib.net](https://lrclib.net) to fetch lyrics. Lyrics are kept in memory only and never saved to disk. You can turn lyrics off in Settings.
- **Spotify's download server.** When you install or update Soloist, Sonear downloads it from Spotify.
- **Windows.** Sonear shows the playing song's title, artist, album and cover in Windows' media controls, where other apps on your computer can read them.

Sonear sends nothing else. It has no analytics, telemetry, crash reporting or advertising, and it never sells or shares your data.

## Cookies

Sonear uses no cookies and lets no third party place cookies or track your browsing. The Spotify login page opens in your browser, where Spotify's own cookie policy applies; you manage those cookies in your browser.

## AI features

This version of Sonear has no AI features. If later versions add them, they will be off by default, each will explain what it sends before you turn it on, and this policy will be updated first.

## Your choices

- **Log out** (Settings or the account menu) removes your Spotify login, cached details, cover art, history and queue from this computer at once. Your Soloist key and pairing stay.
- **Delete all Sonear data** (Settings) also removes your Soloist key, Soloist's pairing and data, and your settings. Only the Soloist program itself stays, and you can delete the `%LOCALAPPDATA%\Sonear` folder to remove it.
- **Revoke access** at any time on your Spotify account's [Manage apps](https://www.spotify.com/account/apps/) page, or delete your developer app in the [Spotify dashboard](https://developer.spotify.com/dashboard).

Because Sonear stores nothing outside your computer, we hold no data about you to access, correct or delete.

## Spotify

Spotify is a third-party beneficiary of this policy and of Sonear's [end-user agreement](terms.md).

## Changes

If we change this policy, Sonear asks you to accept the new version before you continue.

## Contact

Questions about this policy or your data: email <support@hypox.studio>.
