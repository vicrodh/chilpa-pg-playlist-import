use chilpa_pg_playlist_import::{broker::Host, service::Service};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[tokio::main(worker_threads = 4)]
async fn main() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let (out, mut receiver) = tokio::sync::mpsc::channel::<String>(256);
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(line) = receiver.recv().await {
            if stdout
                .write_all(format!("{line}\n").as_bytes())
                .await
                .is_err()
            {
                break;
            }
            let _ = stdout.flush().await;
        }
    });
    let host = Host::new(out.clone());
    let service = Service::new(host.clone());
    let mut input = BufReader::new(tokio::io::stdin());
    loop {
        let mut bytes = Vec::new();
        // Limit allocation even for malformed peers without a newline.
        loop {
            let chunk = match input.fill_buf().await {
                Ok(c) => c,
                Err(_) => return,
            };
            if chunk.is_empty() {
                return;
            }
            let n = chunk
                .iter()
                .position(|b| *b == b'\n')
                .map(|n| n + 1)
                .unwrap_or(chunk.len());
            if bytes.len() + n > 32 * 1024 * 1024 {
                return;
            }
            let done = chunk[n - 1] == b'\n';
            bytes.extend_from_slice(&chunk[..n]);
            input.consume(n);
            if done {
                break;
            }
        }
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if value["method"] == "host.reply" {
            host.reply(value["params"].clone()).await;
            continue;
        }
        let service = service.clone();
        let out = out.clone();
        tokio::spawn(async move {
            let result = service
                .handle(
                    value["method"].as_str().unwrap_or_default(),
                    value["params"].clone(),
                )
                .await;
            if value.get("id").is_none() {
                return;
            }
            let reply = match result {
                Ok(result) => json!({"jsonrpc":"2.0","id":value["id"],"result":result}),
                Err(error) => {
                    json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32000,"message":error}})
                }
            };
            let _ = out.send(reply.to_string()).await;
        });
    }
}
