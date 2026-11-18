//! Script libraries: the host side of `cad.use(name)`.
//!
//! A library is an ES module file. The sandbox engine cannot load ES modules
//! (see `docs/api-surface.md`, "Script libraries"), so the host does what a
//! module loader would: it resolves the name to a file under the run's read
//! root, reads it, and turns its `import` and `export` declarations into
//! plain statements inside an async function. The loader in the script
//! (`cad.use`, in `web/src/cad-api.ts`) evaluates that function once per
//! run and builds the module namespace from what it returns.
//!
//! The transform keeps every line where it was, so line numbers in errors
//! and stack traces are the library's own, and the function is named after
//! the library file (`//# sourceURL=lib/drafting.js`).
//!
//! Supported module syntax, at the top level of the file:
//!
//! - `import "x"`, `import d from "x"`, `import * as ns from "x"`,
//!   `import { a, b as c } from "x"`, `import d, { a } from "x"`,
//!   `import d, * as ns from "x"`
//! - `export const|let|var a = ..., b = ...`, `export function f`,
//!   `export async function f`, `export function* g`, `export class C`
//! - `export default <expression>`, `export default function f`,
//!   `export default class C`
//! - `export { a, b as c }`, `export { a } from "x"`, `export * from "x"`,
//!   `export * as ns from "x"`
//!
//! Not supported (each is a load error naming the line): dynamic
//! `import()`, `import.meta`, import attributes, string export names and
//! exported destructuring patterns. Imported names are bound once, when the
//! import runs; the namespace object (`import * as ns`, or what `cad.use`
//! returns) reads exports live.

use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// The folder, under the run's read root, that bare library names resolve to.
pub const LIBRARY_DIR: &str = "lib";

/// A library ready for the loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// The file, relative to the read root, with `/` separators
    /// (`lib/drafting.js`). One per file: the loader's cache key, and the
    /// name the library has in stack traces.
    pub id: String,
    /// Script text that evaluates to `async function (__cadImport)`. Called,
    /// it runs the module body and returns `{ names, stars }`: a getter per
    /// exported name, and the namespaces of `export * from` modules.
    pub code: String,
}

/// `lib.load` host call / `loadLibrary` RPC: `{spec, from}` to
/// `{id, code}`.
pub fn load_json(root: &Path, args: &str) -> Result<String, String> {
    let a: Value = serde_json::from_str(args).map_err(|e| format!("cad.use: {e}"))?;
    let spec = a["spec"]
        .as_str()
        .ok_or("cad.use: a library name must be a string")?;
    let from = a["from"].as_str();
    let library = load(root, spec, from)?;
    Ok(serde_json::json!({ "id": library.id, "code": library.code }).to_string())
}

/// Resolve `spec` as seen from the library `from` (a library id, or `None`
/// for the script itself), read it and transform it.
///
/// - A bare name (`drafting`, `parts/joints`) is a file in `lib/` under
///   `root`: `lib/drafting.js`.
/// - A relative path (`./geom.js`, `../shared/units`) is relative to the
///   importing library's folder, or to `root` for the script.
///
/// `.js` is added unless the name ends in `.js` or `.mjs`. The file must be
/// inside `root`, the same rule as `cad.readFile`.
pub fn load(root: &Path, spec: &str, from: Option<&str>) -> Result<Library, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("library root '{}': {e}", root.display()))?;
    let rel = resolve(spec, from)?;
    let path = root.join(&rel);
    let shown = to_id(&rel);
    let real = match path.canonicalize() {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("library \"{spec}\" not found (no file {shown})"));
        }
        Err(e) => return Err(format!("library \"{spec}\" ({shown}): {e}")),
    };
    // Lexically inside the root is not enough: a link may point outside.
    let inside = real
        .strip_prefix(&root)
        .map_err(|_| format!("library \"{spec}\" is outside the script's folder"))?;
    if !real.is_file() {
        return Err(format!("library \"{spec}\" ({shown}) is not a file"));
    }
    let id = to_id(inside);
    let source = std::fs::read_to_string(&real).map_err(|e| format!("library {id}: {e}"))?;
    let body = to_function_body(&source)
        .map_err(|e| format!("library {id}: {} at {id}:{}:{}", e.message, e.line, e.col))?;
    Ok(Library {
        code: format!(
            "\"use strict\";(async function (__cadImport) {{{body}\n}})\n//# sourceURL={}",
            source_url(&id)
        ),
        id,
    })
}

/// A path relative to the read root, as an id: `/` separators.
fn to_id(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The id as a `sourceURL`: no line ends or other control characters.
fn source_url(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The library's path relative to the root, checked lexically: it never
/// leaves the root, whatever `spec` and `from` say.
fn resolve(spec: &str, from: Option<&str>) -> Result<PathBuf, String> {
    let bad = || {
        format!(
            "\"{spec}\" is not a library name: use a name like \"drafting\" \
             (the file {LIBRARY_DIR}/drafting.js) or a relative path like \"./parts.js\""
        )
    };
    if spec.is_empty() || spec.contains(['\\', ':', '\0']) || spec.starts_with('/') {
        return Err(bad());
    }
    let relative = spec.starts_with("./") || spec.starts_with("../");
    let mut parts: Vec<&str> = Vec::new();
    if relative {
        // The importing library's folder.
        if let Some(from) = from {
            let from = normalize(&[], from).map_err(|_| bad())?;
            parts = from;
            parts.pop();
        }
    } else {
        // A bare name: plain segments only, no `.` or `..`.
        let plain = spec.split('/').all(|s| {
            !s.is_empty()
                && !s.starts_with('.')
                && s.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '@'))
        });
        if !plain {
            return Err(bad());
        }
        parts.push(LIBRARY_DIR);
    }
    let mut parts = normalize(&parts, spec)
        .map_err(|_| format!("library \"{spec}\" is outside the script's folder"))?;
    let Some(last) = parts.pop() else {
        return Err(bad());
    };
    let file = if last.ends_with(".js") || last.ends_with(".mjs") {
        last.to_string()
    } else {
        format!("{last}.js")
    };
    let mut path: PathBuf = parts.iter().collect();
    path.push(file);
    // Belt and braces: nothing but plain names may remain.
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(bad());
    }
    Ok(path)
}

/// Apply `path`'s segments to `base`: `.` stays, `..` goes up. Going above
/// the root is an error.
fn normalize<'a>(base: &[&'a str], path: &'a str) -> Result<Vec<&'a str>, ()> {
    let mut out = base.to_vec();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop().ok_or(())?;
            }
            s if s.contains(['\\', ':', '\0']) => return Err(()),
            s => out.push(s),
        }
    }
    Ok(out)
}

// ── The module transform ────────────────────────────────────────────

/// Where and why a library could not be transformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ident,
    Punct,
    Str,
    Template,
    Number,
    Regex,
}

#[derive(Debug, Clone, Copy)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
    line: usize,
    col: usize,
    /// A line end between this token and the one before it.
    nl_before: bool,
}

/// Keywords after which a `/` starts a regular expression, not a division.
const REGEX_AFTER: &[&str] = &[
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
];

struct Lexer<'a> {
    src: &'a str,
    b: &'a [u8],
    i: usize,
    line: usize,
    line_start: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer {
            src,
            b: src.as_bytes(),
            i: 0,
            line: 1,
            line_start: 0,
        }
    }

    fn err(&self, at: usize, line: usize, line_start: usize, message: &str) -> TransformError {
        TransformError {
            line,
            col: at.saturating_sub(line_start) + 1,
            message: message.to_string(),
        }
    }

    fn peek(&self, k: usize) -> u8 {
        self.b.get(self.i + k).copied().unwrap_or(0)
    }

    fn newline(&mut self) {
        self.line += 1;
        self.line_start = self.i + 1;
    }

    /// Skip whitespace and comments; true if a line end was crossed.
    fn skip_space(&mut self) -> Result<bool, TransformError> {
        let mut nl = false;
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'\n' => {
                    self.newline();
                    nl = true;
                    self.i += 1;
                }
                b' ' | b'\t' | b'\r' | 0x0b | 0x0c => self.i += 1,
                b'/' if self.peek(1) == b'/' => {
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                b'/' if self.peek(1) == b'*' => {
                    let (at, line, ls) = (self.i, self.line, self.line_start);
                    self.i += 2;
                    loop {
                        if self.i >= self.b.len() {
                            return Err(self.err(at, line, ls, "unterminated comment"));
                        }
                        if self.b[self.i] == b'*' && self.peek(1) == b'/' {
                            self.i += 2;
                            break;
                        }
                        if self.b[self.i] == b'\n' {
                            self.newline();
                            nl = true;
                        }
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
        Ok(nl)
    }

    fn text(&self, t: &Tok) -> &'a str {
        &self.src[t.start..t.end]
    }

    fn regex_allowed(&self, prev: Option<&Tok>) -> bool {
        let Some(p) = prev else { return true };
        match p.kind {
            Kind::Ident => REGEX_AFTER.contains(&self.text(p)),
            Kind::Punct => !matches!(self.text(p), ")" | "]" | "++" | "--"),
            _ => false,
        }
    }

    /// The next token, or `None` at the end.
    fn next(&mut self, prev: Option<&Tok>) -> Result<Option<Tok>, TransformError> {
        let nl_before = self.skip_space()?;
        if self.i >= self.b.len() {
            return Ok(None);
        }
        let (start, line, ls) = (self.i, self.line, self.line_start);
        let c = self.b[self.i];
        let kind = match c {
            b'\'' | b'"' => {
                self.i += 1;
                loop {
                    match self.b.get(self.i) {
                        None | Some(b'\n') => {
                            return Err(self.err(start, line, ls, "unterminated string"))
                        }
                        Some(b'\\') => {
                            if self.peek(1) == b'\n' {
                                self.i += 1;
                                self.newline();
                                self.i += 1;
                            } else {
                                self.i += 2;
                            }
                        }
                        Some(&q) if q == c => {
                            self.i += 1;
                            break;
                        }
                        Some(_) => self.i += 1,
                    }
                }
                Kind::Str
            }
            b'`' => {
                self.template(start, line, ls)?;
                Kind::Template
            }
            b'/' if self.regex_allowed(prev) => {
                self.i += 1;
                let mut class = false;
                loop {
                    match self.b.get(self.i) {
                        None | Some(b'\n') => {
                            return Err(self.err(
                                start,
                                line,
                                ls,
                                "unterminated regular expression",
                            ))
                        }
                        Some(b'\\') => self.i += 2,
                        Some(b'[') => {
                            class = true;
                            self.i += 1;
                        }
                        Some(b']') => {
                            class = false;
                            self.i += 1;
                        }
                        Some(b'/') if !class => {
                            self.i += 1;
                            break;
                        }
                        Some(_) => self.i += 1,
                    }
                }
                while self.i < self.b.len() && is_ident_byte(self.b[self.i]) {
                    self.i += 1;
                }
                Kind::Regex
            }
            b'0'..=b'9' => {
                self.number();
                Kind::Number
            }
            b'.' if self.peek(1).is_ascii_digit() => {
                self.number();
                Kind::Number
            }
            b'.' if self.peek(1) == b'.' && self.peek(2) == b'.' => {
                self.i += 3;
                Kind::Punct
            }
            b'+' | b'-' if self.peek(1) == c => {
                self.i += 2;
                Kind::Punct
            }
            b'#' => {
                self.i += 1;
                while self.i < self.b.len() && is_ident_byte(self.b[self.i]) {
                    self.i += 1;
                }
                Kind::Ident
            }
            c if is_ident_byte(c) => {
                while self.i < self.b.len() && is_ident_byte(self.b[self.i]) {
                    self.i += 1;
                }
                Kind::Ident
            }
            _ => {
                // One punctuator byte (or one UTF-8 character).
                self.i += 1;
                while self.i < self.b.len() && !self.src.is_char_boundary(self.i) {
                    self.i += 1;
                }
                Kind::Punct
            }
        };
        Ok(Some(Tok {
            kind,
            start,
            end: self.i,
            line,
            col: start - ls + 1,
            nl_before,
        }))
    }

    fn number(&mut self) {
        let hex = self.b[self.i] == b'0' && matches!(self.peek(1), b'x' | b'X');
        while self.i < self.b.len() {
            let c = self.b[self.i];
            let exponent_sign =
                (c == b'+' || c == b'-') && !hex && matches!(self.b[self.i - 1], b'e' | b'E');
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || exponent_sign {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    /// A template literal, `${...}` substitutions included.
    fn template(&mut self, start: usize, line: usize, ls: usize) -> Result<(), TransformError> {
        self.i += 1;
        loop {
            match self.b.get(self.i) {
                None => return Err(self.err(start, line, ls, "unterminated template literal")),
                Some(b'\\') => {
                    if self.peek(1) == b'\n' {
                        self.i += 1;
                        self.newline();
                        self.i += 1;
                    } else {
                        self.i += 2;
                    }
                }
                Some(b'`') => {
                    self.i += 1;
                    return Ok(());
                }
                Some(b'\n') => {
                    self.newline();
                    self.i += 1;
                }
                Some(b'$') if self.peek(1) == b'{' => {
                    self.i += 2;
                    let mut depth = 0usize;
                    let mut prev: Option<Tok> = None;
                    loop {
                        let Some(t) = self.next(prev.as_ref())? else {
                            return Err(self.err(start, line, ls, "unterminated template literal"));
                        };
                        if t.kind == Kind::Punct {
                            match self.text(&t) {
                                "{" => depth += 1,
                                "}" if depth == 0 => break,
                                "}" => depth -= 1,
                                _ => {}
                            }
                        }
                        prev = Some(t);
                    }
                }
                Some(_) => self.i += 1,
            }
        }
    }
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'\\' || c >= 0x80
}

fn lex(src: &str) -> Result<Vec<Tok>, TransformError> {
    let mut lexer = Lexer::new(src);
    let mut toks: Vec<Tok> = Vec::new();
    while let Some(t) = lexer.next(toks.last())? {
        toks.push(t);
    }
    Ok(toks)
}

/// Turn an ES module's source into the body of
/// `async function (__cadImport)`: imports become awaited `__cadImport`
/// calls, `export` keywords go, and a final line returns the exports.
pub fn to_function_body(src: &str) -> Result<String, TransformError> {
    let toks = lex(src)?;
    let mut t = Transform {
        src,
        toks: &toks,
        edits: Vec::new(),
        names: Vec::new(),
        stars: Vec::new(),
        reexports: 0,
    };
    t.run()?;

    let mut out = String::with_capacity(src.len() + 256);
    let mut at = 0;
    for (start, end, text) in &t.edits {
        out.push_str(&src[at..*start]);
        out.push_str(text);
        // Keep every following line where it was.
        out.extend(std::iter::repeat_n(
            '\n',
            src[*start..*end].matches('\n').count(),
        ));
        at = *end;
    }
    out.push_str(&src[at..]);

    let names: Vec<String> = t
        .names
        .iter()
        .map(|(name, local)| format!("{}: () => {local}", serde_json::json!(name)))
        .collect();
    out.push_str(&format!(
        "\nreturn {{ names: {{ {} }}, stars: [{}] }};",
        names.join(", "),
        t.stars.join(", ")
    ));
    Ok(out)
}

struct Transform<'a> {
    src: &'a str,
    toks: &'a [Tok],
    /// Byte ranges of the source and what replaces them, in order.
    edits: Vec<(usize, usize, String)>,
    /// Exported name and the expression that reads it.
    names: Vec<(String, String)>,
    /// Namespaces re-exported with `export * from`.
    stars: Vec<String>,
    reexports: usize,
}

impl<'a> Transform<'a> {
    fn text(&self, k: usize) -> &'a str {
        self.toks
            .get(k)
            .map(|t| &self.src[t.start..t.end])
            .unwrap_or("")
    }

    fn is(&self, k: usize, kind: Kind, text: &str) -> bool {
        self.toks.get(k).is_some_and(|t| t.kind == kind) && self.text(k) == text
    }

    fn kind(&self, k: usize) -> Option<Kind> {
        self.toks.get(k).map(|t| t.kind)
    }

    fn fail<T>(&self, k: usize, message: impl Into<String>) -> Result<T, TransformError> {
        let (line, col) = match self.toks.get(k).or(self.toks.last()) {
            Some(t) => (t.line, t.col),
            None => (1, 1),
        };
        Err(TransformError {
            line,
            col,
            message: message.into(),
        })
    }

    fn export(&mut self, k: usize, name: &str, local: String) -> Result<(), TransformError> {
        if self.names.iter().any(|(n, _)| n == name) {
            return self.fail(k, format!("'{name}' is exported twice"));
        }
        self.names.push((name.to_string(), local));
        Ok(())
    }

    fn ident(&self, k: usize, what: &str) -> Result<&'a str, TransformError> {
        match self.kind(k) {
            Some(Kind::Ident) => Ok(self.text(k)),
            Some(Kind::Str) => self.fail(
                k,
                format!("a string as {what} is not supported in a library"),
            ),
            _ => self.fail(k, format!("expected {what}")),
        }
    }

    /// The module specifier at `k`, after `from`.
    fn from(&self, k: usize) -> Result<&'a str, TransformError> {
        if !self.is(k, Kind::Ident, "from") {
            return self.fail(k, "expected 'from'");
        }
        if self.kind(k + 1) != Some(Kind::Str) {
            return self.fail(k + 1, "expected a library name in quotes");
        }
        if self.is(k + 2, Kind::Ident, "with") && self.is(k + 3, Kind::Punct, "{") {
            return self.fail(k + 2, "import attributes are not supported in a library");
        }
        Ok(self.text(k + 1))
    }

    /// One past the statement's last token: an optional `;` after `k`.
    fn end_after(&self, k: usize) -> usize {
        if self.is(k + 1, Kind::Punct, ";") {
            k + 2
        } else {
            k + 1
        }
    }

    fn import_call(&self, spec: &str, k: usize) -> String {
        format!("await __cadImport({spec}, {})", self.toks[k].line)
    }

    /// Replace tokens `from..to` (exclusive) with `text`.
    fn replace(&mut self, from: usize, to: usize, text: String) {
        let start = self.toks[from].start;
        let end = self.toks[to - 1].end;
        self.edits.push((start, end, text));
    }

    /// Remove the source between token `from` and token `to` (exclusive).
    fn strip(&mut self, from: usize, to: usize) {
        let start = self.toks[from].start;
        let end = self.toks[to].start;
        self.edits.push((start, end, String::new()));
    }

    fn run(&mut self) -> Result<(), TransformError> {
        let mut depth = 0i64;
        let mut k = 0;
        while k < self.toks.len() {
            let after_dot = k > 0 && self.is(k - 1, Kind::Punct, ".");
            if self.kind(k) == Some(Kind::Ident) && !after_dot {
                match self.text(k) {
                    "import" if self.is(k + 1, Kind::Punct, "(") => {
                        return self.fail(
                            k,
                            "import() is not supported in a library: \
                             use an import declaration at the top of the file",
                        );
                    }
                    "import" if self.is(k + 1, Kind::Punct, ".") => {
                        return self.fail(k, "import.meta is not supported in a library");
                    }
                    "import" if depth == 0 => {
                        k = self.import(k)?;
                        continue;
                    }
                    "export" if depth == 0 => {
                        k = self.export_decl(k)?;
                        continue;
                    }
                    _ => {}
                }
            }
            if self.kind(k) == Some(Kind::Punct) {
                match self.text(k) {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {}
                }
            }
            k += 1;
        }
        Ok(())
    }

    /// An import declaration at `k`; returns the index after it.
    fn import(&mut self, k: usize) -> Result<usize, TransformError> {
        let mut j = k + 1;
        // import "x";
        if self.kind(j) == Some(Kind::Str) {
            let spec = self.text(j);
            let end = self.end_after(j);
            let call = self.import_call(spec, k);
            self.replace(k, end, format!("{call};"));
            return Ok(end);
        }
        let mut default = None;
        let mut namespace = None;
        let mut named: Vec<(&str, &str)> = Vec::new();
        if self.kind(j) == Some(Kind::Ident) && !self.is(j, Kind::Ident, "from") {
            default = Some(self.text(j));
            j += 1;
            if self.is(j, Kind::Punct, ",") {
                j += 1;
            } else if !self.is(j, Kind::Ident, "from") {
                return self.fail(j, "expected ',' or 'from'");
            }
        }
        if self.is(j, Kind::Punct, "*") {
            if !self.is(j + 1, Kind::Ident, "as") {
                return self.fail(j + 1, "expected 'as'");
            }
            namespace = Some(self.ident(j + 2, "a namespace name")?);
            j += 3;
        } else if self.is(j, Kind::Punct, "{") {
            j += 1;
            while !self.is(j, Kind::Punct, "}") {
                let imported = self.ident(j, "an imported name")?;
                let mut local = imported;
                j += 1;
                if self.is(j, Kind::Ident, "as") {
                    local = self.ident(j + 1, "a local name")?;
                    j += 2;
                }
                if local == "default" {
                    return self.fail(j - 1, "'default' must be imported as another name");
                }
                named.push((imported, local));
                if self.is(j, Kind::Punct, ",") {
                    j += 1;
                } else if !self.is(j, Kind::Punct, "}") {
                    return self.fail(j, "expected ',' or '}'");
                }
            }
            j += 1;
        } else if default.is_none() {
            return self.fail(j, "expected a name, '{' or '*' after 'import'");
        }
        let spec = self.from(j)?;
        let end = self.end_after(j + 1);
        let call = self.import_call(spec, k);
        let text = if let Some(ns) = namespace {
            match default {
                Some(d) => format!("const {ns} = {call}, {d} = {ns}.default;"),
                None => format!("const {ns} = {call};"),
            }
        } else if default.is_none() && named.is_empty() {
            format!("{call};")
        } else {
            let mut fields: Vec<String> = Vec::new();
            if let Some(d) = default {
                fields.push(format!("default: {d}"));
            }
            for (imported, local) in named {
                if imported == local {
                    fields.push(local.to_string());
                } else {
                    fields.push(format!("{imported}: {local}"));
                }
            }
            format!("const {{ {} }} = {call};", fields.join(", "))
        };
        self.replace(k, end, text);
        Ok(end)
    }

    /// An export declaration at `k`; returns the index to continue at.
    fn export_decl(&mut self, k: usize) -> Result<usize, TransformError> {
        let n = k + 1;
        match (self.kind(n), self.text(n)) {
            (Some(Kind::Ident), "default") => self.export_default(k),
            (Some(Kind::Ident), "const" | "let" | "var") => {
                self.strip(k, n);
                self.declarators(n + 1)?;
                // Scan the initializers like any other code.
                Ok(n + 1)
            }
            (Some(Kind::Ident), "function" | "class" | "async") => {
                let name = self.declared_name(n)?;
                self.strip(k, n);
                let local = self.text(name).to_string();
                self.export(name, &local, local.clone())?;
                Ok(n)
            }
            (Some(Kind::Punct), "{") => {
                let mut j = n + 1;
                let mut list: Vec<(&str, &str, usize)> = Vec::new();
                while !self.is(j, Kind::Punct, "}") {
                    let local = self.ident(j, "an exported name")?;
                    let mut exported = local;
                    let at = j;
                    j += 1;
                    if self.is(j, Kind::Ident, "as") {
                        exported = self.ident(j + 1, "an exported name")?;
                        j += 2;
                    }
                    list.push((local, exported, at));
                    if self.is(j, Kind::Punct, ",") {
                        j += 1;
                    } else if !self.is(j, Kind::Punct, "}") {
                        return self.fail(j, "expected ',' or '}'");
                    }
                }
                if self.is(j + 1, Kind::Ident, "from") {
                    let spec = self.from(j + 1)?;
                    let end = self.end_after(j + 2);
                    let ns = self.reexport_name();
                    let call = self.import_call(spec, k);
                    for (local, exported, at) in list {
                        self.export(at, exported, format!("{ns}[{}]", serde_json::json!(local)))?;
                    }
                    self.replace(k, end, format!("const {ns} = {call};"));
                    Ok(end)
                } else {
                    let end = self.end_after(j);
                    for (local, exported, at) in list {
                        self.export(at, exported, local.to_string())?;
                    }
                    self.replace(k, end, String::new());
                    Ok(end)
                }
            }
            (Some(Kind::Punct), "*") => {
                let mut j = n + 1;
                let mut alias = None;
                if self.is(j, Kind::Ident, "as") {
                    alias = Some((self.ident(j + 1, "a namespace name")?, j + 1));
                    j += 2;
                }
                let spec = self.from(j)?;
                let end = self.end_after(j + 1);
                let ns = self.reexport_name();
                let call = self.import_call(spec, k);
                match alias {
                    Some((name, at)) => self.export(at, name, ns.clone())?,
                    None => self.stars.push(ns.clone()),
                }
                self.replace(k, end, format!("const {ns} = {call};"));
                Ok(end)
            }
            _ => self.fail(n, "this export form is not supported in a library"),
        }
    }

    fn reexport_name(&mut self) -> String {
        let name = format!("__cadReexport{}", self.reexports);
        self.reexports += 1;
        name
    }

    /// The name token of the `function`/`class` declaration starting at `k`
    /// (`async function f`, `function* g`, `class C`).
    fn declared_name(&self, k: usize) -> Result<usize, TransformError> {
        let mut j = k;
        if self.is(j, Kind::Ident, "async") {
            j += 1;
            if !self.is(j, Kind::Ident, "function") || self.toks[j].nl_before {
                return self.fail(j, "expected 'function' after 'async'");
            }
        }
        j += 1;
        if self.is(j, Kind::Punct, "*") {
            j += 1;
        }
        if self.kind(j) != Some(Kind::Ident) || self.is(j, Kind::Ident, "extends") {
            return self.fail(j, "an exported function or class needs a name");
        }
        Ok(j)
    }

    fn export_default(&mut self, k: usize) -> Result<usize, TransformError> {
        let d = k + 1;
        let decl = d + 1;
        let named = match self.text(decl) {
            "function" | "class" | "async" if self.kind(decl) == Some(Kind::Ident) => {
                self.declared_name(decl).ok()
            }
            _ => None,
        };
        match named {
            // export default function f() {} : keep the declaration.
            Some(name) => {
                self.strip(k, decl);
                let local = self.text(name).to_string();
                self.export(d, "default", local)?;
            }
            // export default <expression>
            None => {
                self.replace(k, d + 1, "const __cadDefault =".to_string());
                self.export(d, "default", "__cadDefault".to_string())?;
            }
        }
        Ok(decl)
    }

    /// The declarators of `export const|let|var` from `j`: records each name;
    /// returns the index after the statement.
    fn declarators(&mut self, mut j: usize) -> Result<usize, TransformError> {
        loop {
            match self.kind(j) {
                Some(Kind::Ident) => {
                    let name = self.text(j).to_string();
                    self.export(j, &name, name.clone())?;
                }
                Some(Kind::Punct) if matches!(self.text(j), "{" | "[") => {
                    return self.fail(
                        j,
                        "exporting a destructuring pattern is not supported in a library: \
                         declare the names, then `export { a, b }`",
                    );
                }
                _ => return self.fail(j, "expected a name"),
            }
            j += 1;
            // Skip the initializer, to the next declarator or the statement end.
            let mut depth = 0i64;
            loop {
                if j >= self.toks.len() {
                    return Ok(j);
                }
                let tok = self.toks[j];
                if depth == 0 && tok.nl_before && j > 0 && self.statement_ends(j) {
                    return Ok(j);
                }
                if tok.kind == Kind::Punct {
                    match self.text(j) {
                        ";" if depth == 0 => return Ok(j + 1),
                        "," if depth == 0 => {
                            j += 1;
                            break;
                        }
                        "(" | "[" | "{" => depth += 1,
                        ")" | "]" | "}" => depth -= 1,
                        _ => {}
                    }
                }
                j += 1;
            }
        }
    }

    /// At a line end before token `j`, does the statement end (by automatic
    /// semicolon insertion)? Conservative: it continues if the line before
    /// ends in an operator or the next line starts with one.
    fn statement_ends(&self, j: usize) -> bool {
        let prev = j - 1;
        let prev_continues = self.kind(prev) == Some(Kind::Punct)
            && !matches!(self.text(prev), ")" | "]" | "}" | "++" | "--");
        let next_continues = match self.kind(j) {
            Some(Kind::Punct) => !matches!(self.text(j), "{" | "++" | "--" | "!" | "~"),
            Some(Kind::Ident) => matches!(self.text(j), "in" | "instanceof"),
            Some(Kind::Template) => true,
            _ => false,
        };
        !prev_continues && !next_continues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(src: &str) -> String {
        to_function_body(src).unwrap()
    }

    fn fails(src: &str) -> TransformError {
        to_function_body(src).unwrap_err()
    }

    #[test]
    fn imports_become_awaited_calls_on_the_same_line() {
        let src = "// head\nimport \"a\";\nimport d from \"b\";\nimport * as ns from './c.js'\n\
                   import { x, y as z, default as w } from \"d\";\nimport e, { f } from \"e\";\n\
                   import g, * as h from \"g\";\nimport {} from \"i\";\nuse(x);";
        let out = body(src);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "// head");
        assert_eq!(lines[1], "await __cadImport(\"a\", 2);");
        assert_eq!(
            lines[2],
            "const { default: d } = await __cadImport(\"b\", 3);"
        );
        assert_eq!(lines[3], "const ns = await __cadImport('./c.js', 4);");
        assert_eq!(
            lines[4],
            "const { x, y: z, default: w } = await __cadImport(\"d\", 5);"
        );
        assert_eq!(
            lines[5],
            "const { default: e, f } = await __cadImport(\"e\", 6);"
        );
        assert_eq!(
            lines[6],
            "const h = await __cadImport(\"g\", 7), g = h.default;"
        );
        assert_eq!(lines[7], "await __cadImport(\"i\", 8);");
        assert_eq!(lines[8], "use(x);");
        assert_eq!(lines[9], "return { names: {  }, stars: [] };");
    }

    #[test]
    fn a_multi_line_import_keeps_later_lines_in_place() {
        let out = body("import {\n  a,\n  b,\n} from \"x\";\nthrow a;");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "const { a, b } = await __cadImport(\"x\", 1);");
        assert_eq!(lines[4], "throw a;");
    }

    #[test]
    fn exports_are_collected_and_their_keyword_removed() {
        let src = "export const A = 1, B = [1, 2], C = { p: 3 };\n\
                   export let s = 1\n\
                   export var v;\n\
                   export function f() { return s; }\n\
                   export async function g() {}\n\
                   export function* h() {}\n\
                   export class K {}\n\
                   const p = 1, q = 2;\n\
                   export { p, q as r };\n\
                   export default function main() {}";
        let out = body(src);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "const A = 1, B = [1, 2], C = { p: 3 };");
        assert_eq!(lines[1], "let s = 1");
        assert_eq!(lines[3], "function f() { return s; }");
        assert_eq!(lines[4], "async function g() {}");
        assert_eq!(lines[8], "");
        assert_eq!(lines[9], "function main() {}");
        assert_eq!(
            lines[10],
            "return { names: { \"A\": () => A, \"B\": () => B, \"C\": () => C, \"s\": () => s, \
             \"v\": () => v, \"f\": () => f, \"g\": () => g, \"h\": () => h, \"K\": () => K, \
             \"p\": () => p, \"r\": () => q, \"default\": () => main }, stars: [] };"
        );
    }

    #[test]
    fn declarators_follow_automatic_semicolons() {
        // A new statement on the next line ends the declaration ...
        let out = body("export const a = 1\nconst b = 2, c = 3\nexport let d = a\n  + b, e = 4");
        assert!(
            out.ends_with(
                "return { names: { \"a\": () => a, \"d\": () => d, \"e\": () => e }, stars: [] };"
            ),
            "{out}"
        );
        // ... and commas inside calls, arrays and objects are not declarators.
        let out = body("export const f = (x, y) => [x, y], g = { a: 1, b: 2 };");
        assert!(
            out.contains("names: { \"f\": () => f, \"g\": () => g }"),
            "{out}"
        );
    }

    #[test]
    fn default_exports_of_expressions_get_a_binding() {
        let out = body("export default { a: 1 };");
        assert!(out.starts_with("const __cadDefault = { a: 1 };"), "{out}");
        assert!(out.contains("\"default\": () => __cadDefault"), "{out}");
        let out = body("export default function () {}");
        assert!(
            out.starts_with("const __cadDefault = function () {}"),
            "{out}"
        );
        let out = body("export default class extends Base {}");
        assert!(
            out.starts_with("const __cadDefault = class extends Base {}"),
            "{out}"
        );
    }

    #[test]
    fn reexports_import_once_and_read_through_the_namespace() {
        let out = body(
            "export * from \"a\";\nexport * as b from \"b\";\nexport { x, y as z } from \"c\";",
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            "const __cadReexport0 = await __cadImport(\"a\", 1);"
        );
        assert_eq!(
            lines[1],
            "const __cadReexport1 = await __cadImport(\"b\", 2);"
        );
        assert_eq!(
            lines[2],
            "const __cadReexport2 = await __cadImport(\"c\", 3);"
        );
        assert_eq!(
            lines[3],
            "return { names: { \"b\": () => __cadReexport1, \"x\": () => __cadReexport2[\"x\"], \
             \"z\": () => __cadReexport2[\"y\"] }, stars: [__cadReexport0] };"
        );
    }

    #[test]
    fn module_words_inside_strings_comments_and_nested_code_are_left_alone() {
        let src = "const s = \"export const x = 1\";\n\
                   const t = `\nexport const y = ${ { a: \"}\" }.a }\n`;\n\
                   /* export const z = 1; */\n\
                   const r = /export {a}/g, d = 4e-5 / 2 / 1, h = 0x1e - 1 / 2;\n\
                   function f() { const o = { export: 1, import: 2 }; return o.export + o.import; }\n\
                   obj.import(1);\n\
                   class Q { static export() {} }";
        let out = body(src);
        assert_eq!(&out[..src.len()], src);
        assert!(out.ends_with("return { names: {  }, stars: [] };"), "{out}");
    }

    #[test]
    fn unsupported_forms_name_their_line() {
        let e = fails("const a = 1;\nconst m = await import(\"x\");");
        assert_eq!((e.line, e.col), (2, 17));
        assert!(e.message.contains("import()"), "{e:?}");
        assert!(fails("\n\nconsole.log(import.meta.url);")
            .message
            .contains("import.meta"));
        assert_eq!(fails("\n\nconsole.log(import.meta.url);").line, 3);
        let e = fails("export const { a, b } = o;");
        assert!(e.message.contains("destructuring"), "{e:?}");
        let e = fails("import x from \"a\" with { type: \"json\" };");
        assert!(e.message.contains("attributes"), "{e:?}");
        let e = fails("const a = 1;\nexport { a, a };");
        assert_eq!(e.line, 2);
        assert!(e.message.contains("twice"), "{e:?}");
        let e = fails("import { \"a-b\" as c } from \"x\";");
        assert!(e.message.contains("string"), "{e:?}");
        let e = fails("const s = 'open;\n");
        assert!(e.message.contains("unterminated string"), "{e:?}");
        assert!(fails("const t = `x${ 1 ").message.contains("template"));
    }

    #[test]
    fn load_reads_transforms_and_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("lib/folder.js")).unwrap();
        std::fs::write(dir.path().join("lib/geo.js"), "export const a = 1;\n").unwrap();
        let lib = load(dir.path(), "geo", None).unwrap();
        assert_eq!(lib.id, "lib/geo.js");
        assert_eq!(
            lib.code,
            "\"use strict\";(async function (__cadImport) {const a = 1;\n\n\
             return { names: { \"a\": () => a }, stars: [] };\n})\n//# sourceURL=lib/geo.js"
        );
        // Through the JSON entry the host call and the RPC use.
        let reply: Value = serde_json::from_str(
            &load_json(dir.path(), r#"{"spec": "./geo", "from": "lib/x.js"}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(reply["id"], "lib/geo.js");

        let missing = load(dir.path(), "nope", None).unwrap_err();
        assert_eq!(missing, "library \"nope\" not found (no file lib/nope.js)");
        let folder = load(dir.path(), "folder.js", None).unwrap_err();
        assert!(folder.contains("is not a file"), "{folder}");
        std::fs::write(dir.path().join("lib/bad.js"), "\n\nexport { x as };").unwrap();
        let bad = load(dir.path(), "bad", None).unwrap_err();
        assert!(bad.starts_with("library lib/bad.js: "), "{bad}");
        assert!(bad.ends_with(" at lib/bad.js:3:15"), "{bad}");
    }

    #[test]
    fn names_resolve_under_lib_and_relative_paths_under_the_importer() {
        let p = |spec: &str, from: Option<&str>| resolve(spec, from).map(|p| to_id(&p));
        assert_eq!(p("drafting", None).unwrap(), "lib/drafting.js");
        assert_eq!(p("parts/joints.mjs", None).unwrap(), "lib/parts/joints.mjs");
        assert_eq!(p("./tools.js", None).unwrap(), "tools.js");
        assert_eq!(p("./geom", Some("lib/drafting.js")).unwrap(), "lib/geom.js");
        assert_eq!(p("../units", Some("lib/a/b.js")).unwrap(), "lib/units.js");
        assert_eq!(
            p("drafting", Some("lib/a/b.js")).unwrap(),
            "lib/drafting.js"
        );
        for bad in [
            "",
            "../x",
            "a/../../x",
            "/etc/passwd",
            "C:/x.js",
            "a\\b",
            ".hidden",
            "https://example.com/x.js",
        ] {
            assert!(p(bad, None).is_err(), "{bad} should be refused");
        }
        let escape = p("../../x.js", Some("lib/a.js")).unwrap_err();
        assert!(escape.contains("outside"), "{escape}");
        assert!(p("../x.js", None).is_err());
        assert!(p("./x", Some("../../y.js")).is_err());
    }
}
