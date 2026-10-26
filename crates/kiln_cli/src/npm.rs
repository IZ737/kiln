//! npm packages for embers: `kiln add npm:<name>@<version>` and what a
//! build makes of them.
//!
//! `kiln add` downloads a package's tarball from the registry, checks it
//! against the integrity the registry gives, and keeps in the program's
//! `npm/<name>/` folder only the files its browser entry imports, its
//! TypeScript declarations, `package.json` and its licence. It runs none
//! of the package's scripts. `npm.lock` records each package's version and
//! integrity and the SHA-256 of every file kept, and a build refuses files
//! that changed.
//!
//! The declarations become Kiln bindings: a module named after the
//! package (`canvas-confetti` is `canvas_confetti`) whose functions take
//! and give the types Kiln can map. `string` is `String`, `number` is
//! `Float`, `boolean` is `Bool`, `any` and `unknown` are `js.Value`, `T |
//! null` and optional parameters are optional, and arrays of strings,
//! numbers and booleans are lists. A function that returns a `Promise` is
//! waited for, as a `server fn` is. Every call can fail, as JavaScript
//! can throw. Exports with other types are left out, with why, which the
//! checker says when one is used.

use kiln_check::Foreign;
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha512};
use std::collections::BTreeMap;
use std::path::Path;

/// The file in a program's folder that lists its npm packages.
pub const LOCK: &str = "npm.lock";

/// A package a program uses, as a build needs it.
pub struct Package {
    /// Its Kiln name.
    pub name: String,
    pub bindings: Foreign,
    pub fns: Vec<Binding>,
    /// Where its entry module is served.
    pub entry_url: String,
    /// Its files, as the server serves them: URL path and contents.
    pub files: Vec<(String, Vec<u8>)>,
}

/// A function or constant of a package that Kiln can use.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// The Kiln name (`custom_alphabet`) and the JavaScript one.
    pub name: String,
    pub js: String,
    /// Each parameter's name and Kiln type, and whether it may be left out.
    pub params: Vec<(String, String, bool)>,
    /// The Kiln type it gives, if any.
    pub ret: Option<String>,
    /// Whether it returns a promise, which the handler waits for.
    pub waits: bool,
}

/// One package in `npm.lock`.
#[derive(Clone, Debug, PartialEq)]
struct Locked {
    package: String,
    version: String,
    integrity: String,
    entry: String,
    types: Option<String>,
    /// Each file kept, with `sha256-<hex>` of its contents.
    files: BTreeMap<String, String>,
}

/// A package's name as a Kiln name: `@scope/canvas-confetti` is
/// `scope_canvas_confetti`.
pub fn kiln_name(package: &str) -> String {
    let mut out = String::new();
    for c in package.trim_start_matches('@').chars() {
        out.push(if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase()
        } else {
            '_'
        });
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// `customAlphabet` is `custom_alphabet`.
fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

/// Whether `name` is an npm package name (`nanoid`, `@scope/name`), which
/// is also a safe folder name.
fn valid_name(name: &str) -> bool {
    let part = |p: &str| {
        !p.is_empty()
            && !p.starts_with('.')
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-._~".contains(c))
    };
    match name.strip_prefix('@').map(|n| n.split_once('/')) {
        Some(Some((scope, rest))) => part(scope) && part(rest),
        Some(None) => false,
        None => part(name),
    }
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256-{hex}")
}

// ----- `npm.lock` -----

fn read_lock(dir: &Path) -> Result<BTreeMap<String, Locked>, String> {
    let path = dir.join(LOCK);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let bad = || format!("{} is not a lock file kiln wrote", path.display());
    let value: Value = serde_json::from_str(&text).map_err(|_| bad())?;
    let mut out = BTreeMap::new();
    for (name, p) in value.as_object().ok_or_else(bad)? {
        let s = |k: &str| p[k].as_str().map(String::from).ok_or_else(bad);
        let files = p["files"]
            .as_object()
            .ok_or_else(bad)?
            .iter()
            .map(|(f, h)| Ok((f.clone(), h.as_str().ok_or_else(bad)?.to_string())))
            .collect::<Result<_, String>>()?;
        out.insert(
            name.clone(),
            Locked {
                package: s("package")?,
                version: s("version")?,
                integrity: s("integrity")?,
                entry: s("entry")?,
                types: p["types"].as_str().map(String::from),
                files,
            },
        );
    }
    Ok(out)
}

fn write_lock(dir: &Path, lock: &BTreeMap<String, Locked>) -> Result<(), String> {
    let mut value = serde_json::Map::new();
    for (name, p) in lock {
        value.insert(
            name.clone(),
            json!({
                "package": p.package,
                "version": p.version,
                "integrity": p.integrity,
                "entry": p.entry,
                "types": p.types,
                "files": p.files,
            }),
        );
    }
    let text = serde_json::to_string_pretty(&Value::Object(value)).expect("JSON") + "\n";
    let path = dir.join(LOCK);
    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

// ----- loading for a build -----

/// The npm packages of the program in `dir`, checked against `npm.lock`.
pub fn load(dir: &Path) -> Result<Vec<Package>, String> {
    let mut out = Vec::new();
    for (name, p) in read_lock(dir)? {
        if !valid_name(&p.package) {
            return Err(format!(
                "{LOCK} names `{}`, which is not an npm package name",
                p.package
            ));
        }
        let root = dir.join("npm").join(&p.package);
        let mut files = Vec::new();
        for (file, hash) in &p.files {
            let path = root.join(file);
            let body = std::fs::read(&path).map_err(|e| {
                format!(
                    "cannot read {} for the npm package {}: {e} (run `kiln add npm:{}@{}` again)",
                    path.display(),
                    p.package,
                    p.package,
                    p.version
                )
            })?;
            if &sha256(&body) != hash {
                return Err(format!(
                    "{} is not the file `kiln add` downloaded: its SHA-256 differs from {LOCK}'s (run `kiln add npm:{}@{}` again)",
                    path.display(),
                    p.package,
                    p.version
                ));
            }
            files.push((file.clone(), body));
        }
        let declarations = match &p.types {
            Some(t) => files
                .iter()
                .find(|(f, _)| f == t)
                .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default(),
            None => String::new(),
        };
        let (fns, unsupported) = bindings(&declarations);
        let base = format!("/_kiln/npm/{}@{}", p.package, p.version);
        out.push(Package {
            bindings: Foreign {
                name: name.clone(),
                source: binding_source(&p, &fns),
                unsupported,
            },
            name,
            fns,
            entry_url: format!("{base}/{}", p.entry),
            files: files
                .into_iter()
                .filter(|(f, _)| f.ends_with(".js") || f.ends_with(".mjs"))
                .map(|(f, b)| (format!("{base}/{f}"), b))
                .collect(),
        });
    }
    Ok(out)
}

/// The Kiln declarations of a package's bindings, for the checker.
fn binding_source(p: &Locked, fns: &[Binding]) -> String {
    let mut out = format!(
        "// The npm package {} {}, from its TypeScript declarations.\n",
        p.package, p.version
    );
    for f in fns {
        out.push_str(&format!(
            "fn {}({}){} ! {{}}\n",
            f.name,
            params_text(f),
            ret_text(f)
        ));
    }
    out
}

fn params_text(f: &Binding) -> String {
    f.params
        .iter()
        .map(|(n, t, optional)| {
            if *optional {
                format!("{n}: {t} = none")
            } else {
                format!("{n}: {t}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn ret_text(f: &Binding) -> String {
    f.ret
        .as_ref()
        .map_or_else(String::new, |r| format!(" -> {r}"))
}

/// The Kiln functions that call a package's functions that do not wait,
/// for the browser's program: `nanoid.nanoid(size)` calls
/// `__npm__nanoid__nanoid(size)`, which sends the arguments to the bridge
/// as JSON and reads the result.
pub fn calls_source(packages: &[Package]) -> String {
    let mut out = String::new();
    for p in packages {
        for f in p.fns.iter().filter(|f| !f.waits) {
            let fields: Vec<String> = f
                .params
                .iter()
                .map(|(n, _, _)| format!("{n}: {n}"))
                .collect();
            let args = if fields.is_empty() {
                "\"{}\"".to_string()
            } else {
                format!("json.encode({{ {} }})", fields.join(", "))
            };
            out.push_str(&format!(
                "\nfn __npm__{}__{}({}){} ! {{\n    let __reply = try ember.npm({}, {}, {args})\n",
                p.name,
                f.name,
                params_text(f),
                ret_text(f),
                json!(p.name),
                json!(f.js),
            ));
            if let Some(ret) = &f.ret {
                out.push_str(&format!(
                    "    let __value = try json.decode<{{ ok: {ret} }}>(__reply)\n    return __value.ok\n"
                ));
            }
            out.push_str("}\n");
        }
    }
    out
}

/// The package functions handlers wait for, as the glaze compiler takes
/// them.
pub fn waits(packages: &[Package]) -> Vec<kiln_ir::glaze::ServerFn> {
    let mut out = Vec::new();
    for p in packages {
        for f in p.fns.iter().filter(|f| f.waits) {
            out.push(kiln_ir::glaze::ServerFn {
                module: Some(p.name.clone()),
                name: f.name.clone(),
                params: f
                    .params
                    .iter()
                    .map(|(n, t, optional)| {
                        (n.clone(), t.clone(), optional.then(|| "none".to_string()))
                    })
                    .collect(),
                ret: f.ret.clone(),
                throws: true,
            });
        }
    }
    out
}

/// The packages as the bridge loads them.
pub fn modules(packages: &[Package]) -> Vec<kiln_wasm::NpmModule> {
    packages
        .iter()
        .map(|p| kiln_wasm::NpmModule {
            name: p.name.clone(),
            url: p.entry_url.clone(),
            names: p
                .fns
                .iter()
                .map(|f| (f.name.clone(), f.js.clone()))
                .collect(),
        })
        .collect()
}

// ----- TypeScript declarations -----

/// The exports of a `.d.ts` file that Kiln can use, and the others with
/// why it cannot.
pub fn bindings(declarations: &str) -> (Vec<Binding>, Vec<(String, String)>) {
    let text = strip_comments(declarations);
    let mut fns: Vec<Binding> = Vec::new();
    let mut unsupported: Vec<(String, String)> = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for stmt in statements(&text) {
        let words: Vec<&str> = stmt.split_whitespace().collect();
        let mut i = 0;
        let mut exported = false;
        while i < words.len() && matches!(words[i], "export" | "declare" | "default") {
            exported |= words[i] == "export";
            i += 1;
        }
        if !exported || i >= words.len() {
            continue;
        }
        let kind = words[i];
        let rest = stmt[stmt.find(kind).map_or(0, |at| at + kind.len())..].trim();
        let (js, result) = match kind {
            "function" => {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    .collect();
                (name.clone(), function(&name, &rest[name.len()..]))
            }
            "const" | "let" | "var" => {
                let Some((name, ty)) = rest.split_once(':') else {
                    continue;
                };
                let name = name.trim().to_string();
                let result = match map_type(ty.trim().trim_end_matches(';'), false) {
                    Ok(ret) => Ok(Binding {
                        name: snake_case(&name),
                        js: name.clone(),
                        params: Vec::new(),
                        ret: Some(ret),
                        waits: false,
                    }),
                    Err(why) => Err(format!("its type is {why}")),
                };
                (name, result)
            }
            _ => continue,
        };
        if js.is_empty() {
            continue;
        }
        *seen.entry(js.clone()).or_default() += 1;
        match result {
            Ok(b) => fns.push(b),
            Err(why) => unsupported.push((snake_case(&js), why)),
        }
    }
    // A function declared more than once has overloads, which Kiln cannot
    // choose between.
    for (js, n) in seen {
        if n > 1 {
            fns.retain(|f| f.js != js);
            unsupported.retain(|(k, _)| *k != snake_case(&js));
            unsupported.push((snake_case(&js), "it has overloads".into()));
        }
    }
    unsupported.sort();
    unsupported.dedup();
    (fns, unsupported)
}

/// `(size?: number): string` as a binding of `name`.
fn function(name: &str, sig: &str) -> Result<Binding, String> {
    let sig = sig.trim();
    if sig.starts_with('<') {
        return Err("it is generic".into());
    }
    let Some(close) = matching(sig, 0) else {
        return Err("its declaration is not one Kiln reads".into());
    };
    let params_text = &sig[1..close];
    let ret_text = sig[close + 1..]
        .trim()
        .trim_start_matches(':')
        .trim()
        .trim_end_matches(';');
    let mut params = Vec::new();
    for p in split_top(params_text, ',') {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if p.starts_with("...") {
            return Err("it takes any number of arguments".into());
        }
        let (pname, ty) = p.split_once(':').unwrap_or((p, "any"));
        let (pname, optional) = match pname.trim().strip_suffix('?') {
            Some(n) => (n.trim(), true),
            None => (pname.trim(), false),
        };
        let mut ty = map_type(ty.trim(), false)
            .map_err(|why| format!("its parameter `{pname}` is {why}"))?;
        if optional && !ty.ends_with('?') {
            ty.push('?');
        }
        params.push((snake_case(pname), ty, optional || false));
    }
    // Optional parameters can be left out, so they come last in Kiln too.
    if let Some(first) = params.iter().position(|p| p.2) {
        if params[first..].iter().any(|p| !p.2) {
            return Err("an optional parameter comes before a required one".into());
        }
    }
    let (ret_text, waits) = match strip_generic(ret_text, "Promise") {
        Some(inner) => (inner, true),
        None => (ret_text, false),
    };
    let ret = match ret_text.trim() {
        "void" | "undefined" => None,
        t => Some(map_type(t, false).map_err(|why| format!("it returns {why}"))?),
    };
    Ok(Binding {
        name: snake_case(name),
        js: name.to_string(),
        params,
        ret,
        waits,
    })
}

/// A TypeScript type as a Kiln one, or why there is none.
fn map_type(ty: &str, inner: bool) -> Result<String, String> {
    let ty = ty.trim();
    let alternatives = split_top(ty, '|');
    if alternatives.len() > 1 {
        let real: Vec<&str> = alternatives
            .iter()
            .map(|a| a.trim())
            .filter(|a| !matches!(*a, "null" | "undefined" | ""))
            .collect();
        if real.len() == 1 && real.len() < alternatives.len() {
            let t = map_type(real[0], true)?;
            return Ok(if t.ends_with('?') { t } else { format!("{t}?") });
        }
        return Err(format!("`{ty}`, a union Kiln cannot map yet"));
    }
    let element = ty
        .strip_suffix("[]")
        .map(|e| e.trim_start_matches("readonly ").trim())
        .or_else(|| strip_generic(ty, "Array"))
        .or_else(|| strip_generic(ty, "ReadonlyArray"));
    if let Some(e) = element {
        let e = e.trim().trim_start_matches('(').trim_end_matches(')');
        return match map_type(e, true)?.as_str() {
            t @ ("String" | "Float" | "Bool") => Ok(format!("List<{t}>")),
            _ => Err(format!("`{ty}`, an array Kiln cannot map yet")),
        };
    }
    Ok(match ty {
        "string" => "String".into(),
        "number" => "Float".into(),
        "boolean" => "Bool".into(),
        "any" | "unknown" if !inner => "js.Value".into(),
        _ if ty.contains("=>") => return Err(format!("`{ty}`, a function")),
        _ => return Err(format!("`{ty}`, which Kiln cannot map yet")),
    })
}

/// `inner` of `Name<inner>`.
fn strip_generic<'t>(ty: &'t str, name: &str) -> Option<&'t str> {
    let rest = ty.trim().strip_prefix(name)?.trim_start();
    let inner = rest.strip_prefix('<')?.strip_suffix('>')?;
    Some(inner)
}

/// The index of the bracket that closes the one at `open`.
fn matching(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in text.char_indices().skip(open) {
        match c {
            '(' | '<' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '>' if !text[..i].ends_with('=') => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return Some(i);
        }
    }
    None
}

/// `text` split at `sep` outside brackets.
fn split_top(text: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '<' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '>' if !text[..i].ends_with('=') => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

fn strip_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map_or("", |end| &r[end + 2..]);
            out.push(' ');
        } else if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('\n').map_or("", |end| &r[end..]);
        } else {
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The top-level statements of declarations: each ends at a `;` or a line
/// break outside brackets, and a block (`interface`, `namespace`) is one.
fn statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '>' if !current.ends_with('=') => depth -= 1,
            _ => {}
        }
        let ends = depth <= 0 && (c == ';' || (c == '\n' && !continues(&current)) || c == '}');
        current.push(c);
        if ends {
            let stmt = current.trim().trim_end_matches(';').trim().to_string();
            if !stmt.is_empty() {
                out.push(stmt);
            }
            current.clear();
            depth = 0;
        }
    }
    let stmt = current.trim().to_string();
    if !stmt.is_empty() {
        out.push(stmt);
    }
    out
}

/// Whether a declaration goes on past the end of this line.
fn continues(current: &str) -> bool {
    let t = current.trim_end();
    t.is_empty() || t.ends_with(':') || t.ends_with('|') || t.ends_with(',') || t.ends_with("=>")
}

// ----- `kiln add` -----

/// `kiln add npm:<name>[@<version>]`: downloads the package into the
/// program in `dir` and records it in `npm.lock`.
pub fn add(spec: &str, dir: &Path) -> Result<String, String> {
    let spec = spec.strip_prefix("npm:").ok_or_else(|| {
        format!(
            "expected `npm:<package>@<version>`, found `{spec}`: kiln adds npm packages for embers"
        )
    })?;
    let (package, version) = match spec[1..].find('@') {
        Some(at) => (&spec[..at + 1], Some(&spec[at + 2..])),
        None => (spec, None),
    };
    if !valid_name(package) {
        return Err(format!("`{package}` is not an npm package name"));
    }
    let registry =
        std::env::var("KILN_NPM_REGISTRY").unwrap_or_else(|_| "https://registry.npmjs.org".into());
    let registry = registry.trim_end_matches('/');
    let agent = crate::upgrade::agent(std::time::Duration::from_secs(60))?;
    let meta_url = format!("{registry}/{}", package.replace('/', "%2f"));
    let meta = crate::upgrade::get(&agent, &meta_url)?
        .ok_or_else(|| format!("there is no npm package named `{package}`"))?;
    let meta: Value = serde_json::from_slice(&meta)
        .map_err(|_| format!("{meta_url} did not answer with JSON"))?;
    let version = match version {
        Some(v) => v.to_string(),
        None => meta["dist-tags"]["latest"]
            .as_str()
            .ok_or_else(|| format!("`{package}` has no latest version"))?
            .to_string(),
    };
    let info = &meta["versions"][&version];
    if info.is_null() {
        return Err(format!("`{package}` has no version {version}"));
    }
    let integrity = info["dist"]["integrity"]
        .as_str()
        .ok_or_else(|| format!("the registry gives no integrity for {package}@{version}"))?
        .to_string();
    let tarball_url = info["dist"]["tarball"]
        .as_str()
        .ok_or_else(|| format!("the registry gives no tarball for {package}@{version}"))?;
    let tarball = crate::upgrade::get(&agent, tarball_url)?
        .ok_or_else(|| format!("{tarball_url} is not there"))?;
    check_integrity(&tarball, &integrity)?;
    let files = unpack(&tarball)?;
    let locked = keep(package, &version, &integrity, &files)?;
    let root = dir.join("npm").join(package);
    let _ = std::fs::remove_dir_all(&root);
    for file in locked.files.keys() {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap())
            .and_then(|_| std::fs::write(&path, &files[file]))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    let mut lock = read_lock(dir)?;
    lock.insert(kiln_name(package), locked.clone());
    write_lock(dir, &lock)?;
    let declarations = locked
        .types
        .as_ref()
        .map(|t| String::from_utf8_lossy(&files[t]).into_owned())
        .unwrap_or_default();
    let (fns, unsupported) = bindings(&declarations);
    let mut summary = format!(
        "added {package} {version} as `{}`: {}",
        kiln_name(package),
        if fns.is_empty() {
            "nothing Kiln can call".to_string()
        } else {
            fns.iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    if locked.types.is_none() {
        summary.push_str(
            "\nit has no TypeScript declarations, so Kiln cannot type it: use `js.Value`",
        );
    }
    for (name, why) in unsupported {
        summary.push_str(&format!("\nleft out `{name}`: {why}"));
    }
    Ok(summary)
}

fn check_integrity(tarball: &[u8], integrity: &str) -> Result<(), String> {
    use base64::Engine;
    let Some(expected) = integrity.strip_prefix("sha512-") else {
        return Err(format!(
            "kiln checks sha512 integrity, and the registry gives `{integrity}`"
        ));
    };
    let found = base64::engine::general_purpose::STANDARD.encode(Sha512::digest(tarball));
    if found != expected {
        return Err("the tarball does not match the integrity the registry gives".into());
    }
    Ok(())
}

/// The files of a tarball, by their paths inside its `package/` folder.
fn unpack(tarball: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    use std::io::Read;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tarball));
    let mut out = BTreeMap::new();
    for entry in archive
        .entries()
        .map_err(|e| format!("cannot read the tarball: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("cannot read the tarball: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("cannot read the tarball: {e}"))?;
        let path = path.to_string_lossy().replace('\\', "/");
        // Every path is inside the package's folder, whatever it is named.
        let Some((_, inner)) = path.split_once('/') else {
            continue;
        };
        if inner.split('/').any(|part| part == ".." || part.is_empty()) {
            return Err(format!("the tarball has a file outside its folder: {path}"));
        }
        let mut body = Vec::new();
        entry
            .read_to_end(&mut body)
            .map_err(|e| format!("cannot read the tarball: {e}"))?;
        out.insert(inner.to_string(), body);
    }
    Ok(out)
}

/// What `kiln add` keeps of a package: its browser entry and the modules
/// it imports, its declarations, `package.json` and its licence.
fn keep(
    package: &str,
    version: &str,
    integrity: &str,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Locked, String> {
    let manifest: Value = files
        .get("package.json")
        .and_then(|b| serde_json::from_slice(b).ok())
        .ok_or_else(|| format!("{package}@{version} has no package.json"))?;
    let entry = entry(&manifest).ok_or_else(|| {
        format!("{package}@{version} names no module for browsers: Kiln loads packages that are ES modules")
    })?;
    let entry = normalize(&entry);
    let mut keep = vec![entry.clone()];
    let mut i = 0;
    while i < keep.len() {
        let file = keep[i].clone();
        i += 1;
        let body = files
            .get(&file)
            .ok_or_else(|| format!("{package}@{version} has no {file}, which it imports"))?;
        let text = String::from_utf8_lossy(body);
        if file == entry && !looks_like_module(&text) {
            return Err(format!(
                "{package}@{version} is CommonJS: Kiln loads packages that are ES modules"
            ));
        }
        for spec in imports(&text) {
            if spec.starts_with("./") || spec.starts_with("../") {
                let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
                let target = normalize(&format!("{dir}/{spec}"));
                if !keep.contains(&target) {
                    keep.push(target);
                }
            } else if is_node_builtin(&spec) {
                return Err(format!(
                    "{package}@{version} imports `{spec}`, which Node.js has and browsers do not"
                ));
            } else {
                return Err(format!("{package}@{version} imports the package `{spec}`: Kiln adds packages without dependencies for now"));
            }
        }
    }
    let types = ["types", "typings"]
        .iter()
        .find_map(|k| manifest[*k].as_str())
        .map(normalize)
        .or_else(|| {
            let guess = entry
                .trim_end_matches(".mjs")
                .trim_end_matches(".js")
                .to_string()
                + ".d.ts";
            files.contains_key(&guess).then_some(guess)
        })
        .or_else(|| {
            files
                .contains_key("index.d.ts")
                .then(|| "index.d.ts".to_string())
        })
        .filter(|t| files.contains_key(t));
    keep.extend(types.clone());
    keep.push("package.json".into());
    for licence in ["LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE", "license"] {
        if files.contains_key(licence) {
            keep.push(licence.into());
        }
    }
    Ok(Locked {
        package: package.into(),
        version: version.into(),
        integrity: integrity.into(),
        entry,
        types,
        files: keep
            .iter()
            .map(|f| (f.clone(), sha256(&files[f])))
            .collect(),
    })
}

/// The module a package gives browsers: from `exports` (preferring the
/// `browser`, then `import`, then `default` condition), `browser`,
/// `module` or `main`.
fn entry(manifest: &Value) -> Option<String> {
    fn pick(v: &Value) -> Option<String> {
        match v {
            Value::String(s) => Some(s.clone()),
            Value::Object(m) => ["browser", "import", "module", "default"]
                .iter()
                .find_map(|k| m.get(*k).and_then(pick)),
            _ => None,
        }
    }
    let exports = &manifest["exports"];
    let from_exports = match exports {
        Value::Object(m) if m.contains_key(".") => pick(&m["."]),
        Value::Object(m) if m.keys().all(|k| !k.starts_with('.')) => pick(exports),
        Value::String(_) => pick(exports),
        _ => None,
    };
    from_exports
        .or_else(|| manifest["browser"].as_str().map(String::from))
        .or_else(|| manifest["module"].as_str().map(String::from))
        .or_else(|| manifest["main"].as_str().map(String::from))
        .or_else(|| Some("index.js".into()))
}

/// `./a/../b.js` is `b.js`.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn looks_like_module(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("export ") || l.starts_with("import ") || l.starts_with("export{")
    })
}

/// The modules a module imports: `from '...'`, `import '...'` and
/// `import('...')`.
fn imports(text: &str) -> Vec<String> {
    let text = strip_comments(text);
    let mut out = Vec::new();
    for marker in ["from", "import"] {
        let mut rest = text.as_str();
        while let Some(at) = rest.find(marker) {
            let before = rest[..at].chars().last();
            let after = rest[at + marker.len()..].trim_start();
            let after = after.strip_prefix('(').map_or(after, str::trim_start);
            rest = &rest[at + marker.len()..];
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == '.') {
                continue;
            }
            let Some(q) = after.chars().next().filter(|c| matches!(c, '\'' | '"')) else {
                continue;
            };
            if let Some(end) = after[1..].find(q) {
                let spec = after[1..end + 1].to_string();
                if !out.contains(&spec) {
                    out.push(spec);
                }
            }
        }
    }
    out
}

fn is_node_builtin(spec: &str) -> bool {
    const BUILTINS: &[&str] = &[
        "assert",
        "buffer",
        "child_process",
        "cluster",
        "crypto",
        "dgram",
        "dns",
        "events",
        "fs",
        "http",
        "http2",
        "https",
        "net",
        "os",
        "path",
        "process",
        "readline",
        "stream",
        "tls",
        "tty",
        "url",
        "util",
        "v8",
        "vm",
        "worker_threads",
        "zlib",
    ];
    spec.starts_with("node:") || BUILTINS.contains(&spec.split('/').next().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_become_bindings() {
        let (fns, unsupported) = bindings(
            "/** doc */\nexport function nanoid(size?: number): string\n\
             export function customAlphabet(\n  alphabet: string,\n  defaultSize?: number\n): (size?: number) => string\n\
             export const urlAlphabet: string\n\
             export declare function load(url: string): Promise<string | null>;\n\
             export function pick(items: string[], seed?: number | undefined): string[]\n\
             export function twice(a: string): string\nexport function twice(a: number): number\n\
             export interface Options { a: number }\n\
             export function configure(options: Options): void\n",
        );
        let names: Vec<&str> = fns.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["nanoid", "url_alphabet", "load", "pick"]);
        assert_eq!(fns[0].params, [("size".into(), "Float?".into(), true)]);
        assert_eq!(fns[0].ret.as_deref(), Some("String"));
        assert!(fns[2].waits);
        assert_eq!(fns[2].ret.as_deref(), Some("String?"));
        assert_eq!(fns[3].params[0].1, "List<String>");
        assert_eq!(fns[3].params[1].1, "Float?");
        let why: BTreeMap<_, _> = unsupported.into_iter().collect();
        assert_eq!(
            why["custom_alphabet"],
            "it returns `(size?: number) => string`, a function"
        );
        assert_eq!(why["twice"], "it has overloads");
        assert_eq!(
            why["configure"],
            "its parameter `options` is `Options`, which Kiln cannot map yet"
        );
    }

    #[test]
    fn imports_are_found() {
        let found = imports("import { a } from './a.js'\nexport { b } from \"../b.js\"\nimport './c.js'\nconst d = await import('./d.js')\nlet from = 1\n");
        assert_eq!(found, ["./a.js", "../b.js", "./c.js", "./d.js"]);
        assert_eq!(normalize("x/../y/./z.js"), "y/z.js");
        assert_eq!(kiln_name("@scope/canvas-confetti"), "scope_canvas_confetti");
    }
}
