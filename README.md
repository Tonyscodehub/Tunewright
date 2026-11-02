# Tunewright

A small Windows remote for Music Assistant. It connects either straight to a Music Assistant server or through Home Assistant, and gives you play/pause, previous, skip, volume, mute, repeat (off, one, or all), shuffle, and search across tracks, albums, artists, playlists, and radio.

## Connecting

**Music Assistant directly** – use the server address, usually `http://homeassistant.local:8095` (or `http://<ip>:8095`).

- *Username and password*: your Music Assistant login. The password isn't saved; Tunewright swaps it for a long-lived token named "Tunewright", which you can revoke in Music Assistant under your profile.
- *Token*: a long-lived token created in Music Assistant under your profile settings.
- Older servers (before authentication was added) connect without signing in.

**Through Home Assistant** – use the Home Assistant address, usually `http://homeassistant.local:8123`.

- *Username and password*: your Home Assistant login. The password isn't saved; a refresh token is kept instead. Accounts with two-factor sign-in need a token.
- *Token*: a long-lived access token from your Home Assistant profile, Security tab.
- Players are the Music Assistant `media_player` entities. Search and play use the `music_assistant.search` and `music_assistant.play_media` actions, so the Music Assistant integration must be installed in Home Assistant.

Settings are saved in `%APPDATA%\Tunewright\config.json`. Untick "Remember me" to keep tokens out of that file.

## Using it

- Pick a player on the left. Whatever's playing is selected first.
- The drop-down beside Search sets what the play button on a result does: play now, replace the queue, play next, or add to the end.
- If the connection drops, Tunewright reconnects on its own.

## Building

```
cargo build --release
```

The Windows build lands in `target\release\tunewright.exe`. Pushing to GitHub runs `.github/workflows/build.yml`, which builds the exe on a Windows runner and attaches it to any `v*` tag's release.

Backend tests run against a mock server: `TUNEWRIGHT_MOCK=http://127.0.0.1:8899 cargo test -- --ignored`.
