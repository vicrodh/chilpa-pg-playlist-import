//! What a link of another service is, for Chilpa's "Open music link": its
//! kind, title and artist, asked of the service itself. This is QBZ's fast
//! path (qbz-music-link/src/fast_path.rs): Deezer's public API, Spotify's
//! embed page and TIDAL's API with the proxy's app token. Apple Music had no
//! direct path in QBZ (it went to Odesli, whose keyless API was retired on
//! 2026-07-31); here it asks the public iTunes lookup. A playlist is said as
//! one, and the host sends it to the importer.

use serde_json::{json, Value};

use crate::http::{http, USER_AGENT};
use crate::providers::{self, MusicProvider, MusicResource};

/// The answer to `link.identify`: `{kind, title, artist, album}` (the
/// album of a track, where the service says it), `{kind: "playlist",
/// provider}`, or null for a link this plugin does not know or could not
/// read.
pub async fn identify(url: &str) -> Value {
    match providers::detect_music_resource(url) {
        Some(MusicResource::Playlist { provider }) => json!({"kind": "playlist", "provider": provider_name(&provider)}),
        Some(MusicResource::Track { provider, url }) => described("track", metadata(&provider, &url, true).await),
        Some(MusicResource::Album { provider, url }) => described("album", metadata(&provider, &url, false).await),
        _ => Value::Null,
    }
}

/// What a service says of one link: the title, the artist and, for a
/// track, its album when it says it.
type Said = (String, String, String);

fn described(kind: &str, found: Option<Said>) -> Value {
    match found {
        Some((title, artist, album)) if !title.trim().is_empty() => {
            json!({"kind": kind, "title": title.trim(), "artist": artist.trim(), "album": album.trim()})
        }
        _ => Value::Null,
    }
}

fn provider_name(provider: &MusicProvider) -> &'static str {
    match provider {
        MusicProvider::Spotify => "spotify",
        MusicProvider::AppleMusic => "apple",
        MusicProvider::Tidal => "tidal",
        MusicProvider::Deezer => "deezer",
    }
}

async fn metadata(provider: &MusicProvider, url: &str, track: bool) -> Option<Said> {
    match provider {
        MusicProvider::Deezer => deezer(url, track).await,
        MusicProvider::Spotify => spotify(url, track).await,
        MusicProvider::Tidal => tidal(url, track).await,
        MusicProvider::AppleMusic => apple(url, track).await,
    }
}

/// The id after `/<entity>/` in a link, up to `?`, `/` or `#`.
fn entity_id(url: &str, entity: &str) -> Option<String> {
    let pattern = format!("/{entity}/");
    let rest = &url[url.find(&pattern)? + pattern.len()..];
    let id = rest.split(['?', '/', '#']).next()?;
    (!id.is_empty()).then(|| id.to_string())
}

async fn deezer(url: &str, track: bool) -> Option<Said> {
    let entity = if track { "track" } else { "album" };
    let id = entity_id(url, entity)?;
    let data: Value = http().get(format!("https://api.deezer.com/{entity}/{id}")).send().await.ok()?.json().await.ok()?;
    if data.get("error").is_some() {
        return None;
    }
    let title = data.get("title")?.as_str()?.to_string();
    let artist = data.pointer("/artist/name").and_then(Value::as_str).unwrap_or("").to_string();
    let album = data.pointer("/album/title").and_then(Value::as_str).unwrap_or("").to_string();
    Some((title, artist, album))
}

async fn spotify(url: &str, track: bool) -> Option<Said> {
    let entity = if track { "track" } else { "album" };
    let id = url
        .strip_prefix(&format!("spotify:{entity}:"))
        .and_then(|rest| rest.split(['?', '/']).next())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| entity_id(url, entity))?;
    let (title, artist) = providers::spotify::fetch_embed_metadata(entity, &id).await?;
    Some((title, artist, String::new()))
}

async fn tidal(url: &str, track: bool) -> Option<Said> {
    let entity = if track { "track" } else { "album" };
    let id = entity_id(url, entity)?;
    let token = providers::tidal::app_token().await.ok()?;
    let data: Value = http()
        .get(format!("https://openapi.tidal.com/v2/{entity}s/{id}?countryCode=US&include=artists,albums"))
        .header("Authorization", format!("Bearer {token}"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let title = data.pointer("/data/attributes/title")?.as_str()?.to_string();
    let included = |kind: &str, field: &str| {
        data.get("included")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|item| item["type"] == kind))
            .and_then(|item| item.pointer(&format!("/attributes/{field}")))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    Some((title, included("artists", "name"), if track { included("albums", "title") } else { String::new() }))
}

/// An Apple Music link's id: the song's (`?i=` or `/song/<name>/<id>`) for a
/// track, the album's last path segment otherwise.
fn apple_id(url: &str, track: bool) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if track {
        if let Some((_, id)) = parsed.query_pairs().find(|(key, _)| key == "i") {
            return Some(id.into_owned()).filter(|id| id.chars().all(|c| c.is_ascii_digit()));
        }
    }
    let last = parsed.path_segments()?.filter(|segment| !segment.is_empty()).last()?.to_string();
    last.chars().all(|c| c.is_ascii_digit()).then_some(last)
}

async fn apple(url: &str, track: bool) -> Option<Said> {
    let id = apple_id(url, track)?;
    let data: Value = http().get(format!("https://itunes.apple.com/lookup?id={id}")).send().await.ok()?.json().await.ok()?;
    let found = data.get("results")?.as_array()?.first()?;
    let title = if track { found.get("trackName") } else { found.get("collectionName") }?.as_str()?.to_string();
    let artist = found.get("artistName").and_then(Value::as_str).unwrap_or("").to_string();
    let album = if track { found.get("collectionName").and_then(Value::as_str).unwrap_or("").to_string() } else { String::new() };
    Some((title, artist, album))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entity_id_is_the_segment_after_its_name() {
        assert_eq!(entity_id("https://www.deezer.com/en/album/302127?utm=x", "album").as_deref(), Some("302127"));
        assert_eq!(entity_id("https://tidal.com/browse/track/12345/u", "track").as_deref(), Some("12345"));
        assert_eq!(entity_id("https://tidal.com/browse/album/", "album"), None);
        assert_eq!(entity_id("https://example.org/x", "album"), None);
    }

    #[test]
    fn an_apple_link_names_the_song_or_the_album() {
        let album = "https://music.apple.com/us/album/discovery/697194953";
        let song = "https://music.apple.com/us/album/discovery/697194953?i=697195462";
        assert_eq!(apple_id(album, false).as_deref(), Some("697194953"));
        assert_eq!(apple_id(song, true).as_deref(), Some("697195462"));
        assert_eq!(apple_id("https://music.apple.com/us/song/one-more-time/697195462", true).as_deref(), Some("697195462"));
        assert_eq!(apple_id("https://music.apple.com/us/album/discovery/x", false), None);
    }

    #[test]
    fn a_found_link_needs_a_title() {
        let said = described("track", Some(("One More Time".into(), "Daft Punk".into(), "Discovery".into())));
        assert_eq!((said["title"].as_str(), said["album"].as_str()), (Some("One More Time"), Some("Discovery")));
        assert!(described("album", Some(("  ".into(), "x".into(), String::new()))).is_null());
        assert!(described("track", None).is_null());
    }
}
