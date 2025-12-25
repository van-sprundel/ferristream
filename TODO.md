## Vision

Search, select, watch but no permanent storage. Inspired by \*arr stack but for on-the-fly local streaming.

## Minimals

1. search via [torznab](https://torznab.github.io/spec-1.3-draft/torznab/Specification-v1.3.html)
2. add TMDB metadata
3. sort by seeders?? (highest = most likely to succeed right)
4. stream (seq but still dk how to do this) via librqbit

- Launch mpv

## laterons

- edit config in TUI
- support discord rich presence
- sync watch history using trakt
- subtitles
- multi-episode handling (select a season, DL in background but be able to start first ep?)
  - ig we can support both seq streaming and downloading in bg. should this be temp?

## config

- config.toml
  - Torznab indexers (name, url, apikey)
  - TMDB API key
  - Temp directory path (default: system temp)
  - Player command (default to mpv)
- XDG support (`~/.config/ferristream/`)
- validate on startup

# so how the helly banelly do we use torznab

- Endpoint: `GET /api?t=search&apikey=KEY&q=QUERY`
- Response: XML/RSS with `<torznab:attr name="seeders" value="N" />`
- Also extract: magneturl, size, infohash, title

## actual TODO??

- [ ] impl search function
  - build url (t=search, apikey, q, limit) and send with reqwest (maybe add torznab client?)
  - parse response (quick-xml or roxmltree not sure. are there xml definitions? xmld or smth)
  - response should be `title, seeders, size, magneturl/link, infohash` etc.
- [ ] impl caps discovery (`t=caps`)
- [ ] need to support multiple indexers (query all and merge results)
- Need to think out how to handle errors (timeout, invalid API key, no results) and how we want to apply TDD

# TMDB

- [ ] Research TMDB API endpoints
  - search endpoint for movies/tv
  - get metadata (overview, year)
- [ ] Implement search enrichment
  - match Torznab results to TMDB entries (title? title + year?)
  - cache temporarily somewhere
  - handle ambiguous matches (pick best by year/title similarity)

# streaming

## librqbit

librqbit has built-in HTTP server

- start server on `http://127.0.0.1:3030`
- stream endpoint: `/torrents/<torrent_id>/stream/<file_id>`
- block until pieces available + prioritize streamed pieces (this is OOTB so +1)
  - supports range headers/seeking

## first steps ig would be

- start Session with HTTP API enabled
  - `Session::new_with_opts()` with temp dir
  - `session.start_http_api("127.0.0.1:3030")`
- add magnet link via `session.add_torrent()`
- get file list from torrent handle
  - filter for video files (mp4, mkv, avi, etc)
  - skip if no media files found
- get torrent_id and file_id for the video
- build streaming URL: `http://127.0.0.1:3030/torrents/{id}/stream/{file_id}`
- monitor download progress via handle API

## mpv

point mpv at librqbit's streaming URL and let it handle buffering

- [ ] build stream URL from librqbit (see previous)
- [ ] spawn mpv with: `mpv http://127.0.0.1:3030/torrents/{id}/stream/{file_id}`
- [ ] pass flags:
  - `--force-seekable=yes` (range header support)
  - `--cache=yes --demuxer-max-bytes=150M` (buffer config)
  - `--hwdec=auto` (hardware decode)
- [ ] monitor mpv process state (playing, stopped)
- [ ] handle cleanup on exit (kill process, stop torrent)

# TUI

- [ ] first check ratatui
- [ ] need an event loop (keyboard input, tick updates)
- [ ] app state enum (Search, Browsing, Streaming, Settings)

## Views

**Search View (I let claude choose this part):**

- [ ] Input field for query
- [ ] Loading indicator while searching
- [ ] Error display

**Browse View:**

- [ ] Results list (title, year, seeders, size)
- [ ] TMDB metadata panel (poster, description)
- [ ] Highlight selected item
- [ ] Sort controls (seeders, size, name)

**Streaming View:**

- [ ] Now playing info (title, poster)
- [ ] Download progress bar
- [ ] Buffer state indicator
- [ ] Controls hint (q to quit, etc)

**Settings View:**

- [ ] Indexer management (add/remove/edit)
- [ ] TMDB API key input
- [ ] Player preferences

### Navigation

- [ ] Keyboard shortcuts
  - Search: Enter query, Esc to cancel
  - Browse: Up/Down arrows, Enter to select, / to search again
  - Streaming: q to stop, space to pause/resume (if mpv allows)
  - Global: ? for help, Ctrl+C to exit
- [ ] Tab between views

## resource management

- [ ] delete temp files on exit
- [ ] stop torrents on exit
- [ ] shutdown server gracefully
- [ ] kill mpv process if still running

### Error Handling

- [ ] no indexers configured -> prompt to add
- [ ] no search results -> helpful message
- [ ] torrent fails to start -> fallback to next best seeded
- [ ] tmdb rate limiting -> graceful degradation
- [ ] network errors -> retry logic

## UX

- [ ] first-run setup wizard
- [ ] config validation with helpful errors
- [ ] loading states everywhere
- [ ] responsive layout (handle small terminals)

# Future

## Subtitles

- [ ] Research OpenSubtitles alternatives (20/day limit is tight)
- [ ] Auto-fetch subtitles for selected content
- [ ] Pass subtitle file to mpv

## Multi-Episode Handling

- [ ] Detect season packs
- [ ] Pre-download next episode while watching current
- [ ] Playlist mode for binge-watching

## Advanced Features

- [ ] Resume playback from position
- [ ] Watch history
- [ ] Favorites/watchlist
- [ ] Multiple quality selection (720p vs 1080p)
- [ ] Jackett/Prowlarr integration (meta-indexer support)

## References

- Torznab spec: https://torznab.github.io/spec-1.3-draft/torznab/Specification-v1.3.html
- torznab-toolkit: https://github.com/askiiart/torznab-toolkit
- TMDB API: https://developer.themoviedb.org/docs/getting-started
- librqbit: https://github.com/ikatson/rqbit + https://docs.rs/librqbit/
