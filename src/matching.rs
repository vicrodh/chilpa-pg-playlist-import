//! Recording confidence ranks before quality. The playlist's original order plays no part in ranking.
use crate::models::ImportTrack;
use serde_json::Value;
pub const MIN_SCORE: f64 = 0.65;
fn normalized(text: &str) -> String {
    let mut depth = 0usize;
    let mut clean = String::new();
    for c in text.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => {
                if c.is_alphanumeric() {
                    clean.extend(c.to_lowercase())
                } else {
                    clean.push(' ')
                }
            }
            _ => {}
        }
    }
    clean
        .split_whitespace()
        .filter(|word| {
            !matches!(
                *word,
                "remaster"
                    | "remastered"
                    | "deluxe"
                    | "edition"
                    | "feat"
                    | "featuring"
                    | "version"
                    | "mono"
                    | "stereo"
                    | "edit"
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn similarity(a: &str, b: &str) -> f64 {
    let a = normalized(a);
    let b = normalized(b);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    if a.contains(&b) || b.contains(&a) {
        return 0.85;
    }
    let a: Vec<_> = a.split_whitespace().collect();
    let b: Vec<_> = b.split_whitespace().collect();
    a.iter().filter(|word| b.contains(word)).count() as f64 / a.len().max(b.len()) as f64
}
pub fn score(source: &ImportTrack, candidate: &Value) -> f64 {
    if candidate["available"] == false {
        return 0.0;
    }
    if let (Some(a), Some(b)) = (source.isrc.as_deref(), candidate["isrc"].as_str()) {
        if !a.is_empty() && a.eq_ignore_ascii_case(b) {
            return 1.0;
        }
    }
    let title = candidate["title"].as_str().unwrap_or_default();
    // A live recording or remix must not win just because it has more bits.
    for marker in ["live", "remix", "karaoke", "instrumental"] {
        if source.title.to_lowercase().contains(marker) != title.to_lowercase().contains(marker) {
            return 0.0;
        }
    }
    let mut value = similarity(&source.title, title) * 0.6
        + similarity(
            &source.artist,
            candidate["artist"].as_str().unwrap_or_default(),
        ) * 0.3;
    if let Some(album) = &source.album {
        value += similarity(album, candidate["album"].as_str().unwrap_or_default()) * 0.1;
    }
    if let (Some(a), Some(b)) = (
        source.duration_ms,
        candidate["duration_ms"].as_u64().filter(|n| *n > 0),
    ) {
        let diff = a.abs_diff(b);
        if diff <= 3000 {
            value += 0.05
        } else if diff <= 5000 {
            value += 0.02
        }
    }
    value.min(1.0)
}
pub fn quality(candidate: &Value) -> (u8, u64, u64) {
    let q = &candidate["quality"];
    let codec = q["codec"].as_str().unwrap_or_default().to_ascii_lowercase();
    let bits = q["bits"].as_u64().unwrap_or(0);
    let rate = q["rate"].as_u64().unwrap_or(0);
    let tier = if matches!(codec.as_str(), "dsf" | "dff" | "dsd") {
        4
    } else if q["tier"] == "hi_res" || bits > 16 || rate > 48000 {
        3
    } else if matches!(codec.as_str(), "flac" | "alac" | "wav" | "aiff" | "pcm") {
        2
    } else if codec.is_empty() {
        0
    } else {
        1
    };
    (tier, bits, rate)
}
pub fn best(source: &ImportTrack, candidates: impl IntoIterator<Item = Value>) -> Option<Value> {
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .filter_map(|mut c| {
            let confidence = score(source, &c);
            if confidence < MIN_SCORE {
                return None;
            }
            c["confidence"] = serde_json::json!(confidence);
            Some(c)
        })
        .collect();
    ranked.sort_by_key(|c| {
        std::cmp::Reverse((
            (c["confidence"].as_f64().unwrap_or(0.0) * 100.0).round() as u16,
            quality(c),
            c["source"] == "local",
        ))
    });
    ranked.into_iter().next()
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn original() -> ImportTrack {
        ImportTrack {
            title: "Song".into(),
            artist: "Band".into(),
            album: None,
            duration_ms: Some(180000),
            isrc: Some("ABC".into()),
            provider_id: None,
            provider_url: None,
        }
    }
    #[test]
    fn quality_wins_between_the_same_recording_without_choosing_a_live_version() {
        let base = json!({"ref":"track:1","source":"a","title":"Song","artist":"Band","duration_ms":180000,"isrc":"ABC","quality":{"codec":"flac","bits":16,"rate":44100}});
        let mut hires = base.clone();
        hires["ref"] = json!("track:2");
        hires["quality"]["bits"] = json!(24);
        let mut live = hires.clone();
        live["title"] = json!("Song (Live)");
        live["isrc"] = json!("OTHER");
        live["quality"]["rate"] = json!(192000);
        assert_eq!(
            best(&original(), [base, hires, live]).unwrap()["ref"],
            "track:2"
        );
    }
    #[test]
    fn unavailable_and_unrelated_tracks_do_not_match() {
        assert!(
            best(
                &original(),
                [
                    json!({"title":"Different","artist":"Someone"}),
                    json!({"title":"Song","artist":"Band","isrc":"ABC","available":false})
                ]
            )
            .is_none()
        );
    }
    #[test]
    fn missing_availability_is_not_a_rejection_and_unicode_survives() {
        assert_eq!(normalized("Björk"), "björk");
        assert!(best(&original(), [json!({"title":"Song","artist":"Band"})]).is_some());
    }
}
