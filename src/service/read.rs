use crate::{
    models::ImportPlaylist,
    sources::{
        self, LastFmStation, PlaylistSource,
        service::{
            lastfm::{self, LastFmTarget},
            listenbrainz::{self, LbTarget},
        },
    },
};
use base64::Engine;
use serde_json::{Value, json};
pub(super) fn options() -> Value {
    json!({"inputs":[
        {"id":"url","name":"Playlist link","hint":"Spotify, Apple Music, Tidal, Deezer or Qobuz public playlist URL","kind":"text"},
        {"id":"file","name":"Playlist file","hint":"M3U, M3U8, PLS or XSPF. Only the track list is read; files named inside it are never opened.","kind":"file"},
        {"id":"json","name":"JSON","hint":"Best-effort detection of track lists in JSON exports. Review the count before continuing.","kind":"file"},
        {"id":"listenbrainz","name":"ListenBrainz","hint":"Playlist URL, MBID or username for playlists created for you","kind":"choices","private_token":true},
        {"id":"lastfm","name":"Last.fm","hint":"Username, profile URL or public playlist URL","kind":"station","choices":[{"id":0,"name":"Library"},{"id":1,"name":"Mix"},{"id":2,"name":"Recommendations"}]}
    ]})
}
pub(super) async fn choices(input: &Value) -> Result<Value, String> {
    let text = input["text"].as_str().unwrap_or_default();
    let Some(LbTarget::User(user)) = listenbrainz::detect(text) else {
        return Ok(json!({"choices":[]}));
    };
    let rows =
        listenbrainz::list_created_for(&user, input["token"].as_str().filter(|s| !s.is_empty()))
            .await
            .map_err(|e| e.to_string())?;
    Ok(
        json!({"choices":rows.into_iter().map(|r|json!({"id":r.mbid,"name":r.title})).collect::<Vec<_>>()}),
    )
}
pub(super) async fn resolve(input: &Value) -> Result<ImportPlaylist, String> {
    let text = input["text"].as_str().unwrap_or_default();
    let source = match input["input"].as_str().unwrap_or("url") {
        "url" => PlaylistSource::Url(text.into()),
        "file" | "json" => {
            let encoded = input["bytes"].as_str().ok_or("Choose a file first.")?;
            if encoded.len() > 24 * 1024 * 1024 {
                return Err("The selected file is too large.".into());
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| "The selected file could not be read.")?;
            let filename = input["filename"].as_str().unwrap_or("playlist").to_string();
            if input["input"] == "json" {
                PlaylistSource::Json { bytes, filename }
            } else {
                let format =
                    sources::file::detect_format(&bytes, &filename).map_err(|e| e.to_string())?;
                PlaylistSource::File {
                    format,
                    bytes,
                    filename,
                }
            }
        }
        "listenbrainz" => {
            let selected = input["choice"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(text);
            let Some(LbTarget::Mbid(mbid)) = listenbrainz::detect(selected) else {
                return Err(
                    "Choose a playlist created for this user, or paste a playlist URL.".into(),
                );
            };
            PlaylistSource::ListenBrainz {
                mbid,
                token: input["token"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            }
        }
        "lastfm" => match lastfm::detect(text) {
            Some(LastFmTarget::Profile { user }) => PlaylistSource::LastFmStation {
                user,
                station: LastFmStation::from_index(input["station"].as_i64().unwrap_or(0) as i32),
            },
            Some(LastFmTarget::Playlist { user, id }) => PlaylistSource::LastFmPlaylist {
                user,
                playlist_id: id,
            },
            None => return Err("Enter a username or a public playlist URL.".into()),
        },
        _ => return Err("Unknown playlist input.".into()),
    };
    let preview = source.resolve().await.map_err(|e| e.to_string())?;
    if preview.tracks.is_empty() {
        return Err("The playlist has no recognizable tracks.".into());
    }
    if preview.tracks.len() > 10000 {
        return Err("Import at most 10000 tracks at a time.".into());
    }
    Ok(preview)
}
pub(super) fn warnings(preview: &ImportPlaylist) -> Vec<String> {
    use crate::models::ImportProvider;
    match preview.provider {
        ImportProvider::Spotify=>vec!["Spotify's public embed may expose only the first 50 tracks. Review the count; use a playlist file or JSON export for a complete larger list.".into()],
        ImportProvider::LastFm=>vec!["Station recommendations are a generated selection, not a full listening history.".into()],
        ImportProvider::Json=>vec!["JSON is read by structure. Check the titles and track count before matching.".into()],
        _=>Vec::new()
    }
}
