# Chilpa Playlist Importer

Reads a public playlist or a playlist file, matches its tracks against your
library and connected sources, and creates one playlist in each destination
you pick.

## Inputs

- Public links from Spotify, Apple Music, Tidal, Deezer and Qobuz.
- M3U/M3U8, PLS and XSPF files, and structural JSON exports.
- ListenBrainz playlists and "created for you" lists.
- Last.fm playlists and Library, Mix and Recommendations stations.

The readers and their parsing tests come from QBZ (MIT; see NOTICE).

Public links need no playback plugin and borrow no credentials from one.
Spotify's public embed shows only the first 50 tracks; past that, the
importer warns you and suggests a file or JSON export. Stations and JSON
imports also show their limits before matching. The Tidal reader gets an
application token from QBZ's public proxy; no token is stored in this
repository.

## Destinations

Local files only, any writable source, or a mixed local playlist that picks
the best known quality among equally confident matches. Matching keeps the
playlist's order, skips recordings marked unavailable, removes duplicates,
and tells a missing track apart from a failed catalog request. Remote
playlists are split at the destination's size limit. You review the matches
before anything is created, and the final summary lists unmatched tracks and
partial write errors.

## Permissions

The plugin asks for catalog-read, local-library-read and scoped
playlist-write. Every request to another plugin goes through Chilpa; account
storage, credentials and audio URLs never cross that boundary. The protocol
is in the [plugin cooperation contract](https://github.com/vicrodh/chilpa-plugins-sdk/blob/main/docs/plugin-cooperation.md).

## Install

Packages are available for Linux x86_64 and macOS Apple Silicon. The macOS package
needs the updated Chilpa host with its native worker sandbox; earlier macOS
builds cannot launch process plugins. It also needs the cooperation broker. In Chilpa, open
Settings → Plugins → Install from ZIP, review and accept the permissions,
then go to Playlists → Import playlist.

## Build and test

```sh
tools/build-native                       # build and package
cargo test                               # unit and workflow tests
tools/test-ui ../code                    # Chilpa's import screen, offscreen
tools/smoke-native <packaged-plugin>     # the packaged process, no real account
```

Chilpa's own tests cover broker permissions, local transactions and
sandboxed cross-plugin calls.
