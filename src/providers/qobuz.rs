//! Public playlist metadata, independent of the playback plugin and its account.
use crate::{
    errors::PlaylistImportError as Error,
    models::{ImportPlaylist, ImportProvider, ImportTrack},
};
use serde_json::Value;

pub fn parse_playlist_id(input: &str) -> Option<String> {
    let url = reqwest::Url::parse(input.trim()).ok()?;
    if !matches!(
        url.host_str(),
        Some("qobuz.com" | "www.qobuz.com" | "open.qobuz.com" | "play.qobuz.com")
    ) {
        return None;
    }
    let parts: Vec<_> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let id = if parts.first() == Some(&"playlist") {
        *parts.get(1)?
    } else if parts.iter().any(|p| *p == "playlists") {
        *parts.last()?
    } else {
        return None;
    };
    (!id.is_empty() && id.bytes().all(|c| c.is_ascii_digit())).then(|| id.into())
}
fn map(item: &Value) -> Option<ImportTrack> {
    Some(ImportTrack {
        title: item["title"].as_str()?.into(),
        artist: item["performer"]["name"]
            .as_str()
            .or_else(|| item["album"]["artist"]["name"].as_str())
            .unwrap_or_default()
            .into(),
        album: item["album"]["title"].as_str().map(str::to_owned),
        duration_ms: item["duration"].as_u64().map(|s| s.saturating_mul(1000)),
        isrc: item["isrc"].as_str().map(str::to_owned),
        provider_id: item["id"].as_u64().map(|n| n.to_string()),
        provider_url: None,
    })
}
async fn body(url: &str) -> Result<String, Error> {
    let mut response = crate::http::http()
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Http(e.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::Http(format!(
            "Qobuz public playlist returned HTTP {}. It may be private or unavailable.",
            response.status().as_u16()
        )));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Error::Http(e.to_string()))?
    {
        if data.len() + chunk.len() > 16 * 1024 * 1024 {
            return Err(Error::FileTooLarge);
        }
        data.extend_from_slice(&chunk);
    }
    String::from_utf8(data).map_err(|_| Error::Parse("Invalid public playlist text".into()))
}
pub async fn fetch_playlist(id: &str) -> Result<ImportPlaylist, Error> {
    let html = body(&format!(
        "https://www.qobuz.com/us-en/playlists/playlist/{id}"
    ))
    .await?;
    // The public preview includes an application identifier, not an account token.
    // I only retain that identifier and never request audio or application secrets.
    let app = html
        .split("app_id=")
        .skip(1)
        .find_map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            (!digits.is_empty() && digits.len() <= 32).then_some(digits)
        })
        .ok_or_else(|| Error::Parse("Qobuz public playlist metadata is unavailable.".into()))?;
    let mut result = ImportPlaylist {
        provider: ImportProvider::Qobuz,
        provider_id: id.into(),
        name: "Qobuz playlist".into(),
        description: None,
        tracks: Vec::new(),
    };
    let mut offset = 0;
    loop {
        let mut url =
            reqwest::Url::parse("https://www.qobuz.com/api.json/0.2/playlist/get").unwrap();
        url.query_pairs_mut()
            .append_pair("playlist_id", id)
            .append_pair("app_id", &app)
            .append_pair("extra", "tracks")
            .append_pair("limit", "500")
            .append_pair("offset", &offset.to_string());
        let page: Value = serde_json::from_str(&body(url.as_str()).await?)
            .map_err(|e| Error::Parse(e.to_string()))?;
        result.name = page["name"].as_str().unwrap_or(&result.name).into();
        result.description = page["description"].as_str().map(str::to_owned);
        let rows = page["tracks"]["items"]
            .as_array()
            .ok_or_else(|| Error::Parse("The public playlist has no track list.".into()))?;
        result.tracks.extend(rows.iter().filter_map(map));
        offset += rows.len();
        let total = page["tracks"]["total"]
            .as_u64()
            .or_else(|| page["tracks_count"].as_u64())
            .unwrap_or(offset as u64) as usize;
        if offset >= total {
            break;
        }
        if rows.is_empty() || offset >= 10000 {
            return Err(Error::Parse(
                "The public playlist could not be read completely.".into(),
            ));
        }
    }
    if result.tracks.is_empty() {
        return Err(Error::EmptyPlaylist);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_urls_and_lookalike_hosts() {
        for link in [
            "https://open.qobuz.com/playlist/123",
            "https://www.qobuz.com/us-en/playlists/name/123?x=1",
        ] {
            assert_eq!(parse_playlist_id(link).as_deref(), Some("123"));
        }
        assert!(parse_playlist_id("https://qobuz.com.evil.example/playlist/123").is_none());
        assert!(parse_playlist_id("https://open.qobuz.com/album/123").is_none());
    }
    #[test]
    fn metadata_keeps_recording_identity_and_duration() {
        let track=map(&serde_json::json!({"id":1,"title":"Track","performer":{"name":"Artist"},"duration":123,"isrc":"EXAMPLE","album":{"title":"Record"}})).unwrap();
        assert_eq!(track.artist, "Artist");
        assert_eq!(track.duration_ms, Some(123000));
        assert_eq!(track.isrc.as_deref(), Some("EXAMPLE"));
    }
}
