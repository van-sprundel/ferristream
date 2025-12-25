# ferristream

Stream torrents directly to your media player. Search, select, watch - no permanent storage needed.

## Installation

Download the latest binary from [Releases](https://github.com/ramon-dev/ferristream/releases), or build from source:

```bash
cargo install --git https://github.com/ramon-dev/ferristream
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
