//! C header discovery (step 7.1): libclang reads a header and everything it
//! includes, and this module turns what it finds into plain Rust data.
//!
//! `in * from c."SDL2/SDL.h" as sdl;` parses a synthetic C file that only
//! includes the header, with the include directories and definitions of the
//! project's `[native]` table. Every declaration of the header and its
//! transitive includes is recorded under its C name. A declaration the
//! compiler cannot use yet is recorded as `Decl::Unsupported` with the
//! reason, so that using it reports why (E0901) instead of "no such name".
//!
//! libclang is loaded at run time, only when a C import is checked: a program
//! without one never needs it.

#![allow(non_upper_case_globals)]

use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::Mutex;

use clang_sys::*;

use crate::types::CKind;

/// Why discovery failed, before any declaration could be read.
#[derive(Debug, Clone)]
pub enum Failure {
    /// libclang could not be loaded.
    NoLibclang(String),
    /// The header (or a header it needs) was not found.
    NotFound(String),
    /// The header has C errors; the text is clang's.
    Parse(String),
}

/// The settings of the `[native]` table that discovery depends on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Settings {
    pub include_dirs: Vec<PathBuf>,
    pub defines: Vec<String>,
}

/// A C type, reduced to what the compiler maps.
#[derive(Debug, Clone, PartialEq)]
pub enum CType {
    Void,
    Scalar(CKind),
    /// `char`, `unsigned char` and `signed char` behind a pointer: bytes.
    Ptr(Box<CType>),
    /// A struct (or union) by its name in `CUnit::decls`.
    Record(String),
    /// An enum by its name in `CUnit::decls`.
    Enum(String),
    Array(Box<CType>, u64),
    FuncPtr(Box<CSig>),
    /// A type the compiler cannot map yet, with the reason.
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CSig {
    pub ret: CType,
    pub params: Vec<(String, CType)>,
    pub variadic: bool,
}

#[derive(Debug, Clone)]
pub struct CField {
    pub name: String,
    pub ty: CType,
}

#[derive(Debug, Clone)]
pub struct CRecord {
    pub name: String,
    pub is_union: bool,
    /// An incomplete type (`struct SDL_Window;`): only pointers to it exist.
    pub opaque: bool,
    pub fields: Vec<CField>,
    pub size: u64,
    /// Why the fields cannot be mapped (bit-fields, a flexible array, a
    /// layout that is not the natural one).
    pub unsupported: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CEnum {
    pub name: String,
    pub underlying: CKind,
    pub constants: Vec<(String, i128)>,
}

/// A macro that is not function-like: its tokens, evaluated on demand.
#[derive(Debug, Clone)]
pub struct CMacro {
    pub tokens: Vec<String>,
    pub function_like: bool,
}

/// The value of an object-like macro or an enumerator.
#[derive(Debug, Clone, PartialEq)]
pub enum CValue {
    Int(i128),
    Float(f64),
    Str(String),
}

#[derive(Debug, Clone)]
pub enum Decl {
    Func { sig: CSig, inline: bool },
    Record(CRecord),
    Enum(CEnum),
    /// An enumerator: its value and the enum it belongs to.
    Enumerator { value: i128, of: String },
    Macro(CMacro),
    /// A typedef to something that is not a struct or enum of its own name.
    Alias(CType),
    Unsupported(String),
}

/// What discovery found in one header.
#[derive(Debug, Default)]
pub struct CUnit {
    pub header: String,
    pub settings: Settings,
    pub decls: BTreeMap<String, Decl>,
    /// The C source that parses the unit again (macro evaluation, shims).
    pub source: String,
}

/// Load libclang on this thread (the binding keeps one library per thread).
fn load() -> Result<(), Failure> {
    if clang_sys::is_loaded() {
        return Ok(());
    }
    clang_sys::load().map(|_| ()).map_err(|e| Failure::NoLibclang(e.to_string()))
}

static RESOURCE_DIR: Mutex<Option<Option<String>>> = Mutex::new(None);

/// The folder with clang's own headers (`stddef.h`, `stdint.h`). libclang does
/// not always find it by itself, so it is looked for: `DRACHM_CLANG_RESOURCE_DIR`,
/// then `clang -print-resource-dir`, then the usual install folders.
fn resource_dir() -> Option<String> {
    let mut cache = RESOURCE_DIR.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = cache.as_ref() {
        return found.clone();
    }
    let has_headers = |d: &std::path::Path| d.join("include/stddef.h").is_file();
    let mut found = std::env::var("DRACHM_CLANG_RESOURCE_DIR").ok().filter(|d| has_headers(std::path::Path::new(d)));
    if found.is_none() {
        found = std::process::Command::new("clang")
            .arg("-print-resource-dir")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|d| has_headers(std::path::Path::new(d)));
    }
    if found.is_none() {
        for base in ["/usr/lib/clang", "/usr/lib64/clang", "/usr/local/lib/clang", "/opt/homebrew/opt/llvm/lib/clang"] {
            let Ok(entries) = std::fs::read_dir(base) else { continue };
            let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| has_headers(p)).collect();
            dirs.sort();
            if let Some(d) = dirs.pop() {
                found = Some(d.display().to_string());
                break;
            }
        }
    }
    *cache = Some(found.clone());
    found
}

unsafe fn text(s: CXString) -> String {
    unsafe {
        let p = clang_getCString(s);
        let out = if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() };
        clang_disposeString(s);
        out
    }
}

/// The arguments clang is given for a unit.
fn clang_args(settings: &Settings) -> Vec<CString> {
    let mut args = vec!["-x".to_string(), "c".to_string(), "-std=gnu11".to_string()];
    if let Some(dir) = resource_dir() {
        args.push(format!("-resource-dir={dir}"));
    }
    for d in &settings.include_dirs {
        args.push(format!("-I{}", d.display()));
    }
    for d in &settings.defines {
        args.push(format!("-D{d}"));
    }
    args.into_iter().map(|a| CString::new(a).unwrap()).collect()
}

/// The synthetic C file for `header`: `#include <header>`.
pub fn synthetic_source(header: &str) -> String {
    format!("#include <{header}>\n")
}

/// A parsed translation unit that disposes itself.
struct Unit {
    index: CXIndex,
    tu: CXTranslationUnit,
}

impl Drop for Unit {
    fn drop(&mut self) {
        unsafe {
            clang_disposeTranslationUnit(self.tu);
            clang_disposeIndex(self.index);
        }
    }
}

const SYNTHETIC_NAME: &str = "drachm-c-import.c";

fn parse(source: &str, settings: &Settings, detailed: bool) -> Result<Unit, Failure> {
    load()?;
    unsafe {
        let index = clang_createIndex(0, 0);
        let args = clang_args(settings);
        let arg_ptrs: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
        let name = CString::new(SYNTHETIC_NAME).unwrap();
        let src = CString::new(source).map_err(|_| Failure::Parse("the header name has a NUL byte".into()))?;
        let mut unsaved = [CXUnsavedFile { Filename: name.as_ptr(), Contents: src.as_ptr(), Length: source.len() as _ }];
        let mut tu: CXTranslationUnit = std::ptr::null_mut();
        let flags = if detailed { CXTranslationUnit_DetailedPreprocessingRecord } else { 0 };
        let err = clang_parseTranslationUnit2(
            index,
            name.as_ptr(),
            arg_ptrs.as_ptr(),
            arg_ptrs.len() as c_int,
            unsaved.as_mut_ptr(),
            1,
            flags,
            &mut tu,
        );
        if err != CXError_Success || tu.is_null() {
            clang_disposeIndex(index);
            return Err(Failure::Parse(format!("libclang could not start (error {err})")));
        }
        Ok(Unit { index, tu })
    }
}

/// Errors clang reported, as one text; `None` when there are none. A missing
/// file is reported first as `NotFound`.
fn errors(unit: &Unit, header: &str) -> Option<Failure> {
    unsafe {
        let n = clang_getNumDiagnostics(unit.tu);
        let mut lines = Vec::new();
        let mut not_found = false;
        for i in 0..n {
            let d = clang_getDiagnostic(unit.tu, i);
            let sev = clang_getDiagnosticSeverity(d);
            if sev >= CXDiagnostic_Error {
                let msg = text(clang_getDiagnosticSpelling(d));
                if msg.contains("file not found") {
                    not_found = true;
                }
                let loc = clang_getDiagnosticLocation(d);
                let (mut file, mut line, mut col) = (std::ptr::null_mut(), 0, 0);
                clang_getSpellingLocation(loc, &mut file, &mut line, &mut col, std::ptr::null_mut());
                let at = if file.is_null() { String::new() } else { format!("{}:{line}:{col}: ", text(clang_getFileName(file))) };
                lines.push(format!("{at}{msg}"));
            }
            clang_disposeDiagnostic(d);
        }
        if lines.is_empty() {
            None
        } else if not_found && lines.iter().any(|l| l.contains(header)) {
            Some(Failure::NotFound(lines.join("\n")))
        } else {
            Some(Failure::Parse(lines.join("\n")))
        }
    }
}

/// Read `header` with `settings`.
pub fn discover(header: &str, settings: &Settings) -> Result<CUnit, Failure> {
    let source = synthetic_source(header);
    let unit = parse(&source, settings, true)?;
    if let Some(f) = errors(&unit, header) {
        // Recoverable errors (one bad declaration) still leave the rest of the
        // header usable, but a header that is missing is not.
        if matches!(f, Failure::NotFound(_)) || !has_declarations(&unit) {
            return Err(f);
        }
    }
    let mut out = CUnit { header: header.to_string(), settings: settings.clone(), decls: BTreeMap::new(), source };
    unsafe {
        let root = clang_getTranslationUnitCursor(unit.tu);
        let mut ctx = Visit { decls: &mut out.decls };
        clang_visitChildren(root, visit_top, &mut ctx as *mut Visit as *mut c_void);
    }
    Ok(out)
}

fn has_declarations(unit: &Unit) -> bool {
    unsafe {
        let mut found = false;
        extern "C" fn any(_: CXCursor, _: CXCursor, data: CXClientData) -> CXChildVisitResult {
            unsafe { *(data as *mut bool) = true };
            CXChildVisit_Break
        }
        clang_visitChildren(clang_getTranslationUnitCursor(unit.tu), any, &mut found as *mut bool as *mut c_void);
        found
    }
}

struct Visit<'a> {
    decls: &'a mut BTreeMap<String, Decl>,
}

extern "C" fn visit_top(cursor: CXCursor, _parent: CXCursor, data: CXClientData) -> CXChildVisitResult {
    let ctx = unsafe { &mut *(data as *mut Visit) };
    unsafe { declare(cursor, ctx.decls) };
    CXChildVisit_Continue
}

/// Is `cursor` written in the synthetic file itself?
unsafe fn in_synthetic(cursor: CXCursor) -> bool {
    unsafe {
        let loc = clang_getCursorLocation(cursor);
        let mut file = std::ptr::null_mut();
        clang_getSpellingLocation(loc, &mut file, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        !file.is_null() && text(clang_getFileName(file)).ends_with(SYNTHETIC_NAME)
    }
}

unsafe fn declare(cursor: CXCursor, decls: &mut BTreeMap<String, Decl>) {
    unsafe {
        let kind = clang_getCursorKind(cursor);
        let name = text(clang_getCursorSpelling(cursor));
        match kind {
            CXCursor_FunctionDecl => {
                if name.is_empty() {
                    return;
                }
                let ty = clang_getCursorType(cursor);
                let mut sig = signature(cursor, ty);
                sig.variadic = clang_isFunctionTypeVariadic(ty) != 0;
                let inline = clang_Cursor_getStorageClass(cursor) == CX_SC_Static;
                // A later declaration with a body or more detail replaces nothing:
                // repeated declarations of one symbol are one declaration.
                decls.entry(name).or_insert(Decl::Func { sig, inline });
            }
            CXCursor_StructDecl | CXCursor_UnionDecl => {
                if name.is_empty() {
                    return;
                }
                let rec = record(cursor, &name, kind == CXCursor_UnionDecl);
                match decls.get(&name) {
                    // A complete definition replaces an earlier forward declaration.
                    Some(Decl::Record(old)) if !old.opaque => {}
                    _ => {
                        decls.insert(name, Decl::Record(rec));
                    }
                }
            }
            CXCursor_EnumDecl => {
                let en = enum_decl(cursor, &name, decls);
                if !name.is_empty() {
                    decls.entry(name).or_insert(Decl::Enum(en));
                }
            }
            CXCursor_TypedefDecl => {
                if name.is_empty() {
                    return;
                }
                let under = clang_getTypedefDeclUnderlyingType(cursor);
                // `typedef struct { } Name;` and `typedef enum { } Name;`: the
                // typedef names the anonymous declaration.
                let decl = clang_getTypeDeclaration(clang_getCanonicalType(under));
                let dk = clang_getCursorKind(decl);
                let anonymous = text(clang_getCursorSpelling(decl)).is_empty() || clang_Cursor_isAnonymous(decl) != 0;
                if dk == CXCursor_StructDecl || dk == CXCursor_UnionDecl {
                    if anonymous {
                        let rec = record(decl, &name, dk == CXCursor_UnionDecl);
                        decls.insert(name, Decl::Record(rec));
                    } else {
                        let target = text(clang_getCursorSpelling(decl));
                        if target != name {
                            decls.entry(name).or_insert(Decl::Alias(CType::Record(target)));
                        }
                    }
                } else if dk == CXCursor_EnumDecl {
                    if anonymous {
                        let en = enum_decl(decl, &name, decls);
                        decls.insert(name, Decl::Enum(en));
                    } else {
                        let target = text(clang_getCursorSpelling(decl));
                        if target != name {
                            decls.entry(name).or_insert(Decl::Alias(CType::Enum(target)));
                        }
                    }
                } else {
                    let ty = convert(under);
                    decls.entry(name).or_insert(Decl::Alias(ty));
                }
            }
            CXCursor_MacroDefinition => {
                if name.is_empty() || in_synthetic(cursor) {
                    return;
                }
                let tokens = macro_tokens(cursor);
                let function_like = clang_Cursor_isMacroFunctionLike(cursor) != 0;
                decls.entry(name).or_insert(Decl::Macro(CMacro { tokens, function_like }));
            }
            CXCursor_VarDecl => {
                if !name.is_empty() {
                    decls.entry(name).or_insert(Decl::Unsupported("global C variables are not supported yet".into()));
                }
            }
            _ => {}
        }
    }
}

unsafe fn macro_tokens(cursor: CXCursor) -> Vec<String> {
    unsafe {
        let tu = clang_Cursor_getTranslationUnit(cursor);
        let range = clang_getCursorExtent(cursor);
        let mut tokens: *mut CXToken = std::ptr::null_mut();
        let mut n: c_uint = 0;
        clang_tokenize(tu, range, &mut tokens, &mut n);
        let mut out = Vec::new();
        for i in 0..n {
            out.push(text(clang_getTokenSpelling(tu, *tokens.add(i as usize))));
        }
        if !tokens.is_null() {
            clang_disposeTokens(tu, tokens, n);
        }
        // The first token is the macro's name.
        if !out.is_empty() {
            out.remove(0);
        }
        out
    }
}

unsafe fn enum_decl(cursor: CXCursor, name: &str, decls: &mut BTreeMap<String, Decl>) -> CEnum {
    unsafe {
        let int_ty = clang_getEnumDeclIntegerType(cursor);
        let underlying = match convert(int_ty) {
            CType::Scalar(k) => k,
            _ => CKind::Int,
        };
        let signed = underlying.underlying_is_signed();
        let mut constants: Vec<(String, i128)> = Vec::new();
        struct Collect<'a> {
            out: &'a mut Vec<(String, i128)>,
            signed: bool,
        }
        extern "C" fn each(c: CXCursor, _: CXCursor, data: CXClientData) -> CXChildVisitResult {
            unsafe {
                let ctx = &mut *(data as *mut Collect);
                if clang_getCursorKind(c) == CXCursor_EnumConstantDecl {
                    let name = text(clang_getCursorSpelling(c));
                    let v = if ctx.signed { clang_getEnumConstantDeclValue(c) as i128 } else { clang_getEnumConstantDeclUnsignedValue(c) as i128 };
                    ctx.out.push((name, v));
                }
            }
            CXChildVisit_Continue
        }
        let mut ctx = Collect { out: &mut constants, signed };
        clang_visitChildren(cursor, each, &mut ctx as *mut Collect as *mut c_void);
        for (c, v) in &constants {
            decls.entry(c.clone()).or_insert(Decl::Enumerator { value: *v, of: name.to_string() });
        }
        CEnum { name: name.to_string(), underlying, constants }
    }
}

unsafe fn signature(cursor: CXCursor, ty: CXType) -> CSig {
    unsafe {
        let ret = convert(clang_getResultType(ty));
        let n = clang_getNumArgTypes(ty).max(0) as c_uint;
        let mut params = Vec::new();
        for i in 0..n {
            let pname = {
                let a = clang_Cursor_getArgument(cursor, i);
                text(clang_getCursorSpelling(a))
            };
            let pname = if pname.is_empty() { format!("a{i}") } else { pname };
            params.push((pname, convert(clang_getArgType(ty, i))));
        }
        CSig { ret, params, variadic: false }
    }
}

unsafe fn record(cursor: CXCursor, name: &str, is_union: bool) -> CRecord {
    unsafe {
        let ty = clang_getCursorType(cursor);
        let size = clang_Type_getSizeOf(ty);
        if size == CXTypeLayoutError_Incomplete as i64 || clang_isCursorDefinition(cursor) == 0 {
            return CRecord { name: name.to_string(), is_union, opaque: true, fields: Vec::new(), size: 0, unsupported: None };
        }
        struct Collect {
            fields: Vec<CField>,
            problem: Option<String>,
            /// `(offset in bits, size, alignment)` of each field.
            layout: Vec<(i64, i64, i64)>,
        }
        extern "C" fn each(c: CXCursor, _: CXCursor, data: CXClientData) -> CXChildVisitResult {
            unsafe {
                let ctx = &mut *(data as *mut Collect);
                match clang_getCursorKind(c) {
                    CXCursor_FieldDecl => {
                        let fname = text(clang_getCursorSpelling(c));
                        let fty = clang_getCursorType(c);
                        if clang_Cursor_isBitField(c) != 0 {
                            ctx.problem.get_or_insert_with(|| format!("field `{fname}` is a bit-field"));
                        }
                        let conv = convert(fty);
                        if let CType::Unsupported(why) = &conv {
                            ctx.problem.get_or_insert_with(|| format!("field `{fname}`: {why}"));
                        }
                        let canon = clang_getCanonicalType(fty);
                        if canon.kind == CXType_IncompleteArray {
                            ctx.problem.get_or_insert_with(|| format!("field `{fname}` is a flexible array member"));
                        }
                        ctx.layout.push((clang_Cursor_getOffsetOfField(c), clang_Type_getSizeOf(fty), clang_Type_getAlignOf(fty)));
                        ctx.fields.push(CField { name: fname, ty: conv });
                    }
                    CXCursor_StructDecl | CXCursor_UnionDecl => {
                        // An anonymous nested record is a member without a name.
                    }
                    _ => {}
                }
            }
            CXChildVisit_Continue
        }
        let mut ctx = Collect { fields: Vec::new(), problem: None, layout: Vec::new() };
        clang_visitChildren(cursor, each, &mut ctx as *mut Collect as *mut c_void);
        let mut problem = ctx.problem;
        if is_union && problem.is_none() {
            problem = Some("unions are not supported yet".into());
        }
        if problem.is_none() && ctx.fields.iter().any(|f| f.name.is_empty()) {
            problem = Some("it has an anonymous member".into());
        }
        if problem.is_none() {
            // The layout must be the natural one, which is the one Drachm
            // gives a struct with the same fields.
            let mut end = 0i64;
            let mut align = 1i64;
            for &(off_bits, sz, al) in &ctx.layout {
                let at = (end + al - 1) / al * al;
                if off_bits != at * 8 || sz < 0 || al <= 0 {
                    problem = Some("its layout is not the natural one (packed or aligned fields)".into());
                    break;
                }
                end = at + sz;
                align = align.max(al);
            }
            if problem.is_none() && (end + align - 1) / align * align != size {
                problem = Some("its size is not the natural one".into());
            }
        }
        CRecord { name: name.to_string(), is_union, opaque: false, fields: ctx.fields, size: size.max(0) as u64, unsupported: problem }
    }
}

/// Typedef names that stay C types of their own.
fn named_scalar(name: &str) -> Option<CKind> {
    match name {
        "size_t" => Some(CKind::SizeT),
        "ssize_t" => Some(CKind::SSizeT),
        _ => None,
    }
}

unsafe fn convert(ty: CXType) -> CType {
    unsafe {
        match ty.kind {
            CXType_Typedef => {
                let decl = clang_getTypeDeclaration(ty);
                let name = text(clang_getCursorSpelling(decl));
                if let Some(k) = named_scalar(&name) {
                    return CType::Scalar(k);
                }
                let under = clang_getTypedefDeclUnderlyingType(decl);
                // A typedef of an anonymous struct or enum is named by the typedef.
                let target = clang_getTypeDeclaration(clang_getCanonicalType(under));
                let tk = clang_getCursorKind(target);
                let anon = text(clang_getCursorSpelling(target)).is_empty() || clang_Cursor_isAnonymous(target) != 0;
                if anon && (tk == CXCursor_StructDecl || tk == CXCursor_UnionDecl) {
                    return CType::Record(name);
                }
                if anon && tk == CXCursor_EnumDecl {
                    return CType::Enum(name);
                }
                convert(under)
            }
            CXType_Elaborated => convert(clang_Type_getNamedType(ty)),
            CXType_Attributed | CXType_Atomic => {
                convert(clang_getCanonicalType(ty))
            }
            CXType_Void => CType::Void,
            CXType_Bool => CType::Scalar(CKind::Bool),
            CXType_Char_U | CXType_Char_S => CType::Scalar(CKind::Char),
            CXType_SChar => CType::Scalar(CKind::SChar),
            CXType_UChar => CType::Scalar(CKind::UChar),
            CXType_Short => CType::Scalar(CKind::Short),
            CXType_UShort => CType::Scalar(CKind::UShort),
            CXType_Int => CType::Scalar(CKind::Int),
            CXType_UInt => CType::Scalar(CKind::UInt),
            CXType_Long => CType::Scalar(CKind::Long),
            CXType_ULong => CType::Scalar(CKind::ULong),
            CXType_LongLong => CType::Scalar(CKind::LongLong),
            CXType_ULongLong => CType::Scalar(CKind::ULongLong),
            CXType_Float => CType::Scalar(CKind::Float),
            CXType_Double => CType::Scalar(CKind::Double),
            CXType_LongDouble => CType::Unsupported("`long double` is not supported yet".into()),
            CXType_Int128 | CXType_UInt128 => CType::Unsupported("128-bit C integers are not supported yet".into()),
            CXType_Pointer => {
                let to = clang_getPointeeType(ty);
                let canon = clang_getCanonicalType(to);
                if canon.kind == CXType_FunctionProto || canon.kind == CXType_FunctionNoProto {
                    return match convert_function(canon) {
                        Some(sig) => CType::FuncPtr(Box::new(sig)),
                        None => CType::Unsupported("function pointer".into()),
                    };
                }
                CType::Ptr(Box::new(convert(to)))
            }
            CXType_Record => {
                let decl = clang_getTypeDeclaration(ty);
                let name = text(clang_getCursorSpelling(decl));
                if name.is_empty() || clang_Cursor_isAnonymous(decl) != 0 {
                    CType::Unsupported("an anonymous struct or union without a typedef name".into())
                } else {
                    CType::Record(name)
                }
            }
            CXType_Enum => {
                let decl = clang_getTypeDeclaration(ty);
                let name = text(clang_getCursorSpelling(decl));
                if name.is_empty() {
                    convert(clang_getEnumDeclIntegerType(decl))
                } else {
                    CType::Enum(name)
                }
            }
            CXType_ConstantArray => {
                let elem = convert(clang_getArrayElementType(ty));
                CType::Array(Box::new(elem), clang_getArraySize(ty).max(0) as u64)
            }
            CXType_IncompleteArray => CType::Unsupported("an array of unknown length".into()),
            CXType_FunctionProto | CXType_FunctionNoProto => CType::Unsupported("a function type".into()),
            _ => {
                let canon = clang_getCanonicalType(ty);
                if canon.kind != ty.kind {
                    convert(canon)
                } else {
                    CType::Unsupported(format!("the type `{}`", text(clang_getTypeSpelling(ty))))
                }
            }
        }
    }
}

unsafe fn convert_function(ty: CXType) -> Option<CSig> {
    unsafe {
        if ty.kind != CXType_FunctionProto {
            return None;
        }
        let ret = convert(clang_getResultType(ty));
        let n = clang_getNumArgTypes(ty).max(0) as c_uint;
        let params = (0..n).map(|i| (format!("a{i}"), convert(clang_getArgType(ty, i)))).collect();
        Some(CSig { ret, params, variadic: clang_isFunctionTypeVariadic(ty) != 0 })
    }
}

/// Evaluate object-like macros by name. A macro whose expansion is not a
/// constant number or string is absent from the result.
pub fn eval_macros(unit: &CUnit, names: &[String]) -> Result<BTreeMap<String, CValue>, Failure> {
    let mut src = unit.source.clone();
    let mut slots = Vec::new();
    for (i, n) in names.iter().enumerate() {
        // `__auto_type` keeps the macro's own type; string macros are arrays.
        src.push_str(&format!("static const __typeof__(({n})) drachm_macro_{i} = ({n});\n"));
        slots.push((i, n.clone()));
    }
    let parsed = parse(&src, &unit.settings, false)?;
    let mut out = BTreeMap::new();
    unsafe {
        struct Collect<'a> {
            names: &'a [(usize, String)],
            out: &'a mut BTreeMap<String, CValue>,
        }
        extern "C" fn each(c: CXCursor, _: CXCursor, data: CXClientData) -> CXChildVisitResult {
            unsafe {
                let ctx = &mut *(data as *mut Collect);
                if clang_getCursorKind(c) != CXCursor_VarDecl {
                    return CXChildVisit_Continue;
                }
                let vname = text(clang_getCursorSpelling(c));
                let Some(idx) = vname.strip_prefix("drachm_macro_").and_then(|n| n.parse::<usize>().ok()) else {
                    return CXChildVisit_Continue;
                };
                let Some((_, name)) = ctx.names.iter().find(|(i, _)| *i == idx) else { return CXChildVisit_Continue };
                let res = clang_Cursor_Evaluate(c);
                if res.is_null() {
                    return CXChildVisit_Continue;
                }
                let value = match clang_EvalResult_getKind(res) {
                    CXEval_Int => {
                        if clang_EvalResult_isUnsignedInt(res) != 0 {
                            Some(CValue::Int(clang_EvalResult_getAsUnsigned(res) as i128))
                        } else {
                            Some(CValue::Int(clang_EvalResult_getAsLongLong(res) as i128))
                        }
                    }
                    CXEval_Float => Some(CValue::Float(clang_EvalResult_getAsDouble(res))),
                    CXEval_StrLiteral | CXEval_CFStr => {
                        let p = clang_EvalResult_getAsStr(res);
                        (!p.is_null()).then(|| CValue::Str(CStr::from_ptr(p).to_string_lossy().into_owned()))
                    }
                    _ => None,
                };
                clang_EvalResult_dispose(res);
                if let Some(v) = value {
                    ctx.out.insert(name.clone(), v);
                }
            }
            CXChildVisit_Continue
        }
        let mut ctx = Collect { names: &slots, out: &mut out };
        clang_visitChildren(clang_getTranslationUnitCursor(parsed.tu), each, &mut ctx as *mut Collect as *mut c_void);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_settings() -> Settings {
        Settings { include_dirs: vec![PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/c"))], defines: Vec::new() }
    }

    #[test]
    fn reads_the_fixture_header() {
        let unit = discover("drachm_fixture.h", &fixture_settings()).expect("the fixture header parses");
        match unit.decls.get("fx_add") {
            Some(Decl::Func { sig, .. }) => {
                assert_eq!(sig.ret, CType::Scalar(CKind::Int));
                assert_eq!(sig.params.len(), 2);
            }
            other => panic!("fx_add: {other:?}"),
        }
        assert!(matches!(unit.decls.get("FX_COUNT"), Some(Decl::Macro(_))));
    }

    #[test]
    fn missing_header_is_not_found() {
        match discover("no_such_header_anywhere.h", &Settings::default()) {
            Err(Failure::NotFound(_)) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn evaluates_macros() {
        let unit = discover("drachm_fixture.h", &fixture_settings()).unwrap();
        let values = eval_macros(&unit, &["FX_COUNT".to_string(), "FX_NAME".to_string()]).unwrap();
        assert_eq!(values.get("FX_COUNT"), Some(&CValue::Int(12)));
        assert_eq!(values.get("FX_NAME"), Some(&CValue::Str("fixture".into())));
    }
}
