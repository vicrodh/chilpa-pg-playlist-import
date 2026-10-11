//! A preview survives matching and writing; the source is fetched only once.
use crate::{broker::Host, models::ImportPlaylist};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;
mod read;
mod workflow;
#[derive(Default)]
pub(super) struct State {
    phase: String,
    error: String,
    preview: Option<ImportPlaylist>,
    targets: Vec<String>,
    sources: Vec<Value>,
    rows: Vec<Value>,
    outputs: Vec<Value>,
    warnings: Vec<String>,
    log: Vec<String>,
    config: Value,
    completed: usize,
    total_work: usize,
    matched: usize,
    current: String,
    cancel: Arc<AtomicBool>,
    generation: u64,
}
impl State {
    fn busy(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "fetching" | "matching" | "creating" | "adding"
        )
    }
    fn note(&mut self, text: String) {
        self.log.push(text);
        if self.log.len() > 200 {
            self.log.remove(0);
        }
    }
    fn snapshot(&self, input: &Value) -> Value {
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let filter = input["filter"].as_str().unwrap_or("all");
        let rows: Vec<_> = self
            .rows
            .iter()
            .filter(|row| match filter {
                "unmatched" => self
                    .targets
                    .iter()
                    .any(|target| row["matches"][target].is_null()),
                "error" => row["errors"]
                    .as_object()
                    .is_some_and(|errors| !errors.is_empty()),
                _ => true,
            })
            .collect();
        let preview=self.preview.as_ref().map(|p|json!({"name":p.name,"description":p.description,"provider":p.provider.as_str(),"total":p.tracks.len()}));
        json!({"phase":if self.phase.is_empty(){"input"}else{&self.phase},"busy":self.busy(),"error":self.error,"preview":preview,
            "sources":self.sources,"targets":self.targets,"settings":{"title":self.config["title"],"description":self.config["description"],"public":self.config["public"],"folder":self.config["folder"]},"rows":rows.iter().skip(offset).take(200).collect::<Vec<_>>(),"row_count":rows.len(),"offset":offset,
            "outputs":self.outputs,"warnings":self.warnings,"log":self.log,"completed":self.completed,"total_work":self.total_work,"matched":self.matched,"current":self.current})
    }
}
pub struct Service {
    host: Host,
    state: Mutex<State>,
}
impl Service {
    pub fn new(host: Host) -> Arc<Self> {
        Arc::new(Self {
            host,
            state: Mutex::new(State::default()),
        })
    }
    pub async fn handle(self: &Arc<Self>, method: &str, input: Value) -> Result<Value, String> {
        match method {
            "initialize" => Ok(json!({"protocol":1,"capabilities":{"playlist_import":1}})),
            "import.options" => Ok(read::options()),
            // What a link of another service is, for the host's "Open music
            // link": the kind, the title and the artist, or null.
            "link.identify" => Ok(crate::identify::identify(input["url"].as_str().unwrap_or_default()).await),
            "import.choices" => read::choices(&input).await,
            "import.status" => Ok(self.state.lock().await.snapshot(&input)),
            "import.preview" => {
                let generation = {
                    let mut state = self.state.lock().await;
                    if state.busy() {
                        return Err("An import is already running.".into());
                    }
                    let generation = state.generation + 1;
                    *state = State {
                        phase: "fetching".into(),
                        generation,
                        ..Default::default()
                    };
                    generation
                };
                let service = self.clone();
                tokio::spawn(async move {
                    let result = read::resolve(&input).await;
                    let mut state = service.state.lock().await;
                    if state.generation != generation {
                        return;
                    }
                    match result {
                        Ok(preview) => {
                            state.rows=preview.tracks.iter().enumerate().map(|(i,track)|json!({"index":i+1,"original":track,"status":"pending","matches":{},"errors":{}})).collect();
                            state.warnings = read::warnings(&preview);
                            state.preview = Some(preview);
                            state.phase = "preview".into();
                        }
                        Err(error) => {
                            state.error = error;
                            state.phase = "input".into();
                        }
                    }
                });
                Ok(json!({"started":true}))
            }
            "import.match" => {
                let sources = self.host.call("sources.list", json!({})).await?["sources"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                {
                    let mut state = self.state.lock().await;
                    if state.busy() {
                        return Err("An import is already running.".into());
                    }
                    if state.preview.is_none() {
                        return Err("Read a playlist first.".into());
                    }
                    let mut targets: Vec<String> = input["targets"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect();
                    targets.sort();
                    targets.dedup();
                    if targets.is_empty() || targets.len() > sources.len() + 2 {
                        return Err("Select at least one destination.".into());
                    }
                    for target in &targets {
                        if !matches!(target.as_str(), "local" | "mixed")
                            && !sources
                                .iter()
                                .any(|s| s["id"] == *target && s["writable"] == true)
                        {
                            return Err(
                                "A selected destination is not connected or writable.".into()
                            );
                        }
                    }
                    state.targets = targets;
                    state.sources = sources;
                    state.config = input;
                    state.phase = "matching".into();
                    state.error.clear();
                    state.outputs.clear();
                    state.completed = 0;
                    state.total_work = state.rows.len();
                    state.matched = 0;
                    state.cancel = Arc::new(AtomicBool::new(false));
                    state.note("Matching tracks against the selected catalogs.".into());
                }
                let service = self.clone();
                tokio::spawn(async move {
                    workflow::match_tracks(service).await;
                });
                Ok(json!({"started":true}))
            }
            "import.commit" => {
                {
                    let mut state = self.state.lock().await;
                    if state.phase != "ready" {
                        return Err("Match and review the tracks before creating playlists.".into());
                    }
                    if input["grant"].as_str().is_none_or(|v| v.is_empty()) {
                        return Err("Chilpa did not authorize these destinations.".into());
                    }
                    for key in ["title", "description", "public", "folder"] {
                        if let Some(value) = input.get(key) {
                            state.config[key] = value.clone();
                        }
                    }
                    state.config["grant"] = input["grant"].clone();
                    state.phase = "creating".into();
                    state.completed = 0;
                    state.error.clear();
                    state.cancel = Arc::new(AtomicBool::new(false));
                }
                let service = self.clone();
                tokio::spawn(async move {
                    workflow::write(service).await;
                });
                Ok(json!({"started":true}))
            }
            "import.cancel" => {
                let mut state = self.state.lock().await;
                state.cancel.store(true, Ordering::Release);
                if state.phase == "fetching" {
                    state.generation += 1;
                    state.phase = "cancelled".into();
                } else if !state.busy() {
                    state.phase = "cancelled".into();
                }
                state.note("Cancellation requested. Already created playlists are kept.".into());
                Ok(json!({"cancelled":true}))
            }
            "import.reset" => {
                let mut state = self.state.lock().await;
                if state.busy() {
                    return Err("Wait for the running operation or cancel it.".into());
                }
                let generation = state.generation + 1;
                *state = State {
                    generation,
                    ..Default::default()
                };
                Ok(json!({}))
            }
            _ => Err("Unknown importer method.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_missing_list_keeps_tracks_whose_catalog_request_failed() {
        let state = State {
            targets: vec!["a".into()],
            rows: vec![json!({"status":"error","matches":{},"errors":{"a":"Disconnected"}})],
            ..Default::default()
        };
        let missing = state.snapshot(&json!({"filter":"unmatched"}));
        assert_eq!(missing["rows"].as_array().unwrap().len(), 1);
        assert_eq!(missing["rows"][0]["errors"]["a"], "Disconnected");
        assert_eq!(state.snapshot(&json!({"filter":"error"}))["row_count"], 1);
    }
}
