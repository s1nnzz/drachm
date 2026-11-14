//! Calls of the intrinsic functions of the embedded standard modules (`stdlib.rs`).
//!
//! The resolver rewrites `fs::read(p)` to a call of `fs::read`, a function declared
//! `intrinsic` in `src/stdlib/fs.drh`; this is where such a call is typed. `math` has
//! its own file (`math.rs`); `fs`, `os` and `time` are here.

use super::{failures, optionals, Checker, Val};
use crate::ast;
use crate::diag::{codes, Diagnostic, Span};
use crate::hir;
use crate::types::Type;

impl Checker<'_> {
    /// A call of the intrinsic `global` (`module::function`).
    pub(super) fn intrinsic_call(&mut self, global: &str, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        let (module, name) = global.split_once("::").unwrap_or(("", global));
        match module {
            "fs" | "os" => self.system_call(module, name, targs, args, span),
            "time" => self.time_call(name, targs, args, span),
            "math" => self.math_call(name, targs, args, span, None),
            _ => {
                self.diags.push(Diagnostic::at(&codes::E0901, span, format!("`{global}` is not implemented yet")));
                for a in args {
                    self.expr(a);
                }
                None
            }
        }
    }

    /// E0206 for `module::name(...)` where the file did not import the standard module
    /// (only reached when the resolver did not already report it: a one-file program).
    pub(super) fn module_not_imported(&mut self, module: &str, module_span: Span, args: &[ast::Expr]) {
        self.diags.push(
            Diagnostic::at(&codes::E0206, module_span, format!("module `{module}` is used but not imported"))
                .note("not imported")
                .help(format!("add `in {module};` at the top of the file: every file imports what it uses")),
        );
        for a in args {
            self.expr(a);
        }
    }

    fn system_call(&mut self, module: &str, name: &str, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        use hir::SystemOp::*;
        let op = match (module, name) {
            ("fs", "read") => Read,
            ("fs", "write") => Write,
            ("os", "platform") => Platform,
            ("os", "getEnv") => GetEnv,
            _ => {
                self.diags.push(Diagnostic::at(&codes::E0201, span, format!("module `{module}` has no function `{name}`")));
                return None;
            }
        };
        if args.len() != op.inputs() || !targs.is_empty() {
            self.diags.push(Diagnostic::at(&codes::E0103, span, format!("`{}` takes {} string arguments and no type arguments", op.name(), op.inputs())));
            return None;
        }
        let args = args.iter().map(|arg| self.coerce(arg, Type::Str)).collect::<Option<Vec<_>>>()?;
        let mut fails = Vec::new();
        let ty = match op {
            Read | Write => {
                let names = if op == Read { &["FileMissing", "AccessDenied", "InvalidPath", "InvalidFileText", "FileIoError"][..] }
                    else { &["FileMissing", "AccessDenied", "InvalidPath", "FileIoError"][..] };
                fails.extend(names.iter().map(|name| self.g.enums_by_name[*name]));
                failures::outcome_type(self.g, op.fs_result(), &fails);
                op.fs_result()
            }
            Platform => Type::Str,
            CText => optionals::optional_of(self.g, Type::Str)?,
            GetEnv => optionals::optional_of(self.g, Type::Str)?,
        };
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::System(op, args, fails), ty).at(span)))
    }

    fn time_call(&mut self, name: &str, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        if name != "sleep" {
            self.diags.push(Diagnostic::at(&codes::E0201, span, format!("module `time` has no function `{name}`")));
            return None;
        }
        if let Some(arg) = targs.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, arg.span, "`time::sleep` takes no type arguments"));
        }
        let [duration] = args else {
            self.diags.push(Diagnostic::at(&codes::E0103, span, "`time::sleep` takes one duration"));
            return None;
        };
        let duration = self.coerce(duration, super::units::seconds_type())?;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Sleep(Box::new(duration)), Type::None).at(span)))
    }
}
