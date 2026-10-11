use base64::Engine;
use chilpa_pg_playlist_import::{broker::Host, service::Service};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
async fn phase(service: &Arc<Service>, wanted: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = service.handle("import.status", json!({})).await.unwrap();
            if value["phase"] == wanted {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("workflow did not finish")
}
#[tokio::test]
async fn public_file_to_local_remote_and_best_quality_mixed_preserves_order_and_reports_missing() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
    let host = Host::new(tx);
    let service = Service::new(host.clone());
    let writes = Arc::new(Mutex::new(Vec::<Value>::new()));
    let recorded = writes.clone();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            let request: Value = serde_json::from_str(&line).unwrap();
            let p = &request["params"];
            let input = &p["params"];
            let result = match p["method"].as_str().unwrap() {
                "sources.list" => {
                    json!({"sources":[{"id":"a","writable":true,"playlist_limit":1},{"id":"b","writable":true}]})
                }
                "library.search" | "catalog.search" => {
                    let source = input["source"].as_str().unwrap();
                    let query = input["text"].as_str().unwrap();
                    let title = if query.contains("First") {
                        "First"
                    } else if query.contains("Second") {
                        "Second"
                    } else {
                        "Unrelated"
                    };
                    json!({"tracks":[{"kind":"track","source":source,"ref":format!("{source}:{title}"),"title":title,"artist":"Band","available":true,"quality":{"codec":"flac","bits":if source=="b"{24}else{16},"rate":if source=="b"{96000}else{44100}}}]})
                }
                "playlists.create" => {
                    recorded.lock().unwrap().push(input.clone());
                    json!({"ref":format!("playlist:{}",recorded.lock().unwrap().len())})
                }
                "playlists.add" => {
                    recorded.lock().unwrap().push(input.clone());
                    json!({"added":input["tracks"].as_array().unwrap().len()})
                }
                other => panic!("unexpected {other}"),
            };
            host.reply(json!({"id":p["id"],"result":result})).await;
        }
    });
    let bytes=base64::engine::general_purpose::STANDARD.encode(b"#EXTM3U\n#EXTINF:180,Band - First\nfirst.flac\n#EXTINF:180,Band - Second\nsecond.flac\n#EXTINF:180,Band - First\nfirst.flac\n#EXTINF:180,Band - Missing\nmissing.flac\n");
    service
        .handle(
            "import.preview",
            json!({"input":"file","filename":"test.m3u","bytes":bytes}),
        )
        .await
        .unwrap();
    assert_eq!(phase(&service, "preview").await["preview"]["total"], 4);
    service
        .handle(
            "import.match",
            json!({"targets":["local","a","b","mixed"],"title":"Imported","folder":7}),
        )
        .await
        .unwrap();
    let ready = phase(&service, "ready").await;
    assert_eq!(ready["rows"][0]["matches"]["mixed"]["source"], "b");
    assert_eq!(ready["rows"][3]["status"], "unmatched");
    assert!(writes.lock().unwrap().is_empty(), "review must not write");
    assert!(service.handle("import.commit", json!({})).await.is_err());
    service
        .handle("import.commit", json!({"grant":"test"}))
        .await
        .unwrap();
    let done = phase(&service, "done").await;
    assert_eq!(done["outputs"].as_array().unwrap().len(), 4);
    for output in done["outputs"].as_array().unwrap() {
        assert_eq!(output["written"], 2);
        assert_eq!(output["duplicates"], 1);
        assert_eq!(output["unmatched"], 1);
        assert_eq!(output["error"], "");
    }
    assert_eq!(
        done["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["target"] == "a")
            .unwrap()["parts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let writes = writes.lock().unwrap();
    let mixed = writes.iter().find(|w| w["target"] == "mixed").unwrap();
    assert_eq!(mixed["tracks"][0]["title"], "First");
    assert_eq!(mixed["tracks"][1]["title"], "Second");
    assert_eq!(mixed["folder"], 7);
}
