# Chilpa Playlist Importer

I read public playlists and playlist files, match their tracks against connected Chilpa catalogs, and create one playlist per selected destination.

Inputs: Spotify, Apple Music, Tidal, Deezer and Qobuz public links; M3U/M3U8, PLS and XSPF files; structural JSON exports; ListenBrainz playlists and “created for you”; Last.fm playlists and Library/Mix/Recommendations stations. The readers and file parsing tests originate in QBZ (MIT; see NOTICE).

Qobuz input works without the Qobuz playback plugin. Public origins do not borrow credentials from playback plugins. Spotify's public embed can expose only the first 50 tracks: I display a warning and recommend a file/JSON export for a larger complete playlist. Generated stations and structural JSON imports also show their limitations before matching.

Destinations are independent vertical checkboxes: local files only, each writable source, or a mixed local playlist using the best known quality among equally confident recording matches. Matching preserves input order, excludes explicitly unavailable recordings, deduplicates selected references, and distinguishes missing results from failed catalog requests. Remote playlists split at the destination limit; creation is preceded by a review step. The final summary keeps unmatched tracks and partial write errors.

The plugin requests catalog-read, local-library-read and scoped playlist-write permissions. The host mediates every cross-plugin request; account storage, credentials and audio URLs never cross this boundary. See the [plugin cooperation contract](https://github.com/vicrodh/chilpa-plugins-sdk/blob/main/docs/plugin-cooperation.md) for the protocol.

Install the ZIP through Chilpa Settings → Plugins → Install from ZIP, read and accept its permissions, then open Playlists → Import playlist. This plugin requires a Chilpa build with the cooperation broker.

Build/package: `tools/build-native`. Automated checks: Cargo unit and workflow tests, `tools/test-ui ../code`, and `tools/smoke-native <packaged-plugin>`. The host has broker permission, local transaction, and sandboxed cross-plugin integration tests.

The Tidal public playlist reader obtains an application token from the declared
public QBZ proxy endpoint. It does not embed a token in this repository.
