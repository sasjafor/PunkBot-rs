# PunkBot-rs

A Rust implementation of [PunkBot](https://github.com/sasjafor/PunkBot), following the `src/main.rs`, `src/commands`, and `src/util` layout used by Announcer-Bot. It uses Serenity and Poise for Discord interactions and Songbird for voice playback.

## Requirements

- Rust toolchain (Edition 2024)
- FFmpeg and `ffprobe`
- `yt-dlp` and Node.js 22 or newer (for YouTube's JavaScript player challenges)
- `libopus` development files when building on Linux
- A Discord application bot token and a YouTube Data API key

Enable the Discord **Message Content Intent** for the application if you want the legacy `!` command notice. Slash commands and playback use the guild, voice-state, and guild-message intents.

Set `DISCORD_APP_AUTH_TOKEN` and `YOUTUBE_API_KEY`, then run:

```sh
cargo run --release
```

The original CLI options remain available as `--token=...` and `--key=...`; environment variables take effect when an option is omitted.

PunkBot-rs registers its global slash commands when it connects. The legacy `!` prefix only responds with a notice to use slash commands; `!register` confirms that the registered commands are already available.

## Commands

| Command | Options | Behavior |
| --- | --- | --- |
| `/play` | `query`, `silent`, `next` | Play a YouTube search, video, playlist, or `yt-dlp` supported URL. |
| `/search` | `query`, `count` | Search YouTube and choose a result from a menu. |
| `/summon` | | Join your current voice channel. |
| `/now-playing` | | Show track and progress. |
| `/queue` | `index` | Show the queue with paging buttons. |
| `/skip`, `/pause`, `/resume`, `/stop` | | Playback controls. |
| `/loop` | | Toggle repeat for the current track. |
| `/seek` | `time` | Seek using `0:30`, `1:30`, or `1:02:15`. |
| `/remove` | `index` | Remove a waiting track (1-based). |
| `/clear`, `/shuffle` | | Clear or shuffle waiting tracks. |
| `/volume` | `value` | Set the volume multiplier. |
| `/filter` | `filters` | Pass an FFmpeg audio filter expression. |
| `/disconnect` | | Leave the voice channel. |

Playback disconnects after five idle minutes. Queues and settings are held in memory per guild; they reset when the process restarts.

## Optional settings

- `YOUTUBE_COOKIES_FILE`: Netscape-format YouTube cookies file for age-restricted media. If unset, PunkBot-rs also checks `.data/youtube.data`.
- `HEALTHCHECK_URL`: optional endpoint to ping every five minutes. No external healthcheck URL is built into the program.
- `AUDIO_CACHE_MAX_BYTES`: maximum combined size of temporary replay audio caches across tracks in this process. Defaults to `1073741824` (1 GiB); set to `0` to disable replay caching. Cache files are removed when their tracks are discarded; a startup and periodic cleanup removes files left behind by crashed processes.

## Docker

The included Dockerfile builds the binary and installs the media tools in the runtime image. Provide the two required environment variables when starting the container. Mount cookie data at `/config` and set `YOUTUBE_COOKIES_FILE=/config/youtube.data` if needed.
