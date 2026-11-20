//! Canned answers in a fixed order, for tests that need an exact sequence
//! (polling limits, timeouts) rather than a bank.

use std::sync::{Arc, Mutex};

use tokio::task::JoinHandle;

use crate::http::{Responder, listen};
use crate::request::latin1;

/// Canned bank messages, sent in order whatever the request; the last one
/// repeats. For tests that need an exact sequence of answers.
pub struct Script {
    responses: Vec<Vec<u8>>,
}

struct ScriptResponder {
    responses: Vec<Vec<u8>>,
    next: usize,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Responder for ScriptResponder {
    fn respond(&mut self, message: &[u8], _: Option<String>) -> Result<Vec<u8>, String> {
        self.requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(latin1(message));
        let i = self.next.min(self.responses.len().saturating_sub(1));
        self.next += 1;
        self.responses
            .get(i)
            .cloned()
            .ok_or_else(|| "the script has no responses".to_string())
    }
}

impl Script {
    /// Answers in this order (exact bytes, before base64).
    pub fn new(responses: Vec<Vec<u8>>) -> Self {
        Self { responses }
    }

    /// Serves the script on 127.0.0.1 (free port).
    pub async fn serve(self) -> HttpScript {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (url, task) = listen(ScriptResponder {
            responses: self.responses,
            next: 0,
            requests: Arc::clone(&requests),
        })
        .await;
        HttpScript {
            url,
            requests,
            task,
        }
    }
}

/// A served [`Script`]; the server stops when this is dropped.
pub struct HttpScript {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl HttpScript {
    /// The FinTS URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The requests so far, decoded, as ISO 8859-1 text.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for HttpScript {
    fn drop(&mut self) {
        self.task.abort();
    }
}
