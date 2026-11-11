# Sonear privacy policy

Last updated: 10 November 2026

Sonear is maintained by Hypox Studios ("we"). This policy explains what Sonear collects, where it keeps it, and what it sends. Collecting and using your data in Sonear is subject to this policy.

The short version: Sonear runs entirely on your computer and sends nothing to us. It needs no servers of ours.

Sonear plays from one of two sources, Spotify or YT Music, and you pick which one in the app. Each section below says which source it applies to.

## What Sonear collects

### Spotify

When you connect your Spotify account, Sonear reads from Spotify:

- your profile (display name, user ID and avatar);
- your library: saved tracks, albums and shows, your playlists, and the artists you follow;
- your top tracks and artists, and your recently played tracks;
- what is playing, and the playback queue.

You approve these permissions on Spotify's login page. Sonear asks only for what its features use. It changes your Spotify account only when you ask it to: controlling playback, and saving or removing a track with the heart.

### YT Music

YT Music works without an account. Signed out, Sonear reads only public catalogue pages (search, albums, artists, playlists and the home page), and your saved music stays on your computer.

Signing in is optional. If you sign in, you do it on Google's own page, inside a Sonear window. Sonear never sees your password. It then reads from YT Music:

- your account name and picture;
- your library: liked songs, saved playlists and albums, and the artists you subscribe to.

It changes your YT Music account only when you ask it to: liking or unliking a song, and saving or removing a playlist. Playback never uses your sign-in, so what you play in Sonear is not added to your YouTube history.

## Where it is kept

Everything stays on your computer:

| What | Where | How long |
|---|---|---|
| Spotify login token, Soloist key | Windows Credential Manager | Until you log out or delete your data |
| Settings, including your Client ID and which source you use | `%APPDATA%\Sonear\config.toml` | Until you delete your data |
| Spotify listening history (time played, track and context links only) | `%LOCALAPPDATA%\Sonear\history.db` by default | 12 months by default, adjustable in Settings |
| Your Sonear queue for Spotify (track links only) | `%LOCALAPPDATA%\Sonear\queue.json` by default | Until you log out |
| Spotify track, album and playlist details | `%LOCALAPPDATA%\Sonear\cache` by default | Up to 7 days |
| Cover art, for both sources | `%LOCALAPPDATA%\Sonear\cache\covers` by default | Up to 30 days, at most 200 MB |
| Soloist's own data, including its pairing | `%LOCALAPPDATA%\Sonear\soloist` by default | Until you delete your data |
| YT Music sign-in: a browser profile holding Google's cookies, the only copy of your sign-in | `%LOCALAPPDATA%\Sonear\youtube\webview` by default | Until you sign out or delete your data |
| YT Music songs and albums you saved in Sonear, and your last 200 plays | `%LOCALAPPDATA%\Sonear\youtube\library.json` and `plays.json` by default | Until you delete your data |
| YT Music listening history (time played, track and context links only) | `%LOCALAPPDATA%\Sonear\youtube\history.db` by default | 12 months by default, adjustable in Settings |
| Your Sonear queue for YT Music (track links only) | `%LOCALAPPDATA%\Sonear\youtube\queue.json` by default | Until you delete your data |
| YT Music track, album and playlist details | `%LOCALAPPDATA%\Sonear\youtube\cache` by default | Up to 7 days |
| The YT Music playback tools, mpv and yt-dlp | `%LOCALAPPDATA%\Sonear\youtube\tools` by default | Until you delete the data folder or uninstall |
| A downloaded Sonear update, waiting for you to restart | `%LOCALAPPDATA%\Sonear\updates` by default | Until you restart into it, or a newer one replaces it |
| Logs of what Sonear and Soloist did, with no login details, cookies or keys | `%LOCALAPPDATA%\Sonear\logs` by default | The last 7 days |

You can move the data folder in Settings; the table shows the default location. Sonear builds no listening statistics or profile of you. The history is a plain list of plays.

## What Sonear sends, and to whom

- **Spotify.** When you use the Spotify source, Sonear talks to the Spotify Web API, and Soloist talks to Spotify to play music. Spotify's [privacy policy](https://www.spotify.com/legal/privacy-policy/) covers what Spotify does with this.
- **YouTube and Google.** When you use the YT Music source, Sonear sends your searches and the pages you open to music.youtube.com, and yt-dlp and mpv fetch the songs you play from YouTube. If you signed in, the requests that read or change your library carry your sign-in, and Google's sign-in page sees what you type into it. Google's [privacy policy](https://policies.google.com/privacy) covers what Google does with this.
- **Microsoft.** The YT Music sign-in window uses Microsoft Edge WebView2, which is part of Windows. Microsoft may collect diagnostic data from it according to your Windows settings, under [Microsoft's privacy statement](https://privacy.microsoft.com/privacystatement).
- **LRCLIB.** While lyrics are showing, in the lyrics panel or the mini player, Sonear sends the playing track's title, artist, album and length to [lrclib.net](https://lrclib.net) to fetch lyrics. Lyrics are kept in memory only and never saved to disk. You can turn lyrics off in Settings.
- **GitHub.** Unless you turn it off in Settings, Sonear asks GitHub for the latest release when it starts and every few hours, and downloads new versions from it. When you set up the YT Music source, Sonear also downloads mpv and yt-dlp from their GitHub releases. GitHub sees your IP address like any website, and [GitHub's privacy statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement) covers it. Nothing about your account or your music is sent.
- **Spotify's download server.** When you install or update Soloist, Sonear downloads it from Spotify.
- **Windows.** Sonear shows the playing song's title, artist, album and cover in Windows' media controls, where other apps on your computer can read them.

Sonear sends nothing else. It has no analytics, telemetry, crash reporting or advertising, and it never sells or shares your data.

## Cookies

Sonear itself uses no cookies and lets no third party track your browsing. The Spotify login page opens in your browser, where Spotify's own cookie policy applies; you manage those cookies in your browser.

The YT Music sign-in is the one exception: Google sets its cookies in Sonear's own sign-in window, and Sonear uses them only to read and change your YT Music library as described above. They stay in the sign-in profile on your computer and are deleted when you sign out.

## This website

sonear.hypox.studio is a static site served by Cloudflare. Cloudflare handles each request and sees your IP address and browser details, which it processes for us under [Cloudflare's privacy policy](https://www.cloudflare.com/privacypolicy/). We keep no logs of visits, and the site sets no cookies and has no analytics or third-party scripts. Downloads come from GitHub.

## AI features

This version of Sonear has no AI features. If later versions add them, they will be off by default, each will explain what it sends before you turn it on, and this policy will be updated first.

## Your choices

- **Log out of Spotify** (Settings or the account menu) removes your Spotify login, cached details, cover art, history and queue from this computer at once. Your Soloist key and pairing stay.
- **Sign out of YT Music** (Settings) deletes the sign-in profile and its cookies from this computer. Your saved music, plays and history in Sonear stay.
- **Delete all Sonear data** (Settings) removes everything above for both sources: your Spotify login, Soloist key, Soloist's pairing and data, your YT Music sign-in and saved music, your history, queues, caches, logs and settings. Only the Soloist program and the YT Music playback tools stay, and you can delete the data folder to remove them. Uninstalling Sonear offers to delete all of it, including your logins.
- **Revoke access** at any time on your Spotify account's [Manage apps](https://www.spotify.com/account/apps/) page, or delete your developer app in the [Spotify dashboard](https://developer.spotify.com/dashboard). For YT Music, you can also sign out of the Sonear session from your Google account's [security page](https://myaccount.google.com/device-activity).

Because Sonear stores nothing outside your computer, we hold no data about you to access, correct or delete.

## Spotify

Spotify is a third-party beneficiary of this policy and of Sonear's [end-user agreement](terms.md).

## Changes

If we change this policy, Sonear asks you to accept the new version before you continue.

## Contact

Questions about this policy or your data: email <support@hypox.studio>.
