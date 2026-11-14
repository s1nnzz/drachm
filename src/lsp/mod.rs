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
mod complete;
mod hints;
mod pos;
mod query;
mod refs;

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
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
        references_provider: Some(lsp_types::OneOf::Left(true)),
        rename_provider: Some(lsp_types::OneOf::Left(true)),
        inlay_hint_provider: Some(lsp_types::OneOf::Left(true)),
        completion_provider: Some(lsp_types::CompletionOptions { trigger_characters: Some(vec![":".into(), ">".into(), ".".into()]), ..Default::default() }),
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
        let id = req.id.clone();
        let answer = match req.method.as_str() {
            "textDocument/hover" => self.hover(req.params),
            "textDocument/definition" => self.definition(req.params),
            "textDocument/references" => self.references(req.params),
            "textDocument/inlayHint" => self.inlay_hints(req.params),
            "textDocument/rename" => match self.rename(req.params) {
                Ok(edit) => edit,
                // The reason is shown to the user as it is.
                Err(why) => {
                    let err = Response::new_err(id, lsp_server::ErrorCode::RequestFailed as i32, why);
                    let _ = self.conn.sender.send(Message::Response(err));
                    return;
                }
            },
            "textDocument/completion" => self.completion(req.params),
            m => {
                let err = Response::new_err(id, lsp_server::ErrorCode::MethodNotFound as i32, format!("unsupported request {m}"));
                let _ = self.conn.sender.send(Message::Response(err));
                return;
            }
        };
        let _ = self.conn.sender.send(Message::Response(Response::new_ok(id, answer)));
    }

    /// The analysis holding the file `uri` names, the file, and the cursor's byte offset in it.
    fn locate(&self, pos: &lsp_types::TextDocumentPositionParams) -> Option<(&Analysis, PathBuf, usize)> {
        let path = pos::uri_to_path(&pos.text_document.uri)?;
        let a = self.analyses.get(self.root_of.get(&path)?)?;
        let id = a.file_id(&path)?;
        let offset = pos::to_offset(a.source_map().source(id), pos.position);
        Some((a, path, offset))
    }

    fn hover(&self, params: serde_json::Value) -> serde_json::Value {
        let Ok(p) = serde_json::from_value::<lsp_types::HoverParams>(params) else { return serde_json::Value::Null };
        let Some((a, path, offset)) = self.locate(&p.text_document_position_params) else { return serde_json::Value::Null };
        let Some(h) = a.hover(&path, offset) else { return serde_json::Value::Null };
        let src = a.source_map().source(a.file_id(&path).unwrap());
        let hover = lsp_types::Hover {
            contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent { kind: lsp_types::MarkupKind::Markdown, value: h.markdown }),
            range: Some(pos::to_range(src, h.span)),
        };
        serde_json::to_value(hover).unwrap_or_default()
    }

    fn completion(&self, params: serde_json::Value) -> serde_json::Value {
        let Ok(p) = serde_json::from_value::<lsp_types::CompletionParams>(params) else { return serde_json::Value::Null };
        let pos = &p.text_document_position;
        let Some(path) = pos::uri_to_path(&pos.text_document.uri) else { return serde_json::Value::Null };
        let Some(text) = self.docs.get(&path) else { return serde_json::Value::Null };
        let offset = pos::to_offset(&crate::diag::Source::new("", text.as_str()), pos.position);
        let docs = self.docs.clone();
        let items = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Analysis::complete_at(&path, &docs, offset))).unwrap_or_default();
        let items: Vec<lsp_types::CompletionItem> = items
            .into_iter()
            .map(|i| lsp_types::CompletionItem {
                sort_text: Some(format!("{}{}", i.sort, i.label)),
                label: i.label,
                kind: Some(i.kind),
                detail: (!i.detail.is_empty()).then_some(i.detail),
                ..Default::default()
            })
            .collect();
        serde_json::to_value(items).unwrap_or_default()
    }

    fn references(&self, params: serde_json::Value) -> serde_json::Value {
        let Ok(p) = serde_json::from_value::<lsp_types::ReferenceParams>(params) else { return serde_json::Value::Null };
        let Some((a, path, offset)) = self.locate(&p.text_document_position) else { return serde_json::Value::Null };
        serde_json::to_value(a.references(&path, offset, p.context.include_declaration)).unwrap_or_default()
    }

    /// The workspace edit for a rename, or why it cannot be done.
    fn rename(&self, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let p = serde_json::from_value::<lsp_types::RenameParams>(params).map_err(|e| format!("bad rename request: {e}"))?;
        let (a, path, offset) = self.locate(&p.text_document_position).ok_or("this file is not part of an analysed program")?;
        let changes = a.rename(&path, offset, &p.new_name)?;
        let edit = lsp_types::WorkspaceEdit { changes: Some(changes), ..Default::default() };
        Ok(serde_json::to_value(edit).unwrap_or_default())
    }

    fn inlay_hints(&self, params: serde_json::Value) -> serde_json::Value {
        let Ok(p) = serde_json::from_value::<lsp_types::InlayHintParams>(params) else { return serde_json::Value::Null };
        let Some(path) = pos::uri_to_path(&p.text_document.uri) else { return serde_json::Value::Null };
        let Some(a) = self.root_of.get(&path).and_then(|r| self.analyses.get(r)) else { return serde_json::Value::Null };
        serde_json::to_value(a.self_hints(&path, p.range)).unwrap_or_default()
    }

    fn definition(&self, params: serde_json::Value) -> serde_json::Value {
        let Ok(p) = serde_json::from_value::<lsp_types::GotoDefinitionParams>(params) else { return serde_json::Value::Null };
        let Some((a, path, offset)) = self.locate(&p.text_document_position_params) else { return serde_json::Value::Null };
        let Some(place) = a.definition(&path, offset) else { return serde_json::Value::Null };
        let Some(uri) = a.files.get(place.file).and_then(|f| pos::path_to_uri(f)) else { return serde_json::Value::Null };
        let loc = lsp_types::Location { uri, range: pos::to_range(a.source_map().source(place.file), place.span) };
        serde_json::to_value(loc).unwrap_or_default()
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
