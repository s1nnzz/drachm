//! The language server: `drachm lsp` speaks the Language Server Protocol on
//! stdio. It reuses the compiler front end in memory (the open buffers stand in
//! for the files on disk), so there are no temporary files.
//!
//! Engineering choice: `lsp-server` (the synchronous message loop of
//! rust-analyzer) with `lsp-types`, not `tower-lsp`. The front end is a
//! plain blocking library, a program is small, and one thread that analyses
//! and then answers keeps every answer consistent with the text it was made
//! from; an async runtime would only add dependencies.

mod analysis;
mod pos;

pub use analysis::Analysis;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::notification::{DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument, Notification as _, PublishDiagnostics};
use lsp_types::{PublishDiagnosticsParams, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions};

/// How long after the last keystroke the diagnostics are refreshed.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Run the server until the client says goodbye. The exit code.
pub fn run() -> i32 {
    let (connection, io) = Connection::stdio();
    let caps = serde_json::to_value(capabilities()).expect("capabilities are json");
    if connection.initialize(caps).is_err() {
        return 1;
    }
    let mut server = Server::new(&connection);
    let code = server.main_loop();
    drop(server);
    drop(connection);
    let _ = io.join();
    code
}

fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::FULL),
            save: Some(lsp_types::TextDocumentSyncSaveOptions::Supported(true)),
            ..Default::default()
        })),
        ..Default::default()
    }
}

struct Server<'a> {
    conn: &'a Connection,
    /// The open buffers, by path.
    docs: HashMap<PathBuf, String>,
    /// The latest analysis of each program an open buffer belongs to, by entry file.
    analyses: HashMap<PathBuf, Analysis>,
    /// Which program each open buffer was last analysed in.
    root_of: HashMap<PathBuf, PathBuf>,
    /// When the buffers changed and the diagnostics are not refreshed yet.
    dirty_since: Option<Instant>,
    /// Files that have diagnostics published, by program.
    published: HashMap<PathBuf, HashSet<PathBuf>>,
}

impl<'a> Server<'a> {
    fn new(conn: &'a Connection) -> Self {
        Server { conn, docs: HashMap::new(), analyses: HashMap::new(), root_of: HashMap::new(), dirty_since: None, published: HashMap::new() }
    }

    fn main_loop(&mut self) -> i32 {
        loop {
            let msg = match self.dirty_since {
                Some(t) => {
                    let wait = DEBOUNCE.saturating_sub(t.elapsed());
                    match self.conn.receiver.recv_timeout(wait) {
                        Ok(m) => m,
                        Err(e) if e.is_timeout() => {
                            self.refresh();
                            continue;
                        }
                        Err(_) => return 0,
                    }
                }
                None => match self.conn.receiver.recv() {
                    Ok(m) => m,
                    Err(_) => return 0,
                },
            };
            match msg {
                Message::Request(req) => {
                    if self.conn.handle_shutdown(&req).unwrap_or(true) {
                        return 0;
                    }
                    self.request(req);
                }
                Message::Notification(n) => self.notification(n),
                Message::Response(_) => {}
            }
        }
    }

    fn notification(&mut self, n: Notification) {
        match n.method.as_str() {
            DidOpenTextDocument::METHOD => {
                if let Ok(p) = serde_json::from_value::<lsp_types::DidOpenTextDocumentParams>(n.params)
                    && let Some(path) = pos::uri_to_path(&p.text_document.uri)
                {
                    self.docs.insert(path, p.text_document.text);
                    self.refresh();
                }
            }
            DidChangeTextDocument::METHOD => {
                if let Ok(p) = serde_json::from_value::<lsp_types::DidChangeTextDocumentParams>(n.params)
                    && let Some(path) = pos::uri_to_path(&p.text_document.uri)
                    && let Some(change) = p.content_changes.into_iter().last()
                {
                    self.docs.insert(path, change.text);
                    self.dirty_since.get_or_insert_with(Instant::now);
                    // Each keystroke pushes the refresh back.
                    self.dirty_since = Some(Instant::now());
                }
            }
            DidSaveTextDocument::METHOD => {
                if self.dirty_since.is_some() {
                    self.refresh();
                }
            }
            DidCloseTextDocument::METHOD => {
                if let Ok(p) = serde_json::from_value::<lsp_types::DidCloseTextDocumentParams>(n.params)
                    && let Some(path) = pos::uri_to_path(&p.text_document.uri)
                {
                    self.docs.remove(&path);
                    if let Some(root) = self.root_of.remove(&path)
                        && !self.root_of.values().any(|r| *r == root)
                    {
                        self.analyses.remove(&root);
                        for f in self.published.remove(&root).unwrap_or_default() {
                            self.publish(&f, Vec::new());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn request(&mut self, req: Request) {
        self.flush();
        let _ = self.conn.sender.send(Message::Response(Response::new_err(req.id, lsp_server::ErrorCode::MethodNotFound as i32, format!("unsupported request {}", req.method))));
    }

    /// Bring the analyses up to date if the buffers changed.
    fn flush(&mut self) {
        if self.dirty_since.is_some() {
            self.refresh();
        }
    }

    /// Re-check every program an open buffer belongs to, and publish the diagnostics.
    fn refresh(&mut self) {
        self.dirty_since = None;
        let mut paths: Vec<PathBuf> = self.docs.keys().cloned().collect();
        paths.sort();
        let mut done: HashSet<PathBuf> = HashSet::new();
        self.root_of.clear();
        let mut fresh: HashMap<PathBuf, Analysis> = HashMap::new();
        for path in paths {
            // A buffer in a program already analysed this round needs no run of its own.
            if let Some(root) = fresh.iter().find(|(_, a)| a.file_id(&path).is_some()).map(|(r, _)| r.clone()) {
                self.root_of.insert(path, root);
                continue;
            }
            let docs = self.docs.clone();
            let file = path.clone();
            // The compiler must never take the server down.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Analysis::run(&file, &docs)));
            match result {
                Ok(a) => {
                    done.insert(a.entry.clone());
                    self.root_of.insert(path, a.entry.clone());
                    fresh.insert(a.entry.clone(), a);
                }
                Err(_) => eprintln!("drachm lsp: the checker panicked on {}", file.display()),
            }
        }
        for (root, a) in &fresh {
            let mut now = HashSet::new();
            for (file, diags) in a.lsp_diagnostics() {
                now.insert(file.clone());
                self.publish(&file, diags);
            }
            for gone in self.published.get(root).into_iter().flatten().filter(|f| !now.contains(*f)) {
                self.publish(gone, Vec::new());
            }
            self.published.insert(root.clone(), now);
        }
        self.analyses = fresh;
    }

    fn publish(&self, file: &std::path::Path, diagnostics: Vec<lsp_types::Diagnostic>) {
        let Some(uri) = pos::path_to_uri(file) else { return };
        self.notify::<PublishDiagnostics>(PublishDiagnosticsParams { uri, diagnostics, version: None });
    }

    fn notify<N: lsp_types::notification::Notification>(&self, params: N::Params) {
        let n = Notification::new(N::METHOD.to_string(), params);
        let _ = self.conn.sender.send(Message::Notification(n));
    }
}

#[cfg(test)]
mod tests;
