//! Inlay hints: the inferred `self` mode of each method, shown right after its
//! signature (`sint32 get() &self {`). The mode is worked out by the checker's
//! ownership pass (`ownership::infer`), so the hint is only there when the
//! program checks.

use std::path::Path;

use lsp_types::{InlayHint, InlayHintLabel, Position, Range};

use super::analysis::Analysis;
use super::pos::to_position;

fn key(p: Position) -> (u32, u32) {
    (p.line, p.character)
}

impl Analysis {
    /// The hints of the methods of `file` whose hint lies in `range`.
    pub fn self_hints(&self, file: &Path, range: Range) -> Vec<InlayHint> {
        let Some(id) = self.file_id(file) else { return Vec::new() };
        let src = self.source_map().source(id);
        self.self_modes
            .iter()
            .filter_map(|&(sig, mutable)| {
                let (f, local) = self.in_program(sig)?;
                if f != id {
                    return None;
                }
                let position = to_position(src, local.end);
                if key(position) < key(range.start) || key(position) > key(range.end) {
                    return None;
                }
                Some(InlayHint {
                    position,
                    label: InlayHintLabel::String(if mutable { "&mut self" } else { "&self" }.to_string()),
                    kind: None,
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(true),
                    padding_right: None,
                    data: None,
                })
            })
            .collect()
    }
}
