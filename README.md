# ferristream

Stream torrents directly to your media player. Search, select, watch - no permanent storage needed.

## Installation

Download the latest binary from [Releases](https://github.com/van-sprundel/ferristream/releases), or build from source:

```bash
cargo install --git https://github.com/van-sprundel/ferristream
```

## Configuration

Create `~/.config/ferristream/config.toml`:

```toml
[prowlarr]
url = "http://localhost:9696"
apikey = "your-prowlarr-api-key"

[player]
command = "mpv"

# Optional - auto-fetch subtitles
[subtitles]
enabled = true
language = "en"
opensubtitles_api_key = "your-key"  # from opensubtitles.com

# Optional - Discord rich presence
[extensions.discord]
enabled = true

# Optional - Trakt scrobbling
[extensions.trakt]
enabled = true
client_id = "your-trakt-client-id"
access_token = "your-trakt-access-token"
```

## Usage

```bash
ferristream
```

- Type to search, suggestions appear as you type
- `Tab` to accept suggestion, `Enter` to search
- `↑`/`↓` to navigate results
- `Enter` to stream
- `d` for connection diagnostics

## Requirements

- [Prowlarr](https://prowlarr.com/) instance with configured indexers
- Media player (mpv, vlc, etc.)
