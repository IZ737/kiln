//! The LLVM backend of `kiln fire`: writes a program in the IR as LLVM
//! IR text (`.ll`), which a `clang` on the system turns into an optimized
//! object file. The object then goes through the same link as one from
//! Cranelift ([`crate::link`]): kiln's own linker and the runtime kiln
//! carries, so a release build needs clang and nothing else, and the
//! executable it makes is as self-contained as a `kiln build` one.
//!
//! Values are stored as the Cranelift backend (`gen.rs`) stores them:
//! `Int` and `Duration` as `i64`, `Float` as `double`, `Bool` as `i1`,
//! and pointers and `kv` slots as `i64`, so the runtime sees the same
//! calls and the same type descriptions from both. Every local lives in
//! an `alloca`, which LLVM's optimizer turns back into registers.
//!
//! `kiln` does not link LLVM in: it would make the one binary the
//! installers ship a hundred megabytes larger and its static and
//! cross builds much harder. Clang is found with [`find_clang`].

use crate::runtime::{self, C};
use crate::Target;
use kiln_check::types::substitute;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_ir::*;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The oldest clang that reads the IR this writes (opaque pointers).
pub const MIN_CLANG: u32 = 15;

/// A clang that [`compile`] can run.
#[derive(Clone, Debug)]
pub struct Clang {
    pub path: PathBuf,
    pub version: u32,
}

/// Finds a clang: `KILN_CLANG` when it is set, otherwise `clang` and
/// `clang-N` on the `PATH`, newest first. Says what is missing when there
/// is none that is new enough.
pub fn find_clang() -> Result<Clang, String> {
    let need = format!("clang {MIN_CLANG} or later");
    if let Some(path) = std::env::var_os("KILN_CLANG") {
        let path = PathBuf::from(path);
        return match clang_version(&path) {
            Some(version) if version >= MIN_CLANG => Ok(Clang { path, version }),
            Some(version) => Err(format!(
                "KILN_CLANG is {} (clang {version}), and kiln needs {need}",
                path.display()
            )),
            None => Err(format!(
                "KILN_CLANG is {}, which is not a clang that runs",
                path.display()
            )),
        };
    }
    let mut names = vec!["clang".to_string()];
    names.extend((MIN_CLANG..=30).rev().map(|v| format!("clang-{v}")));
    let mut old = None;
    for name in names {
        let path = PathBuf::from(&name);
        match clang_version(&path) {
            Some(version) if version >= MIN_CLANG => return Ok(Clang { path, version }),
            Some(version) => old = old.or(Some((name, version))),
            None => {}
        }
    }
    Err(match old {
        Some((name, version)) => {
            format!("`{name}` is clang {version}, and kiln needs {need}; install it, or set KILN_CLANG to one")
        }
        None => format!(
            "no clang was found on the PATH; install {need} (`apt install clang`, `brew install llvm`, or the LLVM release for Windows), or set KILN_CLANG to one"
        ),
    })
}

/// The major version of the clang at `path`, if it runs.
fn clang_version(path: &Path) -> Option<u32> {
    let out = Command::new(path).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let after = text.split("clang version ").nth(1)?;
    after
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// Compiles the LLVM IR `ll` into an object file for `target` with
/// `clang`, optimized.
pub fn compile(clang: &Clang, ll: &str, target: Target) -> Result<Vec<u8>, String> {
    let dir = scratch_dir()?;
    let result = compile_in(&dir, clang, ll, target);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn compile_in(dir: &Path, clang: &Clang, ll: &str, target: Target) -> Result<Vec<u8>, String> {
    let src = dir.join("program.ll");
    let obj = dir.join("program.o");
    std::fs::write(&src, ll).map_err(|e| format!("cannot write {}: {e}", src.display()))?;
    let out = Command::new(&clang.path)
        .args(["-c", "-x", "ir", "-O2", "-fno-pic"])
        .arg(format!("--target={}", triple(target)))
        .arg("-o")
        .arg(&obj)
        .arg(&src)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", clang.path.display()))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "clang ({}) could not compile the program:\n{}",
            clang.path.display(),
            stderr.trim_end()
        ));
    }
    std::fs::read(&obj).map_err(|e| format!("cannot read {}: {e}", obj.display()))
}

fn triple(target: Target) -> &'static str {
    match target {
        Target::Linux => "x86_64-unknown-linux-musl",
        Target::Windows => "x86_64-pc-windows-gnu",
    }
}

/// A fresh directory for build files.
fn scratch_dir() -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("kiln-llvm-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Writes a program as LLVM IR for `target`. Returns diagnostics for the
/// parts the backend does not support yet (E0401), the same ones the
/// Cranelift backend reports.
pub fn emit(program: &Program, target: Target) -> Result<String, Vec<Diagnostic>> {
    let mut gen = Gen::new(program, target);
    gen.program();
    if !gen.diags.is_empty() {
        let mut diags = gen.diags;
        diags.sort_by_key(|d| (d.span.start, d.span.end));
        diags.dedup_by(|a, b| a.span == b.span && a.message == b.message);
        return Err(diags);
    }
    Ok(gen.finish())
}

/// How a type is stored.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rep {
    Int,
    Float,
    Bool,
    Ptr,
    Kv,
    Void,
}

/// The LLVM types values are held in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum T {
    I1,
    I32,
    I64,
    F64,
}

impl T {
    fn name(self) -> &'static str {
        match self {
            T::I1 => "i1",
            T::I32 => "i32",
            T::I64 => "i64",
            T::F64 => "double",
        }
    }

    /// The type in a signature: a `bool` is zero-extended, as C does.
    fn abi(self) -> &'static str {
        match self {
            T::I1 => "i1 zeroext",
            other => other.name(),
        }
    }

    fn zero(self) -> &'static str {
        match self {
            T::I1 => "false",
            T::F64 => "0.0",
            _ => "0",
        }
    }
}

/// A value: its text in an instruction (a register or a constant) and
/// its type.
#[derive(Clone)]
struct V {
    text: String,
    ty: T,
}

impl V {
    fn new(text: impl Into<String>, ty: T) -> V {
        V {
            text: text.into(),
            ty,
        }
    }

    fn int(n: i64) -> V {
        V::new(n.to_string(), T::I64)
    }

    fn bool(b: bool) -> V {
        V::new(if b { "true" } else { "false" }, T::I1)
    }

    /// `type value`, as an operand.
    fn op(&self) -> String {
        format!("{} {}", self.ty.name(), self.text)
    }
}

/// A function's signature.
#[derive(Clone, PartialEq, Eq)]
struct Sig {
    params: Vec<T>,
    ret: Option<T>,
}

impl Sig {
    fn new(params: &[T], ret: Option<T>) -> Sig {
        Sig {
            params: params.to_vec(),
            ret,
        }
    }

    /// The result in a declaration or a call: a `bool` is zero-extended,
    /// as C does, and the attribute goes before the type there.
    fn ret_name(&self) -> &'static str {
        match self.ret {
            None => "void",
            Some(T::I1) => "zeroext i1",
            Some(t) => t.name(),
        }
    }

    /// The function type, as an indirect call names it: `i64 (i64, i64)`.
    fn fn_type(&self) -> String {
        let params: Vec<&str> = self.params.iter().map(|t| t.name()).collect();
        format!(
            "{} ({})",
            self.ret.map_or("void", T::name),
            params.join(", ")
        )
    }
}

/// The kinds of type descriptions, as `runtime/native.c` numbers them.
mod desc {
    pub const UNKNOWN: i64 = 0;
    pub const STR: i64 = 1;
    pub const INT: i64 = 2;
    pub const FLOAT: i64 = 3;
    pub const BOOL: i64 = 4;
    pub const DURATION: i64 = 5;
    pub const OPT: i64 = 6;
    pub const FN: i64 = 7;
    pub const LIST: i64 = 8;
    pub const MAP: i64 = 9;
    pub const SET: i64 = 10;
    pub const STRUCT: i64 = 11;
    pub const ENUM: i64 = 12;
    pub const RECORD: i64 = 13;
    pub const VARIANT: i64 = 14;
    pub const BYTES: i64 = 15;
}

/// What the runtime does with a type: show, compare, hash or order it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Op {
    Show,
    Eq,
    Hash,
    Cmp,
}

struct Gen<'a> {
    p: &'a Program,
    prims: Prims,
    target: Target,
    diags: Vec<Diagnostic>,
    /// Global definitions: strings, descriptions, the globals array.
    globals: String,
    /// The generated functions.
    bodies: String,
    /// The symbol of each IR function, with its signature.
    funcs: Vec<(String, Sig)>,
    /// Runtime functions used, with their signatures.
    imports: HashMap<&'static str, Sig>,
    data_imports: HashSet<&'static str>,
    /// Intrinsics used, by their declaration.
    intrinsics: HashSet<&'static str>,
    /// String constants by their bytes: the symbol and the length.
    strings: HashMap<Vec<u8>, (String, usize)>,
    descs: HashMap<String, String>,
    /// Helper functions by what they do, such as `show:true:<type>`.
    helpers: HashMap<String, (String, Sig)>,
    /// The types already checked for each operation.
    checked: HashSet<(Op, String)>,
    counter: usize,
}

impl<'a> Gen<'a> {
    fn new(p: &'a Program, target: Target) -> Self {
        Gen {
            p,
            prims: p.prims,
            target,
            diags: Vec::new(),
            globals: String::new(),
            bodies: String::new(),
            funcs: Vec::new(),
            imports: HashMap::new(),
            data_imports: HashSet::new(),
            intrinsics: HashSet::new(),
            strings: HashMap::new(),
            descs: HashMap::new(),
            helpers: HashMap::new(),
            checked: HashSet::new(),
            counter: 0,
        }
    }

    fn next(&mut self) -> usize {
        self.counter += 1;
        self.counter
    }

    /// The whole module.
    fn finish(self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "target triple = \"{}\"", triple(self.target));
        out.push('\n');
        out.push_str(&self.globals);
        let mut imports: Vec<_> = self.imports.iter().collect();
        imports.sort_by_key(|(name, _)| **name);
        for (name, sig) in imports {
            let params: Vec<&str> = sig.params.iter().map(|t| t.abi()).collect();
            let _ = writeln!(
                out,
                "declare {} @{name}({})",
                sig.ret_name(),
                params.join(", ")
            );
        }
        let mut data: Vec<_> = self.data_imports.iter().collect();
        data.sort();
        for name in data {
            let ty = if matches!(*name, "k_failed" | "k_gc_pending") {
                "i8"
            } else {
                "i64"
            };
            let _ = writeln!(out, "@{name} = external global {ty}");
        }
        let mut intrinsics: Vec<_> = self.intrinsics.iter().collect();
        intrinsics.sort();
        for decl in intrinsics {
            let _ = writeln!(out, "{decl}");
        }
        out.push('\n');
        out.push_str(&self.bodies);
        let _ = writeln!(out, "attributes #0 = {{ nounwind {} }}", self.probe_attr());
        out
    }

    /// Windows grows the stack one guard page at a time, so a frame larger
    /// than a page must touch each page in turn; the runtime has no
    /// `___chkstk_ms` for LLVM to call, so the probes are inlined.
    fn probe_attr(&self) -> &'static str {
        match self.target {
            Target::Linux => "",
            Target::Windows => "\"probe-stack\"=\"inline-asm\"",
        }
    }

    // ----- the program -----

    fn program(&mut self) {
        let _ = writeln!(
            self.globals,
            "@kiln_globals = global [{} x i64] zeroinitializer, align 8",
            self.p.globals.len().max(1)
        );
        for id in 0..self.p.funcs.len() {
            let func = &self.p.funcs[id];
            let name = if id == self.p.init {
                "kiln_init".to_string()
            } else if id == self.p.main {
                "kiln_main".to_string()
            } else {
                format!("kf_{id}_{}", mangle(&func.name))
            };
            let sig = self.func_sig(id);
            self.funcs.push((name, sig));
        }
        for id in 0..self.p.funcs.len() {
            self.function(id);
        }
    }

    fn func_sig(&self, id: FuncId) -> Sig {
        let func = &self.p.funcs[id];
        if func.kind == FuncKind::Closure {
            return Sig::new(&[T::I64, T::I64], Some(T::I64));
        }
        let params: Vec<T> = func
            .params
            .iter()
            .map(|&l| self.cty(&func.locals[l].ty))
            .collect();
        let ret = (!is_void(&func.ret)).then(|| self.cty(&func.ret));
        Sig::new(&params, ret)
    }

    /// Generates a function's body with `body`.
    fn define(
        &mut self,
        name: &str,
        internal: bool,
        signature: Sig,
        body: impl FnOnce(&mut Fx<'_, 'a>),
    ) {
        let params: Vec<V> = signature
            .params
            .iter()
            .enumerate()
            .map(|(i, &t)| V::new(format!("%p{i}"), t))
            .collect();
        let mut fx = Fx {
            g: self,
            body: String::new(),
            allocas: String::new(),
            params,
            locals: Vec::new(),
            tys: Vec::new(),
            ret: Ty::Unit,
            closure: false,
            loops: Vec::new(),
            counter: 0,
            terminated: false,
            frame: None,
            frame_alloca: String::new(),
            prologue: String::new(),
        };
        body(&mut fx);
        if !fx.terminated {
            fx.emit("unreachable");
        }
        fx.finish_frame();
        let Fx {
            body,
            allocas,
            frame_alloca,
            prologue,
            ..
        } = fx;
        let linkage = if internal { "internal " } else { "" };
        let params: Vec<String> = signature
            .params
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{} %p{i}", t.abi()))
            .collect();
        let _ = writeln!(
            self.bodies,
            "define {linkage}{} @{name}({}) #0 {{\nentry:",
            signature.ret_name(),
            params.join(", ")
        );
        self.bodies.push_str(&frame_alloca);
        self.bodies.push_str(&allocas);
        self.bodies.push_str(&prologue);
        self.bodies.push_str(&body);
        self.bodies.push_str("}\n\n");
    }

    fn function(&mut self, id: FuncId) {
        let p = self.p;
        let func = &p.funcs[id];
        let (name, signature) = self.funcs[id].clone();
        let internal = id != p.init && id != p.main;
        self.define(&name, internal, signature, |fx| {
            let closure = func.kind == FuncKind::Closure;
            fx.ret = func.ret.clone();
            fx.closure = closure;
            fx.tys = func.locals.iter().map(|l| l.ty.clone()).collect();
            fx.enter_frame();
            if id == p.init {
                let addr = fx.data_addr("@kiln_globals");
                let n = V::int(p.globals.len() as i64);
                fx.call_rt("k_gc_add_roots", &[addr, n]);
            }
            for local in &func.locals {
                let ty = fx.g.cty(&local.ty);
                // A local holding a pointer or a `kv` (or the cell of one)
                // lives in a slot of the frame, so it is a root as it is.
                let rooted = matches!(fx.g.rep(&local.ty), Rep::Ptr | Rep::Kv);
                let slot = if local.cell {
                    let a = if rooted {
                        fx.frame_slot()
                    } else {
                        fx.alloca(T::I64)
                    };
                    fx.store_to(&V::int(0), &a);
                    Slot::Cell(a, ty)
                } else {
                    let a = if rooted {
                        fx.frame_slot()
                    } else {
                        fx.alloca(ty)
                    };
                    let zero = V::new(ty.zero(), ty);
                    fx.store_to(&zero, &a);
                    Slot::Var(a, ty)
                };
                fx.locals.push(slot);
            }
            if closure {
                let (env, args) = (fx.params[0].clone(), fx.params[1].clone());
                for (i, &l) in func.captures.iter().enumerate() {
                    let raw = fx.load(T::I64, &env, 8 * i as i64);
                    match fx.locals[l].clone() {
                        Slot::Cell(a, _) => fx.store_to(&raw, &a),
                        Slot::Var(..) => {
                            let v = fx.unkv(raw, &func.locals[l].ty);
                            fx.set_local(l, v);
                        }
                    }
                }
                for (i, &l) in func.params.iter().enumerate() {
                    let raw = fx.load(T::I64, &args, 8 * i as i64);
                    let v = fx.unkv(raw, &func.locals[l].ty);
                    fx.init_local(l, v);
                }
            } else {
                let params = fx.params.clone();
                for (&l, v) in func.params.iter().zip(params) {
                    fx.init_local(l, v);
                }
            }
            fx.poll();
            let void = is_void(&func.ret);
            let v = fx.block(&func.body, !void);
            let value_ty = block_ty(&func.body);
            if !void && !is_void(&value_ty) {
                let v = fx.convert(v, &value_ty, &func.ret);
                fx.ret_value(v);
            } else {
                fx.fail_return();
            }
        });
    }

    // ----- types -----

    fn is_prim(&self, id: AdtId) -> bool {
        self.p.is_prim(id)
    }

    fn is_adt(&self, ty: &Ty, id: AdtId) -> bool {
        matches!(ty, Ty::Adt(i, _) if *i == id)
    }

    fn is_string(&self, ty: &Ty) -> bool {
        self.is_adt(ty, self.prims.string)
    }

    fn is_bytes(&self, ty: &Ty) -> bool {
        self.is_adt(ty, self.prims.bytes)
    }

    /// A `String` or `Bytes`: both are a `kstr`, compared byte by byte.
    fn is_kstr(&self, ty: &Ty) -> bool {
        self.is_string(ty) || self.is_bytes(ty)
    }

    fn is_error(&self, ty: &Ty) -> bool {
        self.is_adt(ty, self.prims.error)
    }

    fn rep(&self, ty: &Ty) -> Rep {
        match ty {
            Ty::Adt(id, _) => {
                let id = *id;
                if id == self.prims.int || id == self.prims.duration {
                    Rep::Int
                } else if id == self.prims.float {
                    Rep::Float
                } else if id == self.prims.bool_ {
                    Rep::Bool
                } else {
                    Rep::Ptr
                }
            }
            Ty::Opt(_) | Ty::Fn(_) | Ty::Record(_) => Rep::Ptr,
            Ty::Param(_) | Ty::Var(_) | Ty::Any | Ty::Rest | Ty::Unknown | Ty::Style(_) => Rep::Kv,
            Ty::Unit | Ty::Never => Rep::Void,
        }
    }

    /// The LLVM type a value of `ty` is held in. Unit values are never
    /// used; they are a 64-bit zero.
    fn cty(&self, ty: &Ty) -> T {
        match self.rep(ty) {
            Rep::Float => T::F64,
            Rep::Bool => T::I1,
            _ => T::I64,
        }
    }

    fn type_map(&self, id: AdtId, args: &[Ty]) -> HashMap<String, Ty> {
        self.p.adts[id]
            .params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect()
    }

    /// The index of a struct's field.
    fn field_index(&self, id: AdtId, name: &str) -> usize {
        match &self.p.adts[id].kind {
            AdtKind::Struct(fields) => fields.iter().position(|f| f.name == name).unwrap_or(0),
            AdtKind::Enum(_) => 0,
        }
    }

    fn field_count(&self, id: AdtId) -> usize {
        match &self.p.adts[id].kind {
            AdtKind::Struct(fields) => fields.len(),
            AdtKind::Enum(_) => 0,
        }
    }

    fn unsupported(&mut self, span: Span, what: impl Into<String>) {
        self.diags.push(
            Diagnostic::error(
                codes::NOT_COMPILED_YET,
                format!("{} is not supported by the compiler yet", what.into()),
                span,
            )
            .with_help("`kiln check` accepts this; building it needs a later version of Kiln"),
        );
    }

    // ----- symbols -----

    /// A runtime function, declared the first time it is used.
    fn import(&mut self, name: &'static str) -> Sig {
        if let Some(sig) = self.imports.get(name) {
            return sig.clone();
        }
        let (params, ret) = runtime::signature(name)
            .unwrap_or_else(|| panic!("internal compiler error: no signature for `{name}`"));
        let t = |c: C| match c {
            C::I64 => T::I64,
            C::F64 => T::F64,
            C::Bool => T::I1,
            C::I32 => T::I32,
        };
        let params: Vec<T> = params.iter().map(|&c| t(c)).collect();
        let sig = Sig::new(&params, ret.map(t));
        self.imports.insert(name, sig.clone());
        sig
    }

    fn data_import(&mut self, name: &'static str) -> String {
        self.data_imports.insert(name);
        format!("@{name}")
    }

    fn intrinsic(&mut self, decl: &'static str) {
        self.intrinsics.insert(decl);
    }

    /// Read-only bytes in the module: the symbol and the length.
    fn bytes(&mut self, bytes: Vec<u8>) -> (String, usize) {
        if let Some(d) = self.strings.get(&bytes) {
            return d.clone();
        }
        let name = format!("@ks_{}", self.next());
        let n = bytes.len().max(1);
        let mut text = String::new();
        for &b in &bytes {
            match b {
                b'\\' => text.push_str("\\\\"),
                b'"' => text.push_str("\\22"),
                0x20..=0x7e => text.push(b as char),
                _ => {
                    let _ = write!(text, "\\{b:02X}");
                }
            }
        }
        if bytes.is_empty() {
            text.push_str("\\00");
        }
        let _ = writeln!(
            self.globals,
            "{name} = private unnamed_addr constant [{n} x i8] c\"{text}\", align 1"
        );
        let d = (name, bytes.len());
        self.strings.insert(bytes, d.clone());
        d
    }

    /// A NUL-terminated string in the module.
    fn c_string(&mut self, s: &str) -> String {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        self.bytes(bytes).0
    }

    /// A helper function, declared and generated the first time it is
    /// asked for. Returns its symbol and signature.
    fn helper(
        &mut self,
        key: String,
        signature: Sig,
        body: impl FnOnce(&mut Fx<'_, 'a>),
    ) -> (String, Sig) {
        if let Some(f) = self.helpers.get(&key) {
            return f.clone();
        }
        let name = format!("kh_{}", self.next());
        self.helpers.insert(key, (name.clone(), signature.clone()));
        self.define(&name, true, signature.clone(), body);
        (name, signature)
    }

    // ----- type descriptions -----

    /// Checks that the runtime can do `op` with values of `ty`, reporting
    /// E0401 where it cannot, as the C backend does.
    fn check(&mut self, op: Op, ty: &Ty, span: Span) {
        if !self.checked.insert((op, format!("{ty:?}"))) {
            return;
        }
        let p = self.prims;
        let rep = self.rep(ty);
        let simple = match op {
            Op::Show | Op::Eq => self.is_kstr(ty) || (matches!(ty, Ty::Adt(..)) && rep != Rep::Ptr),
            Op::Hash | Op::Cmp => {
                self.is_kstr(ty) || matches!(rep, Rep::Int | Rep::Bool | Rep::Float)
            }
        };
        if simple {
            return;
        }
        let parts: Option<Vec<Ty>> = match ty {
            Ty::Fn(_) if matches!(op, Op::Show | Op::Eq) => Some(Vec::new()),
            Ty::Opt(inner) => Some(vec![(**inner).clone()]),
            Ty::Adt(id, args) if *id == p.list => Some(vec![args[0].clone()]),
            Ty::Adt(id, args) if *id == p.map && op == Op::Show => {
                Some(vec![args[0].clone(), args[1].clone()])
            }
            Ty::Adt(id, args) if *id == p.map && op == Op::Eq => Some(vec![args[1].clone()]),
            Ty::Adt(id, args) if *id == p.set && op == Op::Show => Some(vec![args[0].clone()]),
            Ty::Adt(id, _) if *id == p.set && op == Op::Eq => Some(Vec::new()),
            Ty::Adt(id, args) if !self.is_prim(*id) => {
                let map = self.type_map(*id, args);
                Some(match &self.p.adts[*id].kind {
                    AdtKind::Struct(fields) => {
                        fields.iter().map(|f| substitute(&f.ty, &map)).collect()
                    }
                    AdtKind::Enum(vs) => vs
                        .iter()
                        .flat_map(|v| v.fields.iter().map(|(_, t)| substitute(t, &map)))
                        .collect(),
                })
            }
            Ty::Record(fields) => Some(fields.iter().map(|(_, t)| t.clone()).collect()),
            _ => None,
        };
        match parts {
            Some(parts) => {
                for part in parts {
                    self.check(op, &part, span);
                }
            }
            None => {
                let generic = matches!(ty, Ty::Param(_) | Ty::Var(_) | Ty::Any);
                let what = match op {
                    Op::Show if generic => "showing a value whose type is a generic parameter",
                    Op::Show => "showing a value of this type",
                    Op::Eq if generic => "comparing values whose type is a generic parameter",
                    Op::Eq => "comparing values of this type",
                    Op::Hash => "a map key or set item of this type",
                    Op::Cmp => "sorting by a key of this type",
                };
                self.unsupported(span, what);
            }
        }
    }

    /// The description of `ty` that the runtime's `k_desc_*` functions
    /// read: a `kdesc` (see `runtime/native.c`) with its parts and labels
    /// after it. Returns its symbol.
    fn desc(&mut self, ty: &Ty) -> String {
        let key = format!("{ty:?}");
        if let Some(d) = self.descs.get(&key) {
            return d.clone();
        }
        let d = format!("@kd_{}", self.next());
        // Recursive types refer back to this description.
        self.descs.insert(key, d.clone());
        let p = self.prims;
        let leaf = |kind: i64| (kind, None, Vec::new(), Vec::new());
        let (kind, name, parts, labels): (i64, Option<String>, Vec<String>, Vec<Option<String>>) =
            match ty {
                _ if self.is_string(ty) => leaf(desc::STR),
                _ if self.is_bytes(ty) => leaf(desc::BYTES),
                Ty::Adt(id, _) if *id == p.int => leaf(desc::INT),
                Ty::Adt(id, _) if *id == p.float => leaf(desc::FLOAT),
                Ty::Adt(id, _) if *id == p.bool_ => leaf(desc::BOOL),
                Ty::Adt(id, _) if *id == p.duration => leaf(desc::DURATION),
                Ty::Fn(_) => leaf(desc::FN),
                Ty::Opt(inner) => (desc::OPT, None, vec![self.desc(inner)], vec![None]),
                Ty::Adt(id, args) if *id == p.list => {
                    (desc::LIST, None, vec![self.desc(&args[0])], vec![None])
                }
                Ty::Adt(id, args) if *id == p.set => {
                    (desc::SET, None, vec![self.desc(&args[0])], vec![None])
                }
                Ty::Adt(id, args) if *id == p.map => {
                    let parts = vec![self.desc(&args[0]), self.desc(&args[1])];
                    (desc::MAP, None, parts, vec![None, None])
                }
                Ty::Adt(id, args) if !self.is_prim(*id) => {
                    let adt = self.p.adts[*id].clone();
                    let map = self.type_map(*id, args);
                    match &adt.kind {
                        AdtKind::Struct(fields) => {
                            let parts = fields
                                .iter()
                                .map(|f| self.desc(&substitute(&f.ty, &map)))
                                .collect();
                            let labels = fields.iter().map(|f| Some(f.name.clone())).collect();
                            (desc::STRUCT, Some(adt.name.clone()), parts, labels)
                        }
                        AdtKind::Enum(variants) => {
                            let parts = variants
                                .iter()
                                .map(|v| {
                                    let fields: Vec<(Option<String>, Ty)> = v
                                        .fields
                                        .iter()
                                        .map(|(n, t)| (n.clone(), substitute(t, &map)))
                                        .collect();
                                    self.variant_desc(&v.name, &fields)
                                })
                                .collect();
                            let labels = variants.iter().map(|_| None).collect();
                            (desc::ENUM, Some(adt.name.clone()), parts, labels)
                        }
                    }
                }
                Ty::Record(fields) => {
                    let parts = fields.iter().map(|(_, t)| self.desc(t)).collect();
                    let labels = fields.iter().map(|(n, _)| Some(n.clone())).collect();
                    (desc::RECORD, None, parts, labels)
                }
                _ => leaf(desc::UNKNOWN),
            };
        self.define_desc(&d, kind, name.as_deref(), &parts, &labels);
        d
    }

    fn variant_desc(&mut self, name: &str, fields: &[(Option<String>, Ty)]) -> String {
        let d = format!("@kd_{}", self.next());
        let parts: Vec<String> = fields.iter().map(|(_, t)| self.desc(t)).collect();
        let labels: Vec<Option<String>> = fields.iter().map(|(n, _)| n.clone()).collect();
        self.define_desc(&d, desc::VARIANT, Some(name), &parts, &labels);
        d
    }

    /// A `kdesc` as a global: kind, part count, name, the addresses of its
    /// parts and labels (which follow it), then the parts and the labels.
    fn define_desc(
        &mut self,
        d: &str,
        kind: i64,
        name: Option<&str>,
        parts: &[String],
        labels: &[Option<String>],
    ) {
        let n = parts.len();
        let name = match name {
            Some(name) => self.c_string(name),
            None => "null".to_string(),
        };
        let part_list: Vec<String> = parts.iter().map(|p| format!("ptr {p}")).collect();
        let label_list: Vec<String> = labels
            .iter()
            .map(|l| match l {
                Some(l) => format!("ptr {}", self.c_string(l)),
                None => "ptr null".to_string(),
            })
            .collect();
        let ty = format!("{{ i64, i64, ptr, ptr, ptr, [{n} x ptr], [{n} x ptr] }}");
        let _ = writeln!(
            self.globals,
            "{d} = private constant {ty} {{ i64 {kind}, i64 {n}, ptr {name}, \
             ptr getelementptr (i8, ptr {d}, i64 40), \
             ptr getelementptr (i8, ptr {d}, i64 {}), \
             [{n} x ptr] [{}], [{n} x ptr] [{}] }}, align 8",
            40 + 8 * n,
            part_list.join(", "),
            label_list.join(", ")
        );
    }

    /// A `kstr *(kv)` function that shows values of `ty`, quoting strings
    /// with `quote`, as they are inside lists and structs.
    fn show_fn(&mut self, ty: &Ty, quote: bool, span: Span) -> (String, Sig) {
        self.check(Op::Show, ty, span);
        let d = self.desc(ty);
        let key = format!("show:{quote}:{ty:?}");
        self.helper(key, Sig::new(&[T::I64], Some(T::I64)), |fx| {
            let v = fx.params[0].clone();
            let addr = fx.data_addr(&d);
            let q = V::bool(quote);
            let r = fx.call_rt("k_desc_show", &[addr, v, q]);
            fx.ret_raw(Some(r));
        })
    }

    /// A `bool (kv, kv)` function that compares values of `ty`.
    fn eq_fn(&mut self, ty: &Ty, span: Span) -> (String, Sig) {
        self.check(Op::Eq, ty, span);
        let d = self.desc(ty);
        let key = format!("eq:{ty:?}");
        self.helper(key, Sig::new(&[T::I64, T::I64], Some(T::I1)), |fx| {
            let (a, b) = (fx.params[0].clone(), fx.params[1].clone());
            let addr = fx.data_addr(&d);
            let r = fx.call_rt("k_desc_eq", &[addr, a, b]);
            fx.ret_raw(Some(r));
        })
    }

    /// The hash function for map keys and set items of `ty`.
    fn hash_fn(&mut self, ty: &Ty, span: Span) -> (String, Sig) {
        let rt = |g: &mut Self, name: &'static str| (name.to_string(), g.import(name));
        if self.is_kstr(ty) {
            return rt(self, "k_hash_str");
        }
        match self.rep(ty) {
            Rep::Int | Rep::Bool => return rt(self, "k_hash_int"),
            Rep::Float => return rt(self, "k_hash_float"),
            _ => {}
        }
        self.check(Op::Hash, ty, span);
        let d = self.desc(ty);
        let key = format!("hash:{ty:?}");
        self.helper(key, Sig::new(&[T::I64], Some(T::I64)), |fx| {
            let v = fx.params[0].clone();
            let addr = fx.data_addr(&d);
            let r = fx.call_rt("k_desc_hash", &[addr, v]);
            fx.ret_raw(Some(r));
        })
    }

    /// An `int (kv, kv)` function that orders values of `ty`.
    fn cmp_fn(&mut self, ty: &Ty, span: Span) -> (String, Sig) {
        self.check(Op::Cmp, ty, span);
        let d = self.desc(ty);
        let key = format!("cmp:{ty:?}");
        self.helper(key, Sig::new(&[T::I64, T::I64], Some(T::I32)), |fx| {
            let (a, b) = (fx.params[0].clone(), fx.params[1].clone());
            let addr = fx.data_addr(&d);
            let r = fx.call_rt("k_desc_cmp", &[addr, a, b]);
            fx.ret_raw(Some(r));
        })
    }

    /// The `kstr *(kv)` function that shows a standard `Error`: its
    /// message.
    fn error_message_fn(&mut self) -> (String, Sig) {
        let offset = 8 * self.field_index(self.prims.error, "message") as i64;
        self.helper(
            "error-message".into(),
            Sig::new(&[T::I64], Some(T::I64)),
            |fx| {
                let v = fx.params[0].clone();
                let m = fx.load(T::I64, &v, offset);
                fx.ret_raw(Some(m));
            },
        )
    }

    /// A top-level function as a closure function: `kv (void *, kv *)`.
    fn fn_wrapper(&mut self, id: FuncId) -> (String, Sig) {
        let p = self.p;
        let func = &p.funcs[id];
        let target = self.funcs[id].clone();
        self.helper(
            format!("fn:{id}"),
            Sig::new(&[T::I64, T::I64], Some(T::I64)),
            |fx| {
                let args = fx.params[1].clone();
                let mut vals = Vec::new();
                for (i, &l) in func.params.iter().enumerate() {
                    let raw = fx.load(T::I64, &args, 8 * i as i64);
                    let v = fx.unkv(raw, &func.locals[l].ty);
                    let want = fx.g.cty(&func.locals[l].ty);
                    vals.push(fx.fit(v, want));
                }
                let r = fx.call(&target.0, &target.1, &vals);
                let out = match r {
                    Some(r) if !is_void(&func.ret) => fx.kv_of(r, &func.ret),
                    _ => V::int(0),
                };
                fx.ret_raw(Some(out));
            },
        )
    }
}

/// Where a local lives: an `alloca` of its type, or for a captured `var`,
/// an `alloca` holding a pointer to its cell, with the cell's content
/// type.
#[derive(Clone)]
enum Slot {
    Var(V, T),
    Cell(V, T),
}

/// The function's shadow-stack frame (see `docs/plans/precise-gc.md`): the
/// `alloca` `%s0` holding `prev`, `n` and `n` root slots, linked from
/// `k_gc_top` while the function runs. Rooted locals live in its slots,
/// and every other pointer or `kv` value held across a call is stored in
/// one when it is defined; the collector reads the slots, and never moves
/// an object, so the SSA copy stays good.
struct Frame {
    /// The root slots in use at this point, and the most ever in use.
    top: usize,
    max: usize,
}

/// The bytes before a frame's first root slot: `prev` and `n`.
const FRAME_SLOTS: i64 = 16;

/// Where an expression's value is rooted; see `Fx::rooting`.
enum Rooting {
    New,
    Inner,
    None,
}

/// The state of the function being generated.
struct Fx<'g, 'a> {
    g: &'g mut Gen<'a>,
    /// The instructions after the entry block's `alloca`s.
    body: String,
    allocas: String,
    params: Vec<V>,
    locals: Vec<Slot>,
    tys: Vec<Ty>,
    ret: Ty,
    closure: bool,
    /// For each loop around the current point: where `continue` and
    /// `break` go.
    loops: Vec<(String, String)>,
    counter: usize,
    /// Whether the current block has ended.
    terminated: bool,
    /// The shadow-stack frame of a user function; helpers have none.
    frame: Option<Frame>,
    /// The frame's `alloca` and the code that links it, written by
    /// `finish_frame` once the slot count is known; they go before and
    /// after the other `alloca`s.
    frame_alloca: String,
    prologue: String,
}

impl Fx<'_, '_> {
    // ----- instructions -----

    fn next(&mut self) -> usize {
        self.counter += 1;
        self.counter
    }

    /// Writes an instruction into the current block.
    fn emit(&mut self, inst: &str) {
        let _ = writeln!(self.body, "  {inst}");
        self.terminated = matches!(
            inst.split_whitespace().next(),
            Some("br" | "ret" | "unreachable" | "switch")
        );
    }

    /// Writes an instruction with a result and returns it.
    fn ins(&mut self, ty: T, inst: &str) -> V {
        let r = format!("%t{}", self.next());
        let _ = writeln!(self.body, "  {r} = {inst}");
        V::new(r, ty)
    }

    /// A new block, returned by its label; nothing jumps to it yet.
    fn create_block(&mut self) -> String {
        format!("b{}", self.next())
    }

    /// Continues in `block`. The current one must have ended; if it has
    /// not, it falls through.
    fn switch_to_block(&mut self, block: &str) {
        if !self.terminated {
            self.emit(&format!("br label %{block}"));
        }
        let _ = writeln!(self.body, "{block}:");
        self.terminated = false;
    }

    /// A stack slot of type `ty` in the entry block.
    fn alloca(&mut self, ty: T) -> V {
        let r = format!("%s{}", self.next());
        let _ = writeln!(self.allocas, "  {r} = alloca {}, align 8", ty.name());
        V::new(r, T::I64)
    }

    /// A stack array of `n` slots in the entry block.
    fn alloca_array(&mut self, n: usize) -> V {
        let r = format!("%s{}", self.next());
        let _ = writeln!(self.allocas, "  {r} = alloca [{n} x i64], align 8");
        V::new(r, T::I64)
    }

    fn zero(&mut self, ty: &Ty) -> V {
        let t = self.g.cty(ty);
        V::new(t.zero(), t)
    }

    /// The address `base + offset` as a pointer register. `base` is an
    /// `i64` value, or an `alloca` or global (already a pointer).
    fn ptr(&mut self, base: &V, offset: i64) -> String {
        let p = if base.text.starts_with("%s") || base.text.starts_with('@') {
            base.text.clone()
        } else {
            self.ins(T::I64, &format!("inttoptr i64 {} to ptr", base.text))
                .text
        };
        if offset == 0 {
            return p;
        }
        self.ins(
            T::I64,
            &format!("getelementptr inbounds i8, ptr {p}, i64 {offset}"),
        )
        .text
    }

    fn load(&mut self, ty: T, addr: &V, offset: i64) -> V {
        let p = self.ptr(addr, offset);
        if ty == T::I1 {
            let b = self.ins(T::I64, &format!("load i8, ptr {p}, align 1"));
            return self.ins(T::I1, &format!("icmp ne i8 {}, 0", b.text));
        }
        let align = if ty == T::I32 { 4 } else { 8 };
        self.ins(ty, &format!("load {}, ptr {p}, align {align}", ty.name()))
    }

    fn store(&mut self, v: &V, addr: &V, offset: i64) {
        let p = self.ptr(addr, offset);
        self.store_at(v, &p);
    }

    fn store_at(&mut self, v: &V, p: &str) {
        let align = if v.ty == T::I32 { 4 } else { 8 };
        self.emit(&format!("store {}, ptr {p}, align {align}", v.op()));
    }

    /// Stores into an `alloca`.
    fn store_to(&mut self, v: &V, slot: &V) {
        self.store_at(v, &slot.text.clone());
    }

    /// Loads from an `alloca` of type `ty`.
    fn load_from(&mut self, ty: T, slot: &V) -> V {
        let p = slot.text.clone();
        self.ins(ty, &format!("load {}, ptr {p}, align 8", ty.name()))
    }

    fn data_addr(&mut self, symbol: &str) -> V {
        V::new(format!("ptrtoint (ptr {symbol} to i64)"), T::I64)
    }

    fn func_addr(&mut self, f: &(String, Sig)) -> V {
        V::new(format!("ptrtoint (ptr @{} to i64)", f.0), T::I64)
    }

    /// Calls a function in the module, fitting the arguments to its
    /// parameters. Returns its result, if it has one.
    fn call(&mut self, name: &str, sig: &Sig, args: &[V]) -> Option<V> {
        let args: Vec<String> = args
            .iter()
            .zip(&sig.params)
            .map(|(a, &t)| {
                let v = self.fit(a.clone(), t);
                format!("{} {}", t.abi(), v.text)
            })
            .collect();
        let text = format!("call {} @{name}({})", sig.ret_name(), args.join(", "));
        match sig.ret {
            Some(t) => Some(self.ins(t, &text)),
            None => {
                self.emit(&text);
                None
            }
        }
    }

    /// Calls a function through its address.
    fn call_indirect(&mut self, sig: &Sig, func: &V, args: &[V]) -> Option<V> {
        let p = self.ins(T::I64, &format!("inttoptr i64 {} to ptr", func.text));
        let args: Vec<String> = args
            .iter()
            .zip(&sig.params)
            .map(|(a, &t)| {
                let v = self.fit(a.clone(), t);
                format!("{} {}", t.abi(), v.text)
            })
            .collect();
        let text = format!("call {} {}({})", sig.fn_type(), p.text, args.join(", "));
        match sig.ret {
            Some(t) => Some(self.ins(t, &text)),
            None => {
                self.emit(&text);
                None
            }
        }
    }

    /// Calls a runtime function. Returns its result, or a zero for `void`.
    fn call_rt(&mut self, name: &'static str, args: &[V]) -> V {
        let sig = self.g.import(name);
        match self.call(name, &sig, args) {
            Some(v) => v,
            None => V::int(0),
        }
    }

    /// A C string constant's address.
    fn c_str(&mut self, s: &str) -> V {
        let d = self.g.c_string(s);
        self.data_addr(&d)
    }

    /// A new `kstr *` with the text `s`.
    fn str_const(&mut self, s: &str) -> V {
        let (d, n) = self.g.bytes(s.as_bytes().to_vec());
        let addr = self.data_addr(&d);
        self.call_rt("k_str_n", &[addr, V::int(n as i64)])
    }

    /// Stores values in a new stack array of slots and returns its address.
    fn stack_array(&mut self, vals: &[V]) -> V {
        let slot = self.alloca_array(vals.len().max(1));
        for (i, v) in vals.iter().enumerate() {
            let v = self.fit(v.clone(), T::I64);
            self.store(&v, &slot, 8 * i as i64);
        }
        self.ins(T::I64, &format!("ptrtoint ptr {} to i64", slot.text))
    }

    /// Allocates `n` slots on the heap, telling the collector which hold
    /// pointers: bit `i` of `bits` is set when slot `i` may.
    fn alloc_bits(&mut self, n: usize, bits: u64) -> V {
        let size = V::int(8 * n.max(1) as i64);
        self.call_rt("k_alloc_bits", &[size, V::int(bits as i64)])
    }

    /// The bits of `alloc_bits` for slots holding values of these types.
    fn pointer_bits(&self, tys: &[Ty]) -> u64 {
        let mut bits = 0;
        for (i, ty) in tys.iter().enumerate() {
            if matches!(self.g.rep(ty), Rep::Ptr | Rep::Kv) && i < 64 {
                bits |= 1 << i;
            }
        }
        bits
    }

    // ----- the shadow stack -----

    /// Starts the function's frame. Its `alloca` and the code linking it
    /// from `k_gc_top` are written by `finish_frame`, once the number of
    /// slots is known.
    fn enter_frame(&mut self) {
        self.g.data_import("k_gc_top");
        self.frame = Some(Frame { top: 0, max: 0 });
    }

    /// Writes the frame's `alloca`, sized for the slots the function used,
    /// and the code that fills `prev` and `n`, zeroes the slots (so that
    /// the collector never reads a stale word) and links the frame.
    fn finish_frame(&mut self) {
        let Some(frame) = self.frame.take() else {
            return;
        };
        let n = frame.max;
        let _ = writeln!(
            self.frame_alloca,
            "  %s0 = alloca [{} x i64], align 8",
            n + 2
        );
        let mut p = String::new();
        let _ = writeln!(p, "  %gcprev = load i64, ptr @k_gc_top, align 8");
        let _ = writeln!(p, "  store i64 %gcprev, ptr %s0, align 8");
        let _ = writeln!(p, "  %gcn = getelementptr inbounds i8, ptr %s0, i64 8");
        let _ = writeln!(p, "  store i64 {n}, ptr %gcn, align 8");
        for i in 0..n {
            let _ = writeln!(
                p,
                "  %gcz{i} = getelementptr inbounds i8, ptr %s0, i64 {}",
                FRAME_SLOTS + 8 * i as i64
            );
            let _ = writeln!(p, "  store i64 0, ptr %gcz{i}, align 8");
        }
        let _ = writeln!(p, "  %gcself = ptrtoint ptr %s0 to i64");
        let _ = writeln!(p, "  store i64 %gcself, ptr @k_gc_top, align 8");
        self.prologue = p;
    }

    /// Unlinks the frame before a return.
    fn leave_frame(&mut self) {
        if self.frame.is_some() {
            self.emit("store i64 %gcprev, ptr @k_gc_top, align 8");
        }
    }

    /// A root slot free from here until the enclosing statement or
    /// expression is done.
    fn temp_slot(&mut self) -> usize {
        let frame = self.frame.as_mut().expect("a frame");
        let i = frame.top;
        frame.top += 1;
        frame.max = frame.max.max(frame.top);
        i
    }

    /// The address of root slot `i`, as a pointer register.
    fn slot_ptr(&mut self, i: usize) -> String {
        self.ins(
            T::I64,
            &format!(
                "getelementptr inbounds i8, ptr %s0, i64 {}",
                FRAME_SLOTS + 8 * i as i64
            ),
        )
        .text
    }

    /// A fresh root slot for a local, as the `alloca`-like pointer the
    /// local is loaded from and stored to (computed in the entry block).
    fn frame_slot(&mut self) -> V {
        let i = self.temp_slot();
        let r = format!("%s{}", self.next());
        let _ = writeln!(
            self.allocas,
            "  {r} = getelementptr inbounds i8, ptr %s0, i64 {}",
            FRAME_SLOTS + 8 * i as i64
        );
        V::new(r, T::I64)
    }

    /// Stores a pointer or `kv` value in a root slot.
    fn root_in(&mut self, i: usize, v: V) {
        let v = self.fit(v, T::I64);
        let p = self.slot_ptr(i);
        self.store_at(&v, &p);
    }

    /// A safepoint: collects if the heap has grown enough. Every live
    /// pointer is in a root slot here.
    fn poll(&mut self) {
        if self.frame.is_none() {
            return;
        }
        self.g.data_import("k_gc_pending");
        let pending = self.ins(T::I64, "load i8, ptr @k_gc_pending, align 1");
        let c = self.ins(T::I1, &format!("icmp ne i8 {}, 0", pending.text));
        let (collect, cont) = (self.create_block(), self.create_block());
        self.brif(c, &collect, &cont);
        self.switch_to_block(&collect);
        self.call_rt("k_gc_collect", &[]);
        self.jump(&cont);
        self.switch_to_block(&cont);
    }

    /// Continues in a new block that nothing jumps to, after a `return`,
    /// `break` or `continue`.
    fn dead(&mut self) {
        let block = self.create_block();
        self.switch_to_block(&block);
    }

    fn jump(&mut self, to: &str) {
        self.emit(&format!("br label %{to}"));
    }

    fn brif(&mut self, c: V, then: &str, otherwise: &str) {
        let c = self.fit(c, T::I1);
        self.emit(&format!(
            "br i1 {}, label %{then}, label %{otherwise}",
            c.text
        ));
    }

    /// Moves a value to the LLVM type `want`. Values of the right kind
    /// pass unchanged; this smooths over values whose type is `Never` or
    /// unit, which have no real value.
    fn fit(&mut self, v: V, want: T) -> V {
        if v.ty == want {
            return v;
        }
        match (v.ty, want) {
            (T::I1, T::I64) => self.ins(T::I64, &format!("zext i1 {} to i64", v.text)),
            (T::I1, T::I32) => self.ins(T::I32, &format!("zext i1 {} to i32", v.text)),
            (T::I64, T::I1) => self.ins(T::I1, &format!("icmp ne i64 {}, 0", v.text)),
            (T::I32, T::I1) => self.ins(T::I1, &format!("icmp ne i32 {}, 0", v.text)),
            (T::F64, T::I64) => self.ins(T::I64, &format!("bitcast double {} to i64", v.text)),
            (T::I64, T::F64) => self.ins(T::F64, &format!("bitcast i64 {} to double", v.text)),
            (T::I32, T::I64) => self.ins(T::I64, &format!("sext i32 {} to i64", v.text)),
            (T::I64, T::I32) => self.ins(T::I32, &format!("trunc i64 {} to i32", v.text)),
            _ => V::new(want.zero(), want),
        }
    }

    /// Returns from the function with `v`, or nothing.
    fn ret_raw(&mut self, v: Option<V>) {
        match v {
            Some(v) => self.emit(&format!("ret {}", v.op())),
            None => self.emit("ret void"),
        }
    }

    // ----- values and slots -----

    /// A value of type `ty` stored in a `kv` slot.
    fn kv_of(&mut self, v: V, ty: &Ty) -> V {
        match self.g.rep(ty) {
            Rep::Void => V::int(0),
            _ => self.fit(v, T::I64),
        }
    }

    /// A `kv` slot read back as a value of type `ty`.
    fn unkv(&mut self, v: V, ty: &Ty) -> V {
        match self.g.rep(ty) {
            Rep::Void => V::int(0),
            Rep::Float => self.fit(v, T::F64),
            Rep::Bool => self.fit(v, T::I1),
            _ => v,
        }
    }

    /// Converts a value of type `from` to the representation of `to`: into
    /// or out of a `kv` slot, or into an optional.
    fn convert(&mut self, v: V, from: &Ty, to: &Ty) -> V {
        if matches!(to, Ty::Never | Ty::Unit) {
            return v;
        }
        if matches!(from, Ty::Never) {
            return self.zero(to);
        }
        if matches!(from, Ty::Unknown) || matches!(to, Ty::Unknown) {
            let want = self.g.cty(to);
            return self.fit(v, want);
        }
        let (fr, tr) = (self.g.rep(from), self.g.rep(to));
        // A `T` stored in a `T?` is boxed, even when `T` lives in a slot.
        if matches!(to, Ty::Opt(inner) if **inner == *from) {
            let k = self.kv_of(v, from);
            return self.call_rt("k_box", &[k]);
        }
        if tr == Rep::Kv {
            return self.kv_of(v, from);
        }
        if fr == Rep::Kv {
            return self.unkv(v, to);
        }
        if matches!(to, Ty::Opt(_)) && !matches!(from, Ty::Opt(_)) {
            let k = self.kv_of(v, from);
            return self.call_rt("k_box", &[k]);
        }
        let want = self.g.cty(to);
        self.fit(v, want)
    }

    fn local(&mut self, l: LocalId) -> V {
        match self.locals[l].clone() {
            Slot::Var(a, ty) => self.load_from(ty, &a),
            Slot::Cell(a, ty) => {
                let cell = self.load_from(T::I64, &a);
                self.load(ty, &cell, 0)
            }
        }
    }

    fn set_local(&mut self, l: LocalId, v: V) {
        let want = self.g.cty(&self.tys[l].clone());
        let v = self.fit(v, want);
        match self.locals[l].clone() {
            Slot::Var(a, _) => self.store_to(&v, &a),
            Slot::Cell(a, _) => {
                let cell = self.load_from(T::I64, &a);
                self.store(&v, &cell, 0);
            }
        }
    }

    /// Gives a local its first value, allocating its cell if it has one.
    fn init_local(&mut self, l: LocalId, v: V) {
        if let Slot::Cell(a, _) = self.locals[l].clone() {
            let bits = self.pointer_bits(std::slice::from_ref(&self.tys[l]));
            let cell = self.alloc_bits(1, bits);
            self.store_to(&cell, &a);
        }
        self.set_local(l, v);
    }

    fn global_addr(&mut self, g: GlobalId) -> (V, i64) {
        (V::new("@kiln_globals", T::I64), 8 * g as i64)
    }

    // ----- returns and errors -----

    fn ret_value(&mut self, v: V) {
        self.leave_frame();
        if self.closure {
            let ret = self.ret.clone();
            let k = self.kv_of(v, &ret);
            self.ret_raw(Some(k));
        } else if is_void(&self.ret) {
            self.ret_raw(None);
        } else {
            let want = self.g.cty(&self.ret.clone());
            let v = self.fit(v, want);
            self.ret_raw(Some(v));
        }
    }

    /// Leaves the current function after an error, or at the end of a
    /// function whose value is unused.
    fn fail_return(&mut self) {
        self.leave_frame();
        if self.closure {
            self.ret_raw(Some(V::int(0)));
        } else if is_void(&self.ret) {
            self.ret_raw(None);
        } else {
            let z = self.zero(&self.ret.clone());
            self.ret_raw(Some(z));
        }
    }

    fn failed(&mut self) -> V {
        let d = self.g.data_import("k_failed");
        let addr = V::new(d, T::I64);
        self.load(T::I1, &addr, 0)
    }

    /// After a call that failed, passes its error on to the caller,
    /// wrapping it in the standard `Error` first with `wrap`.
    fn propagate(&mut self, wrap: bool) {
        let (fail, cont) = (self.create_block(), self.create_block());
        let f = self.failed();
        self.brif(f, &fail, &cont);
        self.switch_to_block(&fail);
        if wrap {
            let show_addr = V::new(self.g.data_import("k_error_show"), T::I64);
            let err_addr = V::new(self.g.data_import("k_error"), T::I64);
            let show = self.load(T::I64, &show_addr, 0);
            let err = self.load(T::I64, &err_addr, 0);
            let sig = Sig::new(&[T::I64], Some(T::I64));
            let message = self.call_indirect(&sig, &show, &[err]).unwrap();
            let w = self.new_error(message);
            self.store(&w, &err_addr, 0);
            let f = self.g.error_message_fn();
            let fa = self.func_addr(&f);
            self.store(&fa, &show_addr, 0);
        }
        self.fail_return();
        self.switch_to_block(&cont);
    }

    /// A new standard `Error` with the message.
    fn new_error(&mut self, message: V) -> V {
        let error = self.g.prims.error;
        let n = self.g.field_count(error);
        let w = self.alloc_bits(n, 1 << self.g.field_index(error, "message"));
        let offset = 8 * self.g.field_index(error, "message") as i64;
        self.store(&message, &w, offset);
        w
    }

    fn throw(&mut self, value: &Expr, wrap: bool) {
        let mut et = value.ty.clone();
        let mut v = self.expr(value);
        if wrap {
            let shown = self.display(v, &et, value.span);
            v = self.new_error(shown);
            et = Ty::Adt(self.g.prims.error, Vec::new());
        }
        let show = if self.g.is_error(&et) {
            self.g.error_message_fn()
        } else {
            self.g.show_fn(&et, false, value.span)
        };
        let k = self.kv_of(v, &et);
        let fa = self.func_addr(&show);
        self.call_rt("k_fail", &[k, fa]);
        self.fail_return();
        self.dead();
    }

    fn catch(&mut self, e: &Expr, want: bool) -> V {
        let ExprKind::Catch {
            expr,
            err,
            binding,
            handler,
        } = &e.kind
        else {
            return V::int(0);
        };
        let rt = e.ty.clone();
        let v = self.expr(expr);
        let keep = want && !is_void(&rt) && !is_void(&expr.ty);
        let merge = self.create_block();
        let r = keep.then(|| {
            let t = self.g.cty(&rt);
            (self.alloca(t), t)
        });
        if let Some((slot, _)) = &r {
            let v = self.convert(v, &expr.ty, &rt);
            self.store_to(&v, slot);
        }
        let handle = self.create_block();
        let f = self.failed();
        self.brif(f, &handle, &merge);
        self.switch_to_block(&handle);
        let taken = self.call_rt("k_take_error", &[]);
        if let Some(b) = binding {
            let v = self.unkv(taken, err);
            self.set_local(*b, v);
        }
        let hv = self.block(handler, keep);
        if let Some((slot, _)) = &r {
            let ht = block_ty(handler);
            let v = if is_void(&ht) {
                self.zero(&rt)
            } else {
                self.convert(hv, &ht, &rt)
            };
            self.store_to(&v, slot);
        }
        self.jump(&merge);
        self.switch_to_block(&merge);
        match r {
            Some((slot, t)) => self.load_from(t, &slot),
            None => V::int(0),
        }
    }

    // ----- statements -----

    /// Generates a block. Returns its value when `want` is set; otherwise
    /// the value is generated as a statement.
    fn block(&mut self, b: &Block, want: bool) -> V {
        for stmt in &b.stmts {
            self.stmt(stmt);
        }
        match &b.value {
            Some(v) if want => self.expr(v),
            Some(v) => {
                self.discard(v);
                V::int(0)
            }
            None => V::int(0),
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        let base = self.frame.as_ref().map(|f| f.top);
        self.stmt_inner(stmt);
        if let (Some(base), Some(frame)) = (base, self.frame.as_mut()) {
            frame.top = base;
        }
    }

    fn stmt_inner(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(l, value) => {
                let v = self.expr(value);
                self.init_local(*l, v);
            }
            Stmt::SetGlobal(g, value) => {
                let v = self.expr(value);
                let want = self.g.cty(&self.g.p.globals[*g].ty);
                let v = self.fit(v, want);
                let (addr, offset) = self.global_addr(*g);
                if v.ty == T::I1 {
                    let v = self.fit(v, T::I64);
                    self.store(&v, &addr, offset);
                } else {
                    self.store(&v, &addr, offset);
                }
            }
            Stmt::SetStore(id, value) => {
                let v = self.expr(value);
                let slot = self.call_rt("k_store_slot", &[V::int(*id as i64)]);
                let v = self.fit(v, T::I64);
                self.store(&v, &slot, 0);
            }
            Stmt::Assign { place, op, value } => self.assign(place, *op, value),
            Stmt::Return(value) => {
                let v = match value {
                    Some(v) => self.expr(v),
                    None => V::int(0),
                };
                if value.is_some() {
                    self.ret_value(v);
                } else {
                    self.fail_return();
                }
                self.dead();
            }
            Stmt::Break | Stmt::Continue => {
                if let Some((cont, brk)) = self.loops.last().cloned() {
                    let to = if matches!(stmt, Stmt::Break) {
                        brk
                    } else {
                        cont
                    };
                    self.jump(&to);
                    self.dead();
                }
            }
            Stmt::While { cond, body } => {
                let (header, body_b, exit) = (
                    self.create_block(),
                    self.create_block(),
                    self.create_block(),
                );
                self.jump(&header);
                self.switch_to_block(&header);
                let c = self.expr(cond);
                self.brif(c, &body_b, &exit);
                self.switch_to_block(&body_b);
                self.poll();
                self.loops.push((header.clone(), exit.clone()));
                self.block(body, false);
                self.loops.pop();
                self.jump(&header);
                self.switch_to_block(&exit);
            }
            Stmt::ForRange {
                var,
                lo,
                hi,
                filter,
                body,
            } => {
                let lo = self.expr(lo);
                let hi = self.expr(hi);
                let i = self.alloca(T::I64);
                let lo = self.fit(lo, T::I64);
                let hi = self.fit(hi, T::I64);
                self.store_to(&lo, &i);
                let int = Ty::Adt(self.g.prims.int, Vec::new());
                self.counted_loop(
                    &i,
                    |fx| {
                        let n = fx.load_from(T::I64, &i);
                        fx.ins(T::I1, &format!("icmp slt i64 {}, {}", n.text, hi.text))
                    },
                    |fx| {
                        let n = fx.load_from(T::I64, &i);
                        let vt = fx.tys[*var].clone();
                        let item = fx.convert(n, &int, &vt);
                        fx.set_local(*var, item);
                    },
                    filter.as_ref(),
                    body,
                );
            }
            Stmt::ForEach {
                var,
                iter,
                filter,
                body,
            } => {
                let (elem, is_set) = match &iter.ty {
                    Ty::Adt(id, args) if !args.is_empty() => {
                        (args[0].clone(), *id == self.g.prims.set)
                    }
                    _ => (Ty::Unknown, false),
                };
                let mut l = self.expr(iter);
                // A set is looped over through a snapshot of its items.
                if is_set {
                    l = self.call_rt("k_map_keys", &[l]);
                    if self.frame.is_some() {
                        let i = self.temp_slot();
                        self.root_in(i, l.clone());
                    }
                }
                let i = self.alloca(T::I64);
                self.store_to(&V::int(0), &i);
                self.counted_loop(
                    &i,
                    |fx| {
                        let len = fx.load(T::I64, &l, 0);
                        let n = fx.load_from(T::I64, &i);
                        fx.ins(T::I1, &format!("icmp slt i64 {}, {}", n.text, len.text))
                    },
                    |fx| {
                        let items = fx.load(T::I64, &l, 16);
                        let n = fx.load_from(T::I64, &i);
                        let off = fx.ins(T::I64, &format!("shl i64 {}, 3", n.text));
                        let addr = fx.ins(T::I64, &format!("add i64 {}, {}", items.text, off.text));
                        let raw = fx.load(T::I64, &addr, 0);
                        let item = fx.unkv(raw, &elem);
                        let vt = fx.tys[*var].clone();
                        let item = fx.convert(item, &elem, &vt);
                        fx.set_local(*var, item);
                    },
                    filter.as_ref(),
                    body,
                );
            }
            Stmt::Expr(e) => self.discard(e),
        }
    }

    /// A loop over the counter in the slot `i`, from its value now while
    /// `cond` holds, counting up by one. `start` sets the loop variable
    /// each time round.
    fn counted_loop(
        &mut self,
        i: &V,
        cond: impl FnOnce(&mut Self) -> V,
        start: impl FnOnce(&mut Self),
        filter: Option<&Expr>,
        body: &Block,
    ) {
        let (header, body_b, latch, exit) = (
            self.create_block(),
            self.create_block(),
            self.create_block(),
            self.create_block(),
        );
        self.jump(&header);
        self.switch_to_block(&header);
        let c = cond(self);
        self.brif(c, &body_b, &exit);
        self.switch_to_block(&body_b);
        self.poll();
        start(self);
        if let Some(f) = filter {
            let keep = self.expr(f);
            let run = self.create_block();
            self.brif(keep, &run, &latch);
            self.switch_to_block(&run);
        }
        self.loops.push((latch.clone(), exit.clone()));
        self.block(body, false);
        self.loops.pop();
        self.jump(&latch);
        self.switch_to_block(&latch);
        let n = self.load_from(T::I64, i);
        let n = self.ins(T::I64, &format!("add i64 {}, 1", n.text));
        self.store_to(&n, i);
        self.jump(&header);
        self.switch_to_block(&exit);
    }

    fn assign(&mut self, place: &Place, op: Option<ArithOp>, value: &Expr) {
        match place {
            Place::Local(l) => {
                let ty = self.tys[*l].clone();
                let v = self.expr(value);
                let cur = op.map(|_| self.local(*l));
                let new = self.compound(op, cur, &ty, v);
                self.set_local(*l, new);
            }
            Place::Field { base, field, ty } => {
                let b = self.expr(base);
                let decl = self.field_decl(&base.ty, *field);
                let offset = 8 * *field as i64;
                let v = self.expr(value);
                let cur = match op {
                    Some(_) => {
                        let raw = self.load(T::I64, &b, offset);
                        let raw = self.unkv(raw, &decl);
                        Some(self.convert(raw, &decl, ty))
                    }
                    None => None,
                };
                let new = self.compound(op, cur, ty, v);
                let new = self.convert(new, ty, &decl);
                let k = self.kv_of(new, &decl);
                self.store(&k, &b, offset);
            }
            Place::ListItem { list, index, ty } => {
                let b = self.expr(list);
                let i = self.expr(index);
                let v = self.expr(value);
                let cur = match op {
                    Some(_) => {
                        let raw = self.call_rt("k_list_at", &[b.clone(), i.clone()]);
                        Some(self.unkv(raw, ty))
                    }
                    None => None,
                };
                let new = self.compound(op, cur, ty, v);
                let k = self.kv_of(new, ty);
                self.call_rt("k_list_set", &[b, i, k]);
            }
            Place::MapEntry { map, key } => {
                let (kt, vt) = match &map.ty {
                    Ty::Adt(_, args) if args.len() == 2 => (args[0].clone(), args[1].clone()),
                    _ => (Ty::Unknown, Ty::Unknown),
                };
                let b = self.expr(map);
                let k = self.expr(key);
                let k = self.kv_of(k, &kt);
                let v = self.expr(value);
                let cur = match op {
                    // `map[key] += v` needs the key to be there already.
                    Some(_) => {
                        let raw = self.call_rt("k_map_at", &[b.clone(), k.clone()]);
                        Some(self.unkv(raw, &vt))
                    }
                    None => None,
                };
                let new = self.compound(op, cur, &vt, v);
                let nv = self.kv_of(new, &vt);
                self.call_rt("k_map_set", &[b, k, nv]);
            }
        }
    }

    /// The declared type of a field of a struct, or `Any` for a record's
    /// field, which lives in a slot.
    fn field_decl(&self, base: &Ty, field: usize) -> Ty {
        match base {
            Ty::Adt(id, _) => match &self.g.p.adts[*id].kind {
                AdtKind::Struct(fields) => fields[field].ty.clone(),
                AdtKind::Enum(_) => Ty::Unknown,
            },
            Ty::Record(_) => Ty::Any,
            _ => Ty::Unknown,
        }
    }

    fn compound(&mut self, op: Option<ArithOp>, cur: Option<V>, ty: &Ty, v: V) -> V {
        match (op, cur) {
            (Some(op), Some(cur)) => self.arith(op, cur, v, ty),
            _ => v,
        }
    }

    fn arith(&mut self, op: ArithOp, l: V, r: V, ty: &Ty) -> V {
        if self.g.is_string(ty) {
            return self.call_rt("k_concat", &[l, r]);
        }
        if self.g.rep(ty) == Rep::Float {
            let (l, r) = (self.fit(l, T::F64), self.fit(r, T::F64));
            let inst = match op {
                ArithOp::Add => "fadd",
                ArithOp::Sub => "fsub",
                ArithOp::Mul => "fmul",
                ArithOp::Div => "fdiv",
                ArithOp::Rem => return self.call_rt("fmod", &[l, r]),
            };
            return self.ins(T::F64, &format!("{inst} double {}, {}", l.text, r.text));
        }
        let (l, r) = (self.fit(l, T::I64), self.fit(r, T::I64));
        let intrinsic = match op {
            ArithOp::Add => "sadd",
            ArithOp::Sub => "ssub",
            ArithOp::Mul => "smul",
            ArithOp::Div | ArithOp::Rem => return self.int_div(op, l, r),
        };
        self.g.intrinsic(match op {
            ArithOp::Add => "declare { i64, i1 } @llvm.sadd.with.overflow.i64(i64, i64)",
            ArithOp::Sub => "declare { i64, i1 } @llvm.ssub.with.overflow.i64(i64, i64)",
            _ => "declare { i64, i1 } @llvm.smul.with.overflow.i64(i64, i64)",
        });
        let pair = self.ins(
            T::I64,
            &format!(
                "call {{ i64, i1 }} @llvm.{intrinsic}.with.overflow.i64(i64 {}, i64 {})",
                l.text, r.text
            ),
        );
        let v = self.ins(
            T::I64,
            &format!("extractvalue {{ i64, i1 }} {}, 0", pair.text),
        );
        let overflow = self.ins(
            T::I1,
            &format!("extractvalue {{ i64, i1 }} {}, 1", pair.text),
        );
        let (bad, ok) = (self.create_block(), self.create_block());
        self.brif(overflow, &bad, &ok);
        self.switch_to_block(&bad);
        self.panic_c("integer overflow");
        self.switch_to_block(&ok);
        v
    }

    /// `Int` division or remainder, as the runtime's `k_div` and `k_rem`
    /// do them, written out so that LLVM can optimize around them: a zero
    /// divisor panics, `MIN / -1` panics with an overflow, and `x % -1` is
    /// `0` (which the instruction would trap on).
    fn int_div(&mut self, op: ArithOp, l: V, r: V) -> V {
        let zero = self.ins(T::I1, &format!("icmp eq i64 {}, 0", r.text));
        self.panic_if(zero, "division by zero");
        let minus_one = self.ins(T::I1, &format!("icmp eq i64 {}, -1", r.text));
        if op == ArithOp::Div {
            let min = self.ins(
                T::I1,
                &format!("icmp eq i64 {}, -9223372036854775808", l.text),
            );
            let overflow = self.ins(T::I1, &format!("and i1 {}, {}", min.text, minus_one.text));
            self.panic_if(overflow, "integer overflow");
            return self.ins(T::I64, &format!("sdiv i64 {}, {}", l.text, r.text));
        }
        // The divisor is replaced by 1 when it is -1, so the remainder,
        // 0, is right and `MIN % -1` cannot trap.
        let divisor = self.ins(
            T::I64,
            &format!("select i1 {}, i64 1, i64 {}", minus_one.text, r.text),
        );
        self.ins(T::I64, &format!("srem i64 {}, {}", l.text, divisor.text))
    }

    /// `-x` for an `Int`, panicking on `MIN`, as the runtime's `k_neg`.
    fn int_neg(&mut self, x: V) -> V {
        let x = self.fit(x, T::I64);
        let min = self.ins(
            T::I1,
            &format!("icmp eq i64 {}, -9223372036854775808", x.text),
        );
        self.panic_if(min, "integer overflow");
        self.ins(T::I64, &format!("sub i64 0, {}", x.text))
    }

    /// Panics with the message when `c` holds, in a block of its own.
    fn panic_if(&mut self, c: V, message: &str) {
        let (bad, ok) = (self.create_block(), self.create_block());
        self.brif(c, &bad, &ok);
        self.switch_to_block(&bad);
        self.panic_c(message);
        self.switch_to_block(&ok);
    }

    /// Panics with a fixed message; the current block ends here.
    fn panic_c(&mut self, message: &str) {
        let m = self.c_str(message);
        self.call_rt("k_panic_c", &[m]);
        self.emit("unreachable");
    }

    // ----- expressions -----

    /// Generates an expression whose value is not used.
    fn discard(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.if_chain(cond, then, otherwise.as_ref(), None);
            }
            ExprKind::Match { scrutinee, arms } => {
                self.match_expr(e, scrutinee, arms, false);
            }
            ExprKind::Block(b) => {
                self.block(b, false);
            }
            ExprKind::Catch { .. } => {
                self.catch(e, false);
            }
            ExprKind::Coerce(inner) => self.discard(inner),
            _ => {
                self.expr(e);
            }
        }
    }

    /// Generates an expression. A pointer or `kv` result that is not simply
    /// a local is stored in a root slot, so that it is reachable while
    /// later parts of the enclosing expression run; the slots its own
    /// parts used are free again.
    fn expr(&mut self, e: &Expr) -> V {
        let base = self.frame.as_ref().map(|f| f.top);
        let v = self.expr_inner(e);
        if let Some(base) = base {
            match self.rooting(e) {
                Rooting::New => {
                    self.frame.as_mut().unwrap().top = base;
                    let i = self.temp_slot();
                    self.root_in(i, v.clone());
                }
                Rooting::Inner => {}
                Rooting::None => self.frame.as_mut().unwrap().top = base,
            }
        }
        v
    }

    /// How an expression's value is kept reachable: in a new root slot; in
    /// the slot a part of it already has, for a block's value or a coercion
    /// that leaves a pointer as it is; or not at all, for a value that is
    /// not a pointer or is a local, which has a root slot of its own.
    fn rooting(&self, e: &Expr) -> Rooting {
        if !matches!(self.g.rep(&e.ty), Rep::Ptr | Rep::Kv) {
            return Rooting::None;
        }
        match &e.kind {
            ExprKind::Local(_) | ExprKind::None | ExprKind::Error => Rooting::None,
            ExprKind::Block(_) => Rooting::Inner,
            ExprKind::Coerce(inner) => {
                let boxes = matches!(&e.ty, Ty::Opt(_)) && !matches!(&inner.ty, Ty::Opt(_));
                if boxes || !matches!(self.g.rep(&inner.ty), Rep::Ptr | Rep::Kv) {
                    Rooting::New
                } else if matches!(self.rooting(inner), Rooting::None) {
                    Rooting::None
                } else {
                    Rooting::Inner
                }
            }
            _ => Rooting::New,
        }
    }

    fn expr_inner(&mut self, e: &Expr) -> V {
        match &e.kind {
            ExprKind::Int(n) => V::int(*n),
            ExprKind::Float(x) => V::new(float_const(*x), T::F64),
            ExprKind::Bool(b) => V::bool(*b),
            ExprKind::Str(s) => self.str_const(s),
            ExprKind::None => V::int(0),
            ExprKind::Local(l) => self.local(*l),
            ExprKind::Global(g) => {
                let ty = self.g.cty(&self.g.p.globals[*g].ty);
                let (addr, offset) = self.global_addr(*g);
                if ty == T::I1 {
                    let v = self.load(T::I64, &addr, offset);
                    self.fit(v, T::I1)
                } else {
                    self.load(ty, &addr, offset)
                }
            }
            ExprKind::StoreSlot(id) => {
                let slot = self.call_rt("k_store_slot", &[V::int(*id as i64)]);
                self.load(T::I64, &slot, 0)
            }
            ExprKind::FuncRef(f) => {
                let w = self.g.fn_wrapper(*f);
                let fa = self.func_addr(&w);
                self.call_rt("k_closure", &[fa, V::int(0)])
            }
            ExprKind::Concat(parts) => {
                // The string so far is live while the next part runs, so
                // it has a root slot of its own.
                let slot = self.frame.is_some().then(|| self.temp_slot());
                let mut acc: Option<V> = None;
                for part in parts {
                    let v = self.expr(part);
                    let joined = match acc {
                        None => v,
                        Some(a) => self.call_rt("k_concat", &[a, v]),
                    };
                    if let Some(i) = slot {
                        self.root_in(i, joined.clone());
                    }
                    acc = Some(joined);
                }
                match acc {
                    Some(v) => v,
                    None => self.str_const(""),
                }
            }
            ExprKind::Show(v) => {
                let x = self.expr(v);
                self.display(x, &v.ty, v.span)
            }
            ExprKind::Not(v) => {
                let x = self.expr(v);
                let x = self.fit(x, T::I1);
                self.ins(T::I1, &format!("xor i1 {}, true", x.text))
            }
            ExprKind::Neg(v) => {
                let x = self.expr(v);
                if self.g.rep(&e.ty) == Rep::Float {
                    let x = self.fit(x, T::F64);
                    self.ins(T::F64, &format!("fneg double {}", x.text))
                } else {
                    self.int_neg(x)
                }
            }
            ExprKind::Arith(op, a, b) => {
                let l = self.expr(a);
                let r = self.expr(b);
                self.arith(*op, l, r, &e.ty)
            }
            ExprKind::Cmp(op, a, b) => {
                let l = self.expr(a);
                let r = self.expr(b);
                let (icc, fcc) = match op {
                    CmpOp::Lt => ("slt", "olt"),
                    CmpOp::Le => ("sle", "ole"),
                    CmpOp::Gt => ("sgt", "ogt"),
                    CmpOp::Ge => ("sge", "oge"),
                };
                if self.g.is_kstr(&a.ty) {
                    let c = self.call_rt("k_str_cmp", &[l, r]);
                    self.ins(T::I1, &format!("icmp {icc} i32 {}, 0", c.text))
                } else if self.g.rep(&a.ty) == Rep::Float {
                    let (l, r) = (self.fit(l, T::F64), self.fit(r, T::F64));
                    self.ins(T::I1, &format!("fcmp {fcc} double {}, {}", l.text, r.text))
                } else {
                    let (l, r) = (self.fit(l, T::I64), self.fit(r, T::I64));
                    self.ins(T::I1, &format!("icmp {icc} i64 {}, {}", l.text, r.text))
                }
            }
            ExprKind::Eq(a, b) => {
                let l = self.expr(a);
                let r = self.expr(b);
                self.eq_expr(l, &a.ty, r, &b.ty, e.span)
            }
            ExprKind::IsNone(v) => {
                let x = self.expr(v);
                let x = self.fit(x, T::I64);
                self.ins(T::I1, &format!("icmp eq i64 {}, 0", x.text))
            }
            ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                let and = matches!(e.kind, ExprKind::And(..));
                let l = self.expr(a);
                let l = self.fit(l, T::I1);
                let (rhs, merge) = (self.create_block(), self.create_block());
                let r = self.alloca(T::I1);
                self.store_to(&l, &r);
                if and {
                    self.brif(l, &rhs, &merge);
                } else {
                    self.brif(l, &merge, &rhs);
                }
                self.switch_to_block(&rhs);
                let v = self.expr(b);
                let v = self.fit(v, T::I1);
                self.store_to(&v, &r);
                self.jump(&merge);
                self.switch_to_block(&merge);
                self.load_from(T::I1, &r)
            }
            ExprKind::Coalesce(a, b) => {
                let rt = e.ty.clone();
                let inner = match &a.ty {
                    Ty::Opt(inner) => (**inner).clone(),
                    _ => Ty::Unknown,
                };
                let t = self.expr(a);
                let (some, none, merge) = (
                    self.create_block(),
                    self.create_block(),
                    self.create_block(),
                );
                let void = is_void(&rt);
                let r = (!void).then(|| {
                    let ty = self.g.cty(&rt);
                    (self.alloca(ty), ty)
                });
                self.brif(t.clone(), &some, &none);
                self.switch_to_block(&some);
                if let Some((slot, _)) = &r {
                    let raw = self.load(T::I64, &t, 0);
                    let v = self.unkv(raw, &inner);
                    let v = self.convert(v, &inner, &rt);
                    self.store_to(&v, slot);
                }
                self.jump(&merge);
                self.switch_to_block(&none);
                let rv = self.expr(b);
                if let Some((slot, _)) = &r {
                    let v = if is_void(&b.ty) {
                        self.zero(&rt)
                    } else {
                        self.convert(rv, &b.ty, &rt)
                    };
                    self.store_to(&v, slot);
                }
                self.jump(&merge);
                self.switch_to_block(&merge);
                match r {
                    Some((slot, ty)) => self.load_from(ty, &slot),
                    None => V::int(0),
                }
            }
            ExprKind::Range(a, b) => {
                let l = self.expr(a);
                let r = self.expr(b);
                self.call_rt("k_range", &[l, r])
            }
            ExprKind::List(items) => {
                let elem = match &e.ty {
                    Ty::Adt(_, args) if !args.is_empty() => args[0].clone(),
                    _ => Ty::Unknown,
                };
                let mut vals = Vec::new();
                for item in items {
                    let v = self.expr(item);
                    vals.push(self.kv_of(v, &elem));
                }
                let n = V::int(vals.len() as i64);
                let arr = if vals.is_empty() {
                    V::int(0)
                } else {
                    self.stack_array(&vals)
                };
                self.call_rt("k_list", &[n, arr])
            }
            ExprKind::NewMap => {
                let key = match &e.ty {
                    Ty::Adt(_, targs) if !targs.is_empty() => targs[0].clone(),
                    _ => Ty::Unknown,
                };
                let hash = self.g.hash_fn(&key, e.span);
                let eq = self.g.eq_fn(&key, e.span);
                let (h, q) = (self.func_addr(&hash), self.func_addr(&eq));
                self.call_rt("k_map_new", &[h, q])
            }
            ExprKind::Struct { adt, fields } => {
                if fields.is_empty() {
                    return V::int(0);
                }
                let decls: Vec<Ty> = match &self.g.p.adts[*adt].kind {
                    AdtKind::Struct(fs) => fs.iter().map(|f| f.ty.clone()).collect(),
                    AdtKind::Enum(_) => Vec::new(),
                };
                let vals: Vec<V> = fields
                    .iter()
                    .zip(&decls)
                    .map(|(f, ty)| {
                        let v = self.expr(f);
                        self.kv_of(v, ty)
                    })
                    .collect();
                let bits = self.pointer_bits(&decls);
                let s = self.alloc_bits(vals.len(), bits);
                for (i, v) in vals.into_iter().enumerate() {
                    self.store(&v, &s, 8 * i as i64);
                }
                s
            }
            ExprKind::Variant { adt, index, fields } => {
                let decls: Vec<Ty> = match &self.g.p.adts[*adt].kind {
                    AdtKind::Enum(vs) => vs[*index].fields.iter().map(|(_, t)| t.clone()).collect(),
                    AdtKind::Struct(_) => Vec::new(),
                };
                let mut kvs = Vec::new();
                for (f, ty) in fields.iter().zip(&decls) {
                    let v = self.expr(f);
                    kvs.push(self.kv_of(v, ty));
                }
                let tag = V::int(*index as i64);
                let n = V::new(kvs.len().to_string(), T::I32);
                let arr = if kvs.is_empty() {
                    V::int(0)
                } else {
                    self.stack_array(&kvs)
                };
                self.call_rt("k_enum", &[tag, n, arr])
            }
            ExprKind::Field { base, field } => {
                let b = self.expr(base);
                let raw = self.load(T::I64, &b, 8 * *field as i64);
                self.unkv(raw, &e.ty)
            }
            ExprKind::Record(values) => {
                let vals: Vec<V> = values
                    .iter()
                    .map(|v| {
                        let x = self.expr(v);
                        self.kv_of(x, &v.ty)
                    })
                    .collect();
                let n = V::new(vals.len().to_string(), T::I32);
                let arr = self.stack_array(&vals);
                self.call_rt("k_record", &[n, arr])
            }
            ExprKind::ListItem { list, index } => {
                let b = self.expr(list);
                let i = self.expr(index);
                let raw = self.call_rt("k_list_at", &[b, i]);
                self.unkv(raw, &e.ty)
            }
            ExprKind::MapGet { map, key } => {
                let kt = match &map.ty {
                    Ty::Adt(_, args) if !args.is_empty() => args[0].clone(),
                    _ => Ty::Unknown,
                };
                let b = self.expr(map);
                let k = self.expr(key);
                let k = self.kv_of(k, &kt);
                self.call_rt("k_map_get", &[b, k])
            }
            ExprKind::Call { func, args } => {
                let vals: Vec<V> = args.iter().map(|a| self.expr(a)).collect();
                let (name, sig) = self.g.funcs[*func].clone();
                match self.call(&name, &sig, &vals) {
                    Some(v) => v,
                    None => V::int(0),
                }
            }
            ExprKind::CallClosure { callee, args } => {
                let Ty::Fn(f) = &callee.ty else {
                    self.g.unsupported(e.span, "this call");
                    return V::int(0);
                };
                let f = f.clone();
                let c = self.expr(callee);
                let mut kvs = Vec::new();
                for (a, ty) in args.iter().zip(&f.params) {
                    let v = self.expr(a);
                    kvs.push(self.kv_of(v, ty));
                }
                if kvs.is_empty() {
                    kvs.push(V::int(0));
                }
                let arr = self.stack_array(&kvs);
                let func = self.load(T::I64, &c, 0);
                let env = self.load(T::I64, &c, 8);
                let sig = Sig::new(&[T::I64, T::I64], Some(T::I64));
                let raw = self.call_indirect(&sig, &func, &[env, arr]).unwrap();
                if is_void(&f.ret) {
                    return V::int(0);
                }
                self.unkv(raw, &f.ret)
            }
            ExprKind::Method { name, recv, args } => {
                let r = self.expr(recv);
                let vals: Vec<V> = args.iter().map(|a| self.expr(a)).collect();
                self.builtin_method(r, &recv.ty, name, &vals, args, e.span)
            }
            ExprKind::Lib { module, name, args } => {
                if let Some(rf) = kiln_ir::rt::rt_fn(module, name) {
                    return self.rt_call(rf, args, &e.ty);
                }
                let vals: Vec<V> = args.iter().map(|a| self.expr(a)).collect();
                let tys: Vec<Ty> = args.iter().map(|a| a.described().clone()).collect();
                self.lib_call(module, name, &vals, &tys, &e.ty, e.span)
            }
            ExprKind::Print(v) => {
                let x = self.expr(v);
                self.call_rt("k_print", &[x])
            }
            ExprKind::Env(v) => {
                let x = self.expr(v);
                self.call_rt("k_os_env", &[x])
            }
            ExprKind::Panic(v) => {
                let x = self.expr(v);
                self.call_rt("k_panic", &[x])
            }
            ExprKind::Log { level, args } => {
                let mut shown: Vec<V> = args.iter().map(|a| self.expr(a)).collect();
                let message = shown.remove(0);
                let lv = self.c_str(level);
                let n = V::new(shown.len().to_string(), T::I32);
                let data = if shown.is_empty() {
                    V::int(0)
                } else {
                    self.stack_array(&shown)
                };
                self.call_rt("k_log", &[lv, message, n, data])
            }
            ExprKind::Closure { func, captures } => {
                let f = self.g.funcs[*func].clone();
                let fa = self.func_addr(&f);
                if captures.is_empty() {
                    return self.call_rt("k_closure", &[fa, V::int(0)]);
                }
                // A captured cell is a pointer; a captured value is a
                // pointer when its type is.
                let mut bits = 0;
                for (i, &l) in captures.iter().enumerate() {
                    let ptr = matches!(self.locals[l], Slot::Cell(..))
                        || matches!(self.g.rep(&self.tys[l]), Rep::Ptr | Rep::Kv);
                    if ptr && i < 64 {
                        bits |= 1 << i;
                    }
                }
                let env = self.alloc_bits(captures.len(), bits);
                for (i, &l) in captures.iter().enumerate() {
                    let v = match self.locals[l].clone() {
                        Slot::Cell(a, _) => self.load_from(T::I64, &a),
                        Slot::Var(..) => {
                            let v = self.local(l);
                            let ty = self.tys[l].clone();
                            self.kv_of(v, &ty)
                        }
                    };
                    self.store(&v, &env, 8 * i as i64);
                }
                self.call_rt("k_closure", &[fa, env])
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                let rt = e.ty.clone();
                let want = (!is_void(&rt)).then_some(&rt);
                self.if_chain(cond, then, otherwise.as_ref(), want)
            }
            ExprKind::Match { scrutinee, arms } => self.match_expr(e, scrutinee, arms, true),
            ExprKind::Is { value, pattern } => {
                let vt = value.ty.clone();
                let s = self.expr(value);
                let (fail, merge) = (self.create_block(), self.create_block());
                let r = self.alloca(T::I1);
                self.pattern(pattern, s, &vt, &fail);
                self.store_to(&V::bool(true), &r);
                self.jump(&merge);
                self.switch_to_block(&fail);
                self.store_to(&V::bool(false), &r);
                self.jump(&merge);
                self.switch_to_block(&merge);
                self.load_from(T::I1, &r)
            }
            ExprKind::Block(b) => self.block(b, true),
            ExprKind::Try { expr, wrap } => {
                let v = self.expr(expr);
                self.propagate(*wrap);
                v
            }
            ExprKind::Catch { .. } => self.catch(e, true),
            ExprKind::Throw { value, wrap } => {
                self.throw(value, *wrap);
                V::int(0)
            }
            ExprKind::Coerce(v) => {
                let x = self.expr(v);
                self.convert(x, &v.ty, &e.ty)
            }
            ExprKind::Error => V::int(0),
        }
    }

    fn if_chain(
        &mut self,
        cond: &Expr,
        then: &Block,
        otherwise: Option<&Block>,
        rt: Option<&Ty>,
    ) -> V {
        let c = self.expr(cond);
        let (then_b, else_b, merge) = (
            self.create_block(),
            self.create_block(),
            self.create_block(),
        );
        let r = rt.map(|t| {
            let ty = self.g.cty(t);
            (self.alloca(ty), ty)
        });
        self.brif(c, &then_b, &else_b);
        self.switch_to_block(&then_b);
        let v = self.block(then, rt.is_some());
        self.branch_to(&merge, r.as_ref(), v, &block_ty(then), rt);
        self.switch_to_block(&else_b);
        match otherwise {
            Some(other) => {
                let v = self.block(other, rt.is_some());
                self.branch_to(&merge, r.as_ref(), v, &block_ty(other), rt);
            }
            None => {
                self.branch_to(&merge, r.as_ref(), V::int(0), &Ty::Unit, rt);
            }
        }
        self.switch_to_block(&merge);
        match r {
            Some((slot, ty)) => self.load_from(ty, &slot),
            None => V::int(0),
        }
    }

    /// Jumps to a merge block, leaving the branch's value moved to `rt` in
    /// the result slot when there is one.
    fn branch_to(&mut self, merge: &str, slot: Option<&(V, T)>, v: V, vt: &Ty, rt: Option<&Ty>) {
        if let (Some(rt), Some((slot, _))) = (rt, slot) {
            let v = if is_void(vt) {
                self.zero(rt)
            } else {
                self.convert(v, vt, rt)
            };
            self.store_to(&v, slot);
        }
        self.jump(merge);
    }

    fn match_expr(&mut self, e: &Expr, scrutinee: &Expr, arms: &[Arm], want: bool) -> V {
        let rt = e.ty.clone();
        let st = scrutinee.ty.clone();
        let s = self.expr(scrutinee);
        let merge = self.create_block();
        let keep = want && !is_void(&rt);
        let r = keep.then(|| {
            let ty = self.g.cty(&rt);
            (self.alloca(ty), ty)
        });
        let rt_opt = keep.then_some(&rt);
        for arm in arms {
            let next = self.create_block();
            self.pattern(&arm.pattern, s.clone(), &st, &next);
            if let Some(guard) = &arm.guard {
                let g = self.expr(guard);
                let body = self.create_block();
                self.brif(g, &body, &next);
                self.switch_to_block(&body);
            }
            if keep {
                let v = self.expr(&arm.body);
                self.branch_to(&merge, r.as_ref(), v, &arm.body.ty, rt_opt);
            } else {
                self.discard(&arm.body);
                self.jump(&merge);
            }
            self.switch_to_block(&next);
        }
        self.panic_c("no arm of this match matched");
        self.switch_to_block(&merge);
        match r {
            Some((slot, ty)) => self.load_from(ty, &slot),
            None => V::int(0),
        }
    }

    /// Tests the value `val` of type `ty` against a pattern, jumping to
    /// `fail` when it does not match, and binds the pattern's names when it
    /// does.
    fn pattern(&mut self, p: &Pattern, val: V, ty: &Ty, fail: &str) {
        match p {
            Pattern::Wildcard => {}
            Pattern::Bind(l) => self.set_local(*l, val),
            Pattern::Literal(lit) => {
                let l = self.expr(lit);
                let eq = self.eq_expr(val, ty, l, &lit.ty, lit.span);
                let ok = self.create_block();
                self.brif(eq, &ok, fail);
                self.switch_to_block(&ok);
            }
            Pattern::None => {
                let ok = self.create_block();
                self.brif(val, fail, &ok);
                self.switch_to_block(&ok);
            }
            Pattern::Some(inner, ity) => {
                let ok = self.create_block();
                self.brif(val.clone(), &ok, fail);
                self.switch_to_block(&ok);
                let raw = self.load(T::I64, &val, 0);
                let v = self.unkv(raw, ity);
                self.pattern(inner, v, ity, fail);
            }
            Pattern::Variant { index, fields, .. } => {
                let tag = self.load(T::I64, &val, 0);
                let same = self.ins(T::I1, &format!("icmp eq i64 {}, {index}", tag.text));
                let ok = self.create_block();
                self.brif(same, &ok, fail);
                self.switch_to_block(&ok);
                for (i, (sub, fty)) in fields.iter().enumerate() {
                    let raw = self.load(T::I64, &val, 8 + 8 * i as i64);
                    let v = self.unkv(raw, fty);
                    self.pattern(sub, v, fty, fail);
                }
            }
        }
    }

    // ----- showing and comparing -----

    /// A `kstr *` showing the value, as `print` and interpolation do.
    fn display(&mut self, v: V, ty: &Ty, span: Span) -> V {
        let p = self.g.prims;
        match ty {
            _ if self.g.is_string(ty) => v,
            _ if self.g.is_bytes(ty) => self.call_rt("k_bytes_show", &[v]),
            Ty::Adt(id, _) if *id == p.int => self.call_rt("k_int_str", &[v]),
            Ty::Adt(id, _) if *id == p.float => self.call_rt("k_float_str", &[v]),
            Ty::Adt(id, _) if *id == p.bool_ => self.call_rt("k_bool_str", &[v]),
            Ty::Adt(id, _) if *id == p.duration => self.call_rt("k_duration_str", &[v]),
            _ => {
                self.g.check(Op::Show, ty, span);
                let d = self.g.desc(ty);
                let addr = self.data_addr(&d);
                let k = self.kv_of(v, ty);
                self.call_rt("k_desc_show", &[addr, k, V::bool(false)])
            }
        }
    }

    /// Whether two values are equal.
    fn eq_expr(&mut self, a: V, at: &Ty, b: V, bt: &Ty, span: Span) -> V {
        let (a, at, b) = match (at, bt) {
            (Ty::Opt(_), other) if !matches!(other, Ty::Opt(_)) => {
                let b = self.convert(b, bt, at);
                (a, at.clone(), b)
            }
            (other, Ty::Opt(_)) if !matches!(other, Ty::Opt(_)) => {
                let a = self.convert(a, at, bt);
                (a, bt.clone(), b)
            }
            _ => (a, at.clone(), b),
        };
        if self.g.is_kstr(&at) {
            return self.call_rt("k_str_eq", &[a, b]);
        }
        match self.g.rep(&at) {
            Rep::Float => {
                let (a, b) = (self.fit(a, T::F64), self.fit(b, T::F64));
                self.ins(T::I1, &format!("fcmp oeq double {}, {}", a.text, b.text))
            }
            Rep::Int | Rep::Bool => {
                let t = a.ty;
                let b = self.fit(b, t);
                self.ins(
                    T::I1,
                    &format!("icmp eq {} {}, {}", t.name(), a.text, b.text),
                )
            }
            _ => {
                self.g.check(Op::Eq, &at, span);
                let d = self.g.desc(&at);
                let addr = self.data_addr(&d);
                let (x, y) = (self.kv_of(a, &at), self.kv_of(b, &at));
                self.call_rt("k_desc_eq", &[addr, x, y])
            }
        }
    }

    // ----- standard library methods -----

    fn builtin_method(
        &mut self,
        r: V,
        recv_ty: &Ty,
        name: &str,
        a: &[V],
        args: &[Expr],
        span: Span,
    ) -> V {
        let Ty::Adt(id, targs) = recv_ty else {
            return self.unsupported(span, format!("`.{name}`"));
        };
        let id = *id;
        let arg = |i: usize| a[i].clone();
        let p = self.g.prims;
        if id == p.int {
            return match name {
                "to_string" => self.call_rt("k_int_str", &[r]),
                "to_float" => self.ins(T::F64, &format!("sitofp i64 {} to double", r.text)),
                "abs" => {
                    let negative = self.ins(T::I1, &format!("icmp slt i64 {}, 0", r.text));
                    let negated = self.int_neg(r.clone());
                    self.ins(
                        T::I64,
                        &format!(
                            "select i1 {}, i64 {}, i64 {}",
                            negative.text, negated.text, r.text
                        ),
                    )
                }
                "min" | "max" => {
                    let o = self.fit(arg(0), T::I64);
                    let cc = if name == "min" { "slt" } else { "sgt" };
                    let c = self.ins(T::I1, &format!("icmp {cc} i64 {}, {}", r.text, o.text));
                    self.ins(
                        T::I64,
                        &format!("select i1 {}, i64 {}, i64 {}", c.text, r.text, o.text),
                    )
                }
                "pow" => self.call_rt("k_pow", &[r, arg(0)]),
                "clamp" => self.call_rt("k_clamp", &[r, arg(0), arg(1)]),
                _ => self.unsupported(span, format!("`Int.{name}`")),
            };
        }
        if id == p.duration {
            return match name {
                "to_millis" => self.ins(T::I64, &format!("sdiv i64 {}, 1000000", r.text)),
                "to_seconds" => {
                    let f = self.ins(T::F64, &format!("sitofp i64 {} to double", r.text));
                    self.ins(T::F64, &format!("fdiv double {}, 1.0e9", f.text))
                }
                _ => self.unsupported(span, format!("`Duration.{name}`")),
            };
        }
        if self.is_std_adt(id, "Instant") {
            let off = 8 * self.g.field_index(id, "unix_nanos") as i64;
            let ns = self.load(T::I64, &r, off);
            let Some(ret) = self.method_ret(recv_ty, name) else {
                return self.unsupported(span, format!("`Instant.{name}`"));
            };
            return match name {
                "unix" => self.call_rt("k_time_unix", &[ns]),
                "add" => {
                    let n = self.call_rt("k_add", &[ns, arg(0)]);
                    self.new_instant(n, &ret)
                }
                "since" => {
                    let other = self.load(T::I64, &arg(0), off);
                    self.call_rt("k_sub", &[ns, other])
                }
                "date" => self.new_date(ns, &ret),
                "iso" => self.call_rt("k_time_iso", &[ns]),
                "format" => self.call_rt("k_time_format", &[ns, arg(0)]),
                _ => self.unsupported(span, format!("`Instant.{name}`")),
            };
        }
        if id == p.float {
            return match name {
                "to_string" => self.call_rt("k_float_str", &[r]),
                "round" | "floor" | "ceil" => {
                    let f = match name {
                        "round" => self.call_rt("round", &[r]),
                        "floor" => self.call_rt("floor", &[r]),
                        _ => self.call_rt("ceil", &[r]),
                    };
                    self.call_rt("k_float_to_int", &[f])
                }
                "abs" => {
                    self.g.intrinsic("declare double @llvm.fabs.f64(double)");
                    let r = self.fit(r, T::F64);
                    self.ins(
                        T::F64,
                        &format!("call double @llvm.fabs.f64(double {})", r.text),
                    )
                }
                "sqrt" => {
                    self.g.intrinsic("declare double @llvm.sqrt.f64(double)");
                    let r = self.fit(r, T::F64);
                    self.ins(
                        T::F64,
                        &format!("call double @llvm.sqrt.f64(double {})", r.text),
                    )
                }
                "min" => self.call_rt("fmin", &[r, arg(0)]),
                "max" => self.call_rt("fmax", &[r, arg(0)]),
                "fixed" => self.call_rt("k_float_fixed", &[r, arg(0)]),
                _ => self.unsupported(span, format!("`Float.{name}`")),
            };
        }
        if id == p.string {
            let f: &'static str = match name {
                "len" => "k_str_len",
                "is_empty" => {
                    let len = self.load(T::I64, &r, 0);
                    return self.ins(T::I1, &format!("icmp eq i64 {}, 0", len.text));
                }
                "upper" => "k_str_upper",
                "lower" => "k_str_lower",
                "trim" => "k_str_trim",
                "trim_start" => "k_str_trim_start",
                "trim_end" => "k_str_trim_end",
                "to_int" => "k_str_to_int",
                "to_float" => "k_str_to_float",
                "chars" => "k_str_chars",
                "lines" => "k_str_lines",
                "contains" => "k_str_contains",
                "starts_with" => "k_str_starts_with",
                "ends_with" => "k_str_ends_with",
                "split" => "k_str_split",
                "index_of" => "k_str_index_of",
                "repeat" => "k_str_repeat",
                "replace" => "k_str_replace",
                "slice" => "k_str_slice",
                "pad_start" => "k_str_pad_start",
                "pad_end" => "k_str_pad_end",
                _ => return self.unsupported(span, format!("`String.{name}`")),
            };
            let mut all = vec![r];
            all.extend_from_slice(a);
            return self.call_rt(f, &all);
        }
        if id == p.list {
            let elem = targs.first().cloned().unwrap_or(Ty::Unknown);
            let f: &'static str = match name {
                "len" => return self.load(T::I64, &r, 0),
                "is_empty" => {
                    let len = self.load(T::I64, &r, 0);
                    return self.ins(T::I1, &format!("icmp eq i64 {}, 0", len.text));
                }
                "contains" | "index_of" => {
                    let eq = self.g.eq_fn(&elem, span);
                    let fa = self.func_addr(&eq);
                    let f = if name == "contains" {
                        "k_list_contains"
                    } else {
                        "k_list_index_of"
                    };
                    return self.call_rt(f, &[r, arg(0), fa]);
                }
                "sort" => {
                    let cmp = self.g.cmp_fn(&elem, span);
                    let fa = self.func_addr(&cmp);
                    return self.call_rt("k_list_sort", &[r, fa]);
                }
                "min" | "max" => {
                    let cmp = self.g.cmp_fn(&elem, span);
                    let fa = self.func_addr(&cmp);
                    let sign = V::new(if name == "min" { "1" } else { "-1" }, T::I32);
                    return self.call_rt("k_list_extreme", &[r, fa, sign]);
                }
                "sort_by" => {
                    let key = match args.first().map(|a| &a.uncoerced().ty) {
                        Some(Ty::Fn(f)) => f.ret.clone(),
                        _ => Ty::Unknown,
                    };
                    let cmp = self.g.cmp_fn(&key, span);
                    let fa = self.func_addr(&cmp);
                    return self.call_rt("k_list_sort_by", &[r, arg(0), fa]);
                }
                "join" => {
                    let show = self.g.show_fn(&elem, false, span);
                    let fa = self.func_addr(&show);
                    return self.call_rt("k_list_join", &[r, arg(0), fa]);
                }
                "first" => "k_list_first",
                "last" => "k_list_last",
                "reverse" => "k_list_reverse",
                "pop" => "k_list_pop",
                "get" => "k_list_get",
                "push" => "k_list_push",
                "filter" => "k_list_filter",
                "map" => "k_list_map",
                "find" => "k_list_find",
                "any" => "k_list_any",
                "all" => "k_list_all",
                "remove_at" => "k_list_remove_at",
                "take" => "k_list_take",
                "skip" => "k_list_skip",
                "concat" => "k_list_concat",
                "count" => "k_list_count",
                "flat_map" => "k_list_flat_map",
                "insert" => "k_list_insert",
                "slice" => "k_list_slice",
                "fold" => "k_list_fold",
                _ => return self.unsupported(span, format!("`List.{name}`")),
            };
            let mut all = vec![r];
            all.extend_from_slice(a);
            return self.call_rt(f, &all);
        }
        if id == p.map || id == p.set {
            let set = id == p.set;
            let f: &'static str = match name {
                "len" => return self.load(T::I64, &r, 0),
                "is_empty" => {
                    let len = self.load(T::I64, &r, 0);
                    return self.ins(T::I1, &format!("icmp eq i64 {}, 0", len.text));
                }
                "keys" | "to_list" => "k_map_keys",
                "values" => "k_map_values",
                "entries" => "k_map_entries",
                "get" => "k_map_get",
                "contains" => "k_map_contains",
                "remove" if set => "k_set_remove",
                "remove" => "k_map_remove",
                "add" => "k_set_add",
                "set" => "k_map_set",
                _ => {
                    let what = format!("`{}.{name}`", self.g.p.adts[id].name);
                    return self.unsupported(span, what);
                }
            };
            let mut all = vec![r];
            all.extend_from_slice(a);
            return self.call_rt(f, &all);
        }
        let what = format!("`{}.{name}`", self.g.p.adts[id].name);
        self.unsupported(span, what)
    }

    fn unsupported(&mut self, span: Span, what: String) -> V {
        self.g.unsupported(span, what);
        V::int(0)
    }

    // ----- standard library modules -----

    fn is_std_adt(&self, id: AdtId, name: &str) -> bool {
        id < self.g.p.std_adts && self.g.p.adts[id].name == name
    }

    fn method_ret(&self, recv_ty: &Ty, name: &str) -> Option<Ty> {
        let Ty::Adt(id, _) = recv_ty else { return None };
        self.g.p.adts[*id]
            .methods
            .iter()
            .find(|m| m.name == name)
            .map(|m| m.ret.clone())
    }

    /// A new `time.Instant` of type `ty` at `ns` nanoseconds.
    fn new_instant(&mut self, ns: V, ty: &Ty) -> V {
        let Ty::Adt(id, _) = ty else {
            return V::int(0);
        };
        let n = self.g.field_count(*id);
        let t = self.alloc_bits(n, 0);
        let off = 8 * self.g.field_index(*id, "unix_nanos") as i64;
        self.store(&ns, &t, off);
        t
    }

    /// A new `Date` of type `ty`: the UTC date of the instant `ns`.
    fn new_date(&mut self, ns: V, ty: &Ty) -> V {
        let Ty::Adt(id, _) = ty else {
            return V::int(0);
        };
        let n = self.g.field_count(*id);
        let t = self.alloc_bits(n, 0);
        for (part, f) in [
            ("year", "k_time_year"),
            ("month", "k_time_month"),
            ("day", "k_time_day"),
        ] {
            let v = self.call_rt(f, std::slice::from_ref(&ns));
            let off = 8 * self.g.field_index(*id, part) as i64;
            self.store(&v, &t, off);
        }
        t
    }

    /// A call to a function of the `time`, `fs`, `os`, `io` or `web` module,
    /// or a method of a `web` type. A
    /// function that can fail is followed by the check that turns the
    /// runtime's message into a failed standard `Error`.
    fn lib_call(
        &mut self,
        module: &str,
        name: &str,
        a: &[V],
        tys: &[Ty],
        ty: &Ty,
        span: Span,
    ) -> V {
        let (v, fallible) = match (module, name) {
            ("time", "now") => {
                let n = self.call_rt("k_time_now", &[]);
                (self.new_instant(n, ty), false)
            }
            ("time", "unix") => {
                let sec = V::int(1_000_000_000);
                let ns = self.call_rt("k_mul", &[a[0].clone(), sec]);
                (self.new_instant(ns, ty), false)
            }
            ("time", "since") => {
                let now = self.call_rt("k_time_now", &[]);
                let instant = self.g.p.adts.iter().position(|d| d.name == "Instant");
                let off = instant.map_or(0, |id| 8 * self.g.field_index(id, "unix_nanos") as i64);
                let then = self.load(T::I64, &a[0], off);
                (self.call_rt("k_sub", &[now, then]), false)
            }
            ("time", "sleep") => (self.call_rt("k_time_sleep", a), false),
            ("time", "today") => {
                let n = self.call_rt("k_time_now", &[]);
                (self.new_date(n, ty), false)
            }
            ("time", "parse") => {
                let n = self.call_rt("k_time_parse", a);
                (self.new_instant(n, ty), true)
            }
            ("fs", _) => {
                let f: &'static str = match name {
                    "read" => "k_fs_read",
                    "read_lines" => "k_fs_read_lines",
                    "write" => "k_fs_write",
                    "append" => "k_fs_append",
                    "list" => "k_fs_list",
                    "mkdir" => "k_fs_mkdir",
                    "remove" => "k_fs_remove",
                    "rename" => "k_fs_rename",
                    "exists" => return self.call_rt("k_fs_exists", a),
                    "is_dir" => return self.call_rt("k_fs_is_dir", a),
                    _ => return self.unsupported(span, format!("`{module}.{name}`")),
                };
                (self.call_rt(f, a), true)
            }
            ("os", _) => {
                let f: &'static str = match name {
                    "args" => "k_os_args",
                    "cwd" => "k_os_cwd",
                    "temp_dir" => "k_os_temp_dir",
                    "pid" => "k_os_pid",
                    "platform" => "k_os_platform",
                    "exit" => "k_os_exit",
                    _ => return self.unsupported(span, format!("`{module}.{name}`")),
                };
                (self.call_rt(f, a), false)
            }
            ("io", _) => {
                let f: &'static str = match name {
                    "read_line" => "k_io_read_line",
                    "read_all" => "k_io_read_all",
                    "read_lines" => "k_io_read_lines",
                    "write" => "k_io_write",
                    "flush" => "k_io_flush",
                    "print_err" => "k_io_print_err",
                    _ => return self.unsupported(span, format!("`{module}.{name}`")),
                };
                (self.call_rt(f, a), false)
            }
            ("html", "escape") => (self.call_rt("k_rt_html_escape", a), false),
            ("html", "safe_url") => (self.call_rt("k_rt_html_safe_url", a), false),
            ("ui", "merge") => (self.call_rt("k_rt_ui_merge", a), false),
            ("base64", "encode") => (self.call_rt("k_rt_base64_encode", a), false),
            ("markdown", "to_html") => (self.call_rt("k_rt_markdown_to_html", a), false),
            ("base64", "decode") => (self.call_rt("k_base64_decode", a), true),
            ("json", "encode") => {
                let d = self.g.desc(&tys[0]);
                let addr = self.data_addr(&d);
                (self.call_rt("k_json_encode", &[addr, a[0].clone()]), false)
            }
            ("json", "decode") | ("web", "Request.json") => {
                let d = self.g.desc(ty);
                let addr = self.data_addr(&d);
                let f = if module == "json" {
                    "k_json_decode"
                } else {
                    "k_web_req_json"
                };
                let raw = self.call_rt(f, &[addr, a[0].clone()]);
                (self.unkv(raw, ty), true)
            }
            ("web", "json") => {
                let d = self.g.desc(&tys[0]);
                let addr = self.data_addr(&d);
                (
                    self.call_rt("k_web_json", &[addr, a[0].clone(), a[1].clone()]),
                    false,
                )
            }
            ("web", "App.listen") => (self.call_rt("k_web_listen", a), true),
            ("web", "Route.static") => {
                let list = match &tys[1] {
                    Ty::Fn(f) => f.ret.clone(),
                    other => other.clone(),
                };
                let d = self.g.desc(&list);
                let addr = self.data_addr(&d);
                (
                    self.call_rt(
                        "k_web_route_static",
                        &[a[0].clone(), a[1].clone(), a[2].clone(), addr],
                    ),
                    false,
                )
            }
            ("web", "Route.dynamic") => (self.call_rt("k_web_route_dynamic", a), false),
            ("stores", "enter") => (self.call_rt("k_store_enter", a), false),
            ("stores", "leave") => (self.call_rt("k_store_leave", a), false),
            ("web", _) => {
                let f: &'static str = match name {
                    "app" => "k_web_app",
                    "not_found" => "k_web_not_found",
                    "__request" => "k_web_current_request",
                    "error" => "k_web_error",
                    "App.on_error" => "k_web_on_error",
                    "App.before" => "k_web_before",
                    "text" => "k_web_text",
                    "html" => "k_web_html",
                    "redirect" => "k_web_redirect",
                    "__with_headers" => "k_web_with_headers",
                    "Request.query" => "k_web_query",
                    "Request.header" => "k_web_header",
                    "Request.cookie" => "k_web_cookie",
                    "App.get" | "App.post" | "App.put" | "App.patch" | "App.delete" => {
                        let index = ["get", "post", "put", "patch", "delete"]
                            .iter()
                            .position(|m| name.ends_with(&format!(".{m}")))
                            .unwrap_or(0);
                        let method = V::int(index as i64);
                        return self.call_rt(
                            "k_web_route",
                            &[a[0].clone(), method, a[1].clone(), a[2].clone()],
                        );
                    }
                    _ => return self.unsupported(span, format!("`{module}.{name}`")),
                };
                (self.call_rt(f, a), false)
            }
            _ => return self.unsupported(span, format!("`{module}.{name}`")),
        };
        if fallible {
            self.check_lib_error();
        }
        v
    }

    /// A call of a function of the Rust runtime's table (see
    /// [`kiln_ir::rt`]): every argument in a slot, after the type
    /// description it asks for.
    fn rt_call(&mut self, rf: &'static kiln_ir::rt::RtFn, args: &[Expr], ty: &Ty) -> V {
        let mut vals = Vec::new();
        let described = match rf.desc {
            kiln_ir::rt::DescOf::None => None,
            kiln_ir::rt::DescOf::Ret => Some(ty.clone()),
            kiln_ir::rt::DescOf::Arg(i) => args.get(i).map(|a| a.described().clone()),
        };
        if let Some(t) = described {
            let d = self.g.desc(&t);
            vals.push(self.data_addr(&d));
        }
        for a in args {
            let v = self.expr(a);
            vals.push(self.kv_of(v, &a.ty));
        }
        let raw = self.call_rt(rf.symbol, &vals);
        if rf.fallible {
            self.check_lib_error();
        }
        self.unkv(raw, ty)
    }

    /// After a library call that can fail: fails with a standard `Error`
    /// when it stored a message in `k_lib_error`.
    fn check_lib_error(&mut self) {
        let addr = V::new(self.g.data_import("k_lib_error"), T::I64);
        let message = self.load(T::I64, &addr, 0);
        let (bad, ok) = (self.create_block(), self.create_block());
        self.brif(message.clone(), &bad, &ok);
        self.switch_to_block(&bad);
        let w = self.new_error(message);
        self.store(&V::int(0), &addr, 0);
        let show = self.g.error_message_fn();
        let fa = self.func_addr(&show);
        self.call_rt("k_fail", &[w, fa]);
        self.jump(&ok);
        self.switch_to_block(&ok);
    }
}

fn is_void(ty: &Ty) -> bool {
    matches!(ty, Ty::Unit | Ty::Never)
}

/// A symbol name for a Kiln name.
fn mangle(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// The type of a block's value.
fn block_ty(b: &Block) -> Ty {
    b.value.as_ref().map(|v| v.ty.clone()).unwrap_or(Ty::Unit)
}

/// A `double` constant with its exact bits, so no decimal rounding happens.
fn float_const(x: f64) -> String {
    format!("0x{:016X}", x.to_bits())
}
