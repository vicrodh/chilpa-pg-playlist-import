//! Plugin-side host calls. Replies share the existing JSON-RPC pipe without blocking its reader.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{Mutex, mpsc, oneshot};
#[derive(Clone)]
pub struct Host {
    out: mpsc::Sender<String>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>,
    next: Arc<AtomicU64>,
}
impl Host {
    pub fn new(out: mpsc::Sender<String>) -> Self {
        Self {
            out,
            pending: Default::default(),
            next: Arc::new(AtomicU64::new(1)),
        }
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = format!("host-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        if self.out.send(json!({"jsonrpc":"2.0","method":"host.request","params":{"id":id,"method":method,"params":params}}).to_string()).await.is_err() { self.pending.lock().await.remove(&id);return Err("Chilpa disconnected.".into()); }
        let answer = tokio::time::timeout(std::time::Duration::from_secs(45), rx).await;
        self.pending.lock().await.remove(&id);
        answer
            .map_err(|_| "Chilpa did not answer in time; a write may have completed.".to_string())?
            .map_err(|_| "Chilpa disconnected.".to_string())?
    }
    pub async fn reply(&self, params: Value) {
        if let Some(tx) = self
            .pending
            .lock()
            .await
            .remove(params["id"].as_str().unwrap_or_default())
        {
            let _ = tx.send(if let Some(error) = params["error"].as_str() {
                Err(error.into())
            } else {
                Ok(params["result"].clone())
            });
        }
    }
}
