use super::*;
use crate::matching;
use futures_util::{StreamExt, stream};
use std::collections::{BTreeMap, HashSet};

pub(super) async fn match_tracks(service: Arc<Service>) {
    let (tracks, targets, sources, cancel) = {
        let s = service.state.lock().await;
        (
            s.preview.as_ref().unwrap().tracks.clone(),
            s.targets.clone(),
            s.sources.clone(),
            s.cancel.clone(),
        )
    };
    let mut catalogs: Vec<String> = targets
        .iter()
        .filter(|t| t.as_str() != "mixed")
        .cloned()
        .collect();
    if targets.iter().any(|t| t == "mixed") {
        catalogs.push("local".into());
        catalogs.extend(
            sources
                .iter()
                .filter_map(|s| s["id"].as_str().map(str::to_owned)),
        );
    }
    catalogs.sort();
    catalogs.dedup();
    let jobs = tracks.into_iter().enumerate().map(|(index, original)| {
        let service=service.clone(); let catalogs=catalogs.clone(); let targets=targets.clone(); let cancel=cancel.clone();
        async move {
            if cancel.load(Ordering::Acquire) { return; }
            let mut found=BTreeMap::new(); let mut errors=serde_json::Map::new();
            for source in catalogs {
                if cancel.load(Ordering::Acquire) { return; }
                let method=if source=="local" {"library.search"} else {"catalog.search"};
                let answer=service.host.call(method,json!({"source":source,"text":format!("{} {}",original.artist,original.title),"isrc":original.isrc,"limit":20})).await;
                match answer {
                    Ok(answer)=>{found.insert(source, answer["tracks"].as_array().cloned().unwrap_or_default());},
                    Err(error)=>{errors.insert(source,json!(error));}
                }
            }
            let mut matches=serde_json::Map::new();
            for target in &targets {
                let candidates=if target=="mixed" {found.values().flatten().cloned().collect()} else {found.get(target).cloned().unwrap_or_default()};
                if let Some(best)=matching::best(&original,candidates) {matches.insert(target.clone(),best);}
            }
            let status=if !errors.is_empty(){"error"}else if matches.len()==targets.len(){"matched"}else{"unmatched"};
            let mut s=service.state.lock().await;
            s.rows[index]=json!({"index":index+1,"original":original,"matches":matches,"errors":errors,"status":status});
            s.completed+=1; if !matches.is_empty(){s.matched+=1;}
            s.current=format!("{} — {}",original.artist,original.title);
        }
    });
    stream::iter(jobs)
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
    let mut s = service.state.lock().await;
    s.phase = if cancel.load(Ordering::Acquire) {
        "cancelled"
    } else {
        "ready"
    }
    .into();
    s.note(
        "Matching finished. Review missing tracks and catalog errors before creating playlists."
            .into(),
    );
}

fn selections(rows: &[Value], target: &str) -> (Vec<Value>, usize, usize) {
    let mut seen = HashSet::new();
    let mut selected = Vec::new();
    let mut duplicates = 0;
    let mut missing = 0;
    for row in rows {
        let candidate = &row["matches"][target];
        if candidate.is_null() {
            missing += 1;
            continue;
        }
        let key = (
            candidate["source"].as_str().unwrap_or_default().to_owned(),
            candidate["ref"].as_str().unwrap_or_default().to_owned(),
        );
        if seen.insert(key) {
            selected.push(candidate.clone());
        } else {
            duplicates += 1;
        }
    }
    (selected, duplicates, missing)
}
async fn publish(service: &Service, output: &Value) {
    let mut state = service.state.lock().await;
    if let Some(slot) = state
        .outputs
        .iter_mut()
        .find(|o| o["target"] == output["target"])
    {
        *slot = output.clone();
    } else {
        state.outputs.push(output.clone());
    }
    state.completed = state
        .outputs
        .iter()
        .filter_map(|o| o["written"].as_u64())
        .sum::<u64>() as usize;
    state.current = output["parts"]
        .as_array()
        .and_then(|parts| parts.last())
        .and_then(|p| p["title"].as_str())
        .unwrap_or_default()
        .into();
}

pub(super) async fn write(service: Arc<Service>) {
    let (targets, rows, sources, config, cancel, name) = {
        let s = service.state.lock().await;
        (
            s.targets.clone(),
            s.rows.clone(),
            s.sources.clone(),
            s.config.clone(),
            s.cancel.clone(),
            s.preview.as_ref().unwrap().name.clone(),
        )
    };
    service.state.lock().await.total_work = targets
        .iter()
        .map(|target| selections(&rows, target).0.len())
        .sum();
    for target in targets {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let (selected, duplicates, missing) = selections(&rows, &target);
        let local = matches!(target.as_str(), "local" | "mixed");
        let limit = if local {
            10000
        } else {
            sources
                .iter()
                .find(|s| s["id"] == target)
                .and_then(|s| s["playlist_limit"].as_u64())
                .unwrap_or(2000)
                .clamp(1, 10000) as usize
        };
        let title = config["title"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&name);
        let mut output = json!({"target":target,"total":rows.len(),"matched":selected.len(),"duplicates":duplicates,"unmatched":missing,"written":0,"parts":[],"error":""});
        publish(&service, &output).await;
        for (part, tracks) in selected.chunks(limit).enumerate() {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            let title = if selected.len() > limit {
                format!("{title} — Part {}", part + 1)
            } else {
                title.to_string()
            };
            let created=service.host.call("playlists.create",json!({"grant":config["grant"],"target":target,"title":title,"description":config["description"],"public":config["public"],"folder":config["folder"],"tracks":if local{json!(tracks)}else{Value::Null}})).await;
            let created = match created {
                Ok(c) => c,
                Err(e) => {
                    output["error"] = json!(e);
                    break;
                }
            };
            let reference = created["ref"].clone();
            output["parts"].as_array_mut().unwrap().push(json!({"ref":reference,"title":title,"written":if local{tracks.len()}else{0},"warning":created["warning"]}));
            publish(&service, &output).await;
            if local {
                output["written"] =
                    json!(output["written"].as_u64().unwrap() + tracks.len() as u64);
            } else {
                service.state.lock().await.phase = "adding".into();
                for batch in tracks.chunks(50) {
                    if cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let result=service.host.call("playlists.add",json!({"grant":config["grant"],"target":target,"ref":reference,"tracks":batch.iter().map(|t|t["ref"].clone()).collect::<Vec<_>>()})).await;
                    match result {
                        Ok(_) => {
                            output["written"] =
                                json!(output["written"].as_u64().unwrap() + batch.len() as u64);
                            let p = output["parts"].as_array_mut().unwrap().last_mut().unwrap();
                            p["written"] =
                                json!(p["written"].as_u64().unwrap() + batch.len() as u64);
                        }
                        Err(e) => {
                            output["error"] = json!(e);
                            break;
                        }
                    }
                    publish(&service, &output).await;
                }
            }
            publish(&service, &output).await;
            if output["error"].as_str().is_some_and(|s| !s.is_empty()) {
                break;
            }
        }
        publish(&service, &output).await;
    }
    let mut s = service.state.lock().await;
    s.phase = if cancel.load(Ordering::Acquire) {
        "cancelled"
    } else {
        "done"
    }
    .into();
    s.note("Import finished. The summary distinguishes written tracks, missing matches and duplicates.".into());
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_are_disjoint_and_order_is_preserved() {
        let rows = vec![
            json!({"matches":{"a":{"source":"a","ref":"2"}}}),
            json!({"matches":{"a":{"source":"a","ref":"2"}}}),
            json!({"matches":{}}),
            json!({"matches":{"a":{"source":"a","ref":"1"}}}),
        ];
        let (tracks, duplicates, missing) = selections(&rows, "a");
        assert_eq!(tracks[0]["ref"], "2");
        assert_eq!(tracks[1]["ref"], "1");
        assert_eq!((tracks.len(), duplicates, missing), (2, 1, 1));
        assert_eq!(tracks.len() + duplicates + missing, rows.len());
    }
}
