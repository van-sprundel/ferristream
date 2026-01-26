# ferristream

Stream torrents directly to your media player. Search, select, watch.

## Motivation

Sometimes I just want to try out a movie/series without having to download their entire discography. This tools makes it possible to connect to my indexer, start a sequential torrent and see if I dig it.

## Disclaimer

This project is partly written by Claude Opus 4.5

## Installation

### Quick Install (Recommended)

**macOS/Linux:**
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/van-sprundel/ferristream/releases/latest/download/ferristream-installer.sh | sh
```

**Windows (PowerShell):**
```powershell
powershell -ExecutionPolicy ByPass -c "irm https://github.com/van-sprundel/ferristream/releases/latest/download/ferristream-installer.ps1 | iex"
```

### Alternative Methods

**cargo-binstall** (downloads pre-built binary):
```bash
cargo binstall ferristream
```

**Manual download:**
Download the latest binary from [Releases](https://github.com/van-sprundel/ferristream/releases).

**Build from source:**
```bash
cargo install --git https://github.com/van-sprundel/ferristream
```

## Configuration

```toml
[prowlarr]
url = "http://localhost:9696"
apikey = "your-prowlarr-api-key"

[player]
command = "mpv"

# Optional - TMDB for autocomplete and metadata
[tmdb]
apikey = "your-tmdb-api-key"

# Optional - auto-race torrents (0 = disabled, shows manual selection)
[streaming]
auto_race = 10  # race top 10 torrents, pick first matching one

# Optional - auto-fetch subtitles
[subtitles]
enabled = true
language = "en"
opensubtitles_api_key = "your-key"  # from opensubtitles.com

# Optional - Discord rich presence
[extensions.discord]
enabled = true
app_id = "your-discord-app-id"

# Optional - Trakt scrobbling
[extensions.trakt]
enabled = true
client_id = "your-trakt-client-id"
access_token = "your-trakt-access-token"
```

## Requirements

- [Prowlarr](https://prowlarr.com/) instance with configured indexers
- Media player (mpv, vlc, etc.)
