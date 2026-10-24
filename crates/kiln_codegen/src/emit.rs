//! Emits C for a checked program.
//!
//! Every Kiln function becomes a C function. Expressions are lowered by
//! [`Gen::expr`], which writes any statements it needs (calls, branches,
//! loops) into the current function and returns a C expression for the
//! value. Calls are always stored in temporaries, so the returned
//! expressions have no side effects and evaluation follows source order.
//! Locals and temporaries are declared at the top of their C function,
//! which lets `is` patterns bind names inside conditions.
//!
//! Values that generic code handles (type parameters, list items, enum
//! fields, closure arguments, optionals' contents) are stored as `kv`
//! 64-bit slots; [`Gen::convert`] moves values between the two forms.

use kiln_check::types::{substitute, AdtId, AdtKind, FnSig, FnTy, ParamSig, Ty};
use kiln_check::Program;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

pub(crate) fn emit(module: &Module, program: &Program) -> Result<String, Vec<Diagnostic>> {
    let mut gen = Gen::new(module, program);
    gen.program();
    if gen.diags.is_empty() {
        Ok(gen.finish())
    } else {
        let mut diags = gen.diags;
        diags.sort_by_key(|d| (d.span.start, d.span.end));
        diags.dedup_by(|a, b| a.span == b.span && a.message == b.message);
        Err(diags)
    }
}

/// How a type is stored in C.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rep {
    Int,
    Float,
    Bool,
    Ptr,
    Kv,
    Void,
}

#[derive(Clone)]
struct Local {
    /// The C expression that reads or writes the local.
    c: String,
    ty: Ty,
    /// For a `var` that closures capture: the pointer to its cell.
    cell: Option<String>,
}

struct FnState {
    decls: String,
    body: String,
    depth: usize,
    scopes: Vec<HashMap<String, Local>>,
    ret: Ty,
    throws: Option<Ty>,
    closure: bool,
}

impl FnState {
    fn new(ret: Ty, throws: Option<Ty>, closure: bool) -> Self {
        FnState {
            decls: String::new(),
            body: String::new(),
            depth: 1,
            scopes: vec![HashMap::new()],
            ret,
            throws,
            closure,
        }
    }
}

struct Prims {
    int: AdtId,
    float: AdtId,
    bool_: AdtId,
    string: AdtId,
    duration: AdtId,
    list: AdtId,
    error: AdtId,
}

struct Gen<'a> {
    p: &'a Program,
    module: &'a Module,
    fns: HashMap<String, &'a FnDecl>,
    methods: HashMap<(AdtId, String), &'a FnDecl>,
    structs: HashMap<AdtId, &'a StructDecl>,
    prims: Prims,
    diags: Vec<Diagnostic>,
    protos: String,
    globals: String,
    helpers: String,
    bodies: String,
    helper_names: HashMap<String, String>,
    counter: usize,
    f: FnState,
    /// Names used inside closures in the current top-level function. A
    /// `var` with one of these names lives in a cell so closures share it.
    captured: HashSet<String>,
}

impl<'a> Gen<'a> {
    fn new(module: &'a Module, p: &'a Program) -> Self {
        let id = |name: &str| p.global_types[name];
        Gen {
            p,
            module,
            fns: HashMap::new(),
            methods: HashMap::new(),
            structs: HashMap::new(),
            prims: Prims {
                int: id("Int"),
                float: id("Float"),
                bool_: id("Bool"),
                string: id("String"),
                duration: id("Duration"),
                list: id("List"),
                error: id("Error"),
            },
            diags: Vec::new(),
            protos: String::new(),
            globals: String::new(),
            helpers: String::new(),
            bodies: String::new(),
            helper_names: HashMap::new(),
            counter: 0,
            f: FnState::new(Ty::Unit, None, false),
            captured: HashSet::new(),
        }
    }

    // ----- the program -----

    fn program(&mut self) {
        for item in &self.module.items {
            match item {
                Item::Fn(f) => {
                    self.fns.insert(f.name.name.clone(), f);
                }
                Item::Struct(s) => {
                    if let Some(&id) = self.p.global_types.get(&s.name.name) {
                        self.structs.insert(id, s);
                    }
                }
                Item::Impl(imp) => {
                    if let TypeKind::Named { path, .. } = &imp.target.kind {
                        if let Some(&id) = self.p.global_types.get(&path[0].name) {
                            for m in &imp.methods {
                                self.methods.insert((id, m.name.name.clone()), m);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !self.fns.contains_key("main") {
            self.diags.push(
                Diagnostic::error(
                    codes::NO_MAIN,
                    "this program has no `main` function",
                    Span::new(0, 0),
                )
                .with_help("add `fn main() { ... }`; the program starts there"),
            );
            return;
        }
        self.consts();
        let module = self.module;
        for item in &module.items {
            match item {
                Item::Fn(f) => self.function(f, None),
                Item::Impl(imp) => {
                    if let TypeKind::Named { path, .. } = &imp.target.kind {
                        if let Some(&id) = self.p.global_types.get(&path[0].name) {
                            for m in &imp.methods {
                                self.function(m, Some(id));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn finish(self) -> String {
        let mut out = String::from(crate::RUNTIME);
        out.push_str("\n/* ----- generated from Kiln ----- */\n\n");
        let mut defs = String::new();
        for (id, adt) in self.p.adts.iter().enumerate() {
            if let AdtKind::Struct(fields) = &adt.kind {
                if fields.is_empty() || self.is_prim(id) {
                    continue;
                }
                let name = self.struct_name(id);
                let _ = writeln!(out, "typedef struct {name} {name};");
                let _ = writeln!(defs, "struct {name} {{");
                for f in fields {
                    let _ = writeln!(defs, "    {} f_{};", self.c_type(&f.ty), mangle(&f.name));
                }
                defs.push_str("};\n");
            }
        }
        out.push('\n');
        out.push_str(&defs);
        out.push('\n');
        out.push_str(&self.protos);
        out.push('\n');
        out.push_str(&self.globals);
        out.push('\n');
        out.push_str(&self.helpers);
        out.push_str(&self.bodies);
        out.push_str(
            "int main(void) {\n    GC_INIT();\n    k_init();\n    kf_main();\n    if (k_failed) {\n        fflush(stdout);\n        fprintf(stderr, \"error: %s\\n\", k_error_show(k_error)->data);\n        return 1;\n    }\n    return 0;\n}\n",
        );
        out
    }

    fn consts(&mut self) {
        self.f = FnState::new(Ty::Unit, None, false);
        let module = self.module;
        for item in &module.items {
            if let Item::Const(c) = item {
                let ty = self
                    .p
                    .consts
                    .get(&c.name.name)
                    .cloned()
                    .unwrap_or(Ty::Unknown);
                let name = format!("kc_{}", mangle(&c.name.name));
                let _ = writeln!(self.globals, "static {} {name};", self.c_type(&ty));
                let v = self.expr(&c.value);
                let v = self.convert(&v, &self.ty(&c.value), &ty);
                self.line(&format!("{name} = {v};"));
            }
        }
        let f = std::mem::replace(&mut self.f, FnState::new(Ty::Unit, None, false));
        self.protos.push_str("static void k_init(void);\n");
        let _ = write!(
            self.bodies,
            "static void k_init(void) {{\n{}{}}}\n\n",
            f.decls, f.body
        );
    }

    fn function(&mut self, decl: &FnDecl, owner: Option<AdtId>) {
        let sig = match owner {
            None => self.p.global_fns.get(&decl.name.name).cloned(),
            Some(id) => self.p.adts[id]
                .methods
                .iter()
                .find(|m| m.name == decl.name.name)
                .cloned(),
        };
        let Some(sig) = sig else { return };
        let mut captured = HashSet::new();
        closure_names_block(&decl.body, false, &mut captured);
        self.captured = captured;
        self.f = FnState::new(sig.ret.clone(), sig.throws.clone(), false);
        let mut params = Vec::new();
        for p in &decl.params {
            let ty = self
                .p
                .binding_types
                .get(&p.name.span)
                .cloned()
                .unwrap_or(Ty::Unknown);
            let c = format!("v_{}_{}", mangle(&p.name.name), self.next());
            params.push(format!("{} {c}", self.c_type(&ty)));
            self.bind(&p.name.name, Local { c, ty, cell: None });
        }
        if self.is_void(&sig.ret) {
            self.block(&decl.body, false);
        } else {
            let v = self.block(&decl.body, true);
            let bt = self.block_ty(&decl.body);
            if !matches!(bt, Ty::Never) {
                let v = self.convert(&v, &bt, &sig.ret);
                self.line(&format!("return {v};"));
            }
        }
        let name = match owner {
            None => format!("kf_{}", mangle(&decl.name.name)),
            Some(id) => self.method_name(id, &decl.name.name),
        };
        let ret = if self.is_void(&sig.ret) {
            "void".to_string()
        } else {
            self.c_type(&sig.ret)
        };
        let params = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };
        let header = format!("static {ret} {name}({params})");
        let f = std::mem::replace(&mut self.f, FnState::new(Ty::Unit, None, false));
        let _ = writeln!(self.protos, "{header};");
        let _ = write!(self.bodies, "{header} {{\n{}{}}}\n\n", f.decls, f.body);
    }

    // ----- types -----

    fn is_prim(&self, id: AdtId) -> bool {
        id < self.p.std_adts && self.p.global_types.get(&self.p.adts[id].name) == Some(&id) && {
            let AdtKind::Struct(fields) = &self.p.adts[id].kind else {
                return false;
            };
            fields.is_empty()
        }
    }

    fn struct_name(&self, id: AdtId) -> String {
        format!("ks_{id}_{}", mangle(&self.p.adts[id].name))
    }

    fn method_name(&self, id: AdtId, name: &str) -> String {
        format!("km_{id}_{}", mangle(name))
    }

    fn is_adt(&self, ty: &Ty, id: AdtId) -> bool {
        matches!(ty, Ty::Adt(i, _) if *i == id)
    }

    fn is_string(&self, ty: &Ty) -> bool {
        self.is_adt(ty, self.prims.string)
    }

    fn is_error(&self, ty: &Ty) -> bool {
        self.is_adt(ty, self.prims.error)
    }

    fn is_void(&self, ty: &Ty) -> bool {
        matches!(ty, Ty::Unit | Ty::Never)
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
            Ty::Param(_) | Ty::Var(_) | Ty::Any | Ty::Rest | Ty::Unknown => Rep::Kv,
            Ty::Unit | Ty::Never => Rep::Void,
        }
    }

    fn c_type(&self, ty: &Ty) -> String {
        match self.rep(ty) {
            Rep::Int => "int64_t".into(),
            Rep::Float => "double".into(),
            Rep::Bool => "bool".into(),
            Rep::Kv => "kv".into(),
            // Unit values are never stored; this keeps declarations valid.
            Rep::Void => "int64_t".into(),
            Rep::Ptr => match ty {
                Ty::Opt(_) => "kbox *".into(),
                Ty::Fn(_) => "kclosure *".into(),
                Ty::Adt(id, _) if *id == self.prims.string => "kstr *".into(),
                Ty::Adt(id, _) if *id == self.prims.list => "klist *".into(),
                Ty::Adt(id, _) => match &self.p.adts[*id].kind {
                    AdtKind::Enum(_) => "kenum *".into(),
                    AdtKind::Struct(fields) if fields.is_empty() => "void *".into(),
                    AdtKind::Struct(_) => format!("{} *", self.struct_name(*id)),
                },
                _ => "void *".into(),
            },
        }
    }

    /// A value of type `ty` stored in a `kv` slot.
    fn to_kv(&self, e: &str, ty: &Ty) -> String {
        match self.rep(ty) {
            Rep::Int | Rep::Bool => format!("(kv)({e})"),
            Rep::Float => format!("k_f2kv({e})"),
            Rep::Ptr => format!("K_P2KV({e})"),
            Rep::Kv => e.to_string(),
            Rep::Void => "0".into(),
        }
    }

    /// A `kv` slot read back as a value of type `ty`.
    fn unkv(&self, e: &str, ty: &Ty) -> String {
        match self.rep(ty) {
            Rep::Int => format!("(int64_t)({e})"),
            Rep::Bool => format!("(({e}) != 0)"),
            Rep::Float => format!("k_kv2f({e})"),
            Rep::Ptr => format!("K_KV2P({}, {e})", self.c_type(ty)),
            Rep::Kv => e.to_string(),
            Rep::Void => "0".into(),
        }
    }

    /// Converts a value of type `from` to the representation of `to`: into
    /// or out of a `kv` slot, or into an optional.
    fn convert(&self, e: &str, from: &Ty, to: &Ty) -> String {
        if matches!(from, Ty::Never | Ty::Unknown)
            || matches!(to, Ty::Never | Ty::Unknown | Ty::Unit)
        {
            return e.to_string();
        }
        let (fr, tr) = (self.rep(from), self.rep(to));
        if tr == Rep::Kv {
            return if fr == Rep::Kv {
                e.to_string()
            } else {
                self.to_kv(e, from)
            };
        }
        if fr == Rep::Kv {
            return self.unkv(e, to);
        }
        if matches!(to, Ty::Opt(_)) && !matches!(from, Ty::Opt(_)) {
            return format!("k_box({})", self.to_kv(e, from));
        }
        e.to_string()
    }

    fn ty(&self, e: &Expr) -> Ty {
        self.p
            .expr_types
            .get(&e.span)
            .cloned()
            .unwrap_or(Ty::Unknown)
    }

    fn block_ty(&self, b: &Block) -> Ty {
        match b.stmts.last() {
            Some(Stmt::Expr(e)) => self.ty(e),
            Some(Stmt::Return { .. } | Stmt::Break(_) | Stmt::Continue(_)) => Ty::Never,
            _ => Ty::Unit,
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

    // ----- output -----

    fn next(&mut self) -> usize {
        self.counter += 1;
        self.counter
    }

    fn line(&mut self, s: &str) {
        for _ in 0..self.f.depth {
            self.f.body.push_str("    ");
        }
        self.f.body.push_str(s);
        self.f.body.push('\n');
    }

    fn open(&mut self, s: &str) {
        self.line(&format!("{s} {{"));
        self.f.depth += 1;
    }

    fn close(&mut self) {
        self.f.depth -= 1;
        self.line("}");
    }

    fn reopen(&mut self, s: &str) {
        self.f.depth -= 1;
        self.line(&format!("}} {s} {{"));
        self.f.depth += 1;
    }

    fn decl(&mut self, cty: &str, name: &str) {
        let _ = writeln!(self.f.decls, "    {cty} {name} = 0;");
    }

    fn tmp(&mut self, ty: &Ty) -> String {
        let cty = self.c_type(ty);
        self.tmp_c(&cty)
    }

    fn tmp_c(&mut self, cty: &str) -> String {
        let name = format!("t{}", self.next());
        self.decl(cty, &name);
        name
    }

    fn new_local(&mut self, name: &str, ty: &Ty) -> String {
        let c = format!("v_{}_{}", mangle(name), self.next());
        let cty = self.c_type(ty);
        self.decl(&cty, &c);
        c
    }

    /// Generates code into a separate buffer, one level deeper, and returns
    /// it with the value.
    fn capture(&mut self, f: impl FnOnce(&mut Self) -> String) -> (String, String) {
        let saved = std::mem::take(&mut self.f.body);
        self.f.depth += 1;
        let v = f(self);
        self.f.depth -= 1;
        let code = std::mem::replace(&mut self.f.body, saved);
        (code, v)
    }

    fn push_scope(&mut self) {
        self.f.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.f.scopes.pop();
    }

    fn bind(&mut self, name: &str, local: Local) {
        self.f
            .scopes
            .last_mut()
            .unwrap()
            .insert(name.to_string(), local);
    }

    fn lookup(&self, name: &str) -> Option<Local> {
        self.f
            .scopes
            .iter()
            .rev()
            .find_map(|s| s.get(name))
            .cloned()
    }

    fn unsupported(&mut self, span: Span, what: impl Into<String>) -> String {
        self.diags.push(
            Diagnostic::error(
                codes::NOT_COMPILED_YET,
                format!("{} is not supported by the compiler yet", what.into()),
                span,
            )
            .with_help("`kiln check` accepts this; building it needs a later version of Kiln"),
        );
        "0".into()
    }

    /// The statement that leaves the current function after an error.
    fn fail_return(&mut self) {
        if self.f.closure || !self.is_void(&self.f.ret.clone()) {
            self.line("return 0;");
        } else {
            self.line("return;");
        }
    }

    // ----- statements -----

    fn block(&mut self, b: &Block, want: bool) -> String {
        self.push_scope();
        let mut value = "0".to_string();
        let last = b.stmts.len().saturating_sub(1);
        for (i, stmt) in b.stmts.iter().enumerate() {
            match stmt {
                Stmt::Expr(e) if i == last && want => value = self.expr(e),
                _ => self.stmt(stmt),
            }
        }
        self.pop_scope();
        value
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let {
                mutable,
                name,
                value,
                ..
            } => {
                let vt = self.ty(value);
                let ty = self
                    .p
                    .binding_types
                    .get(&name.span)
                    .cloned()
                    .unwrap_or(vt.clone());
                let v = self.expr(value);
                let v = self.convert(&v, &vt, &ty);
                if *mutable && self.captured.contains(&name.name) {
                    let cty = self.c_type(&ty);
                    let cell = self.tmp_c(&format!("{cty} *"));
                    self.line(&format!("{cell} = k_alloc(sizeof({cty}));"));
                    self.line(&format!("*{cell} = {v};"));
                    let local = Local {
                        c: format!("(*{cell})"),
                        ty,
                        cell: Some(cell),
                    };
                    self.bind(&name.name, local);
                } else {
                    let c = self.new_local(&name.name, &ty);
                    self.line(&format!("{c} = {v};"));
                    self.bind(&name.name, Local { c, ty, cell: None });
                }
            }
            Stmt::Assign {
                target, op, value, ..
            } => self.assign(target, *op, value),
            Stmt::Return { value, .. } => {
                let ret = self.f.ret.clone();
                let v = match value {
                    Some(v) => {
                        let x = self.expr(v);
                        self.convert(&x, &self.ty(v), &ret)
                    }
                    None => "0".into(),
                };
                if self.f.closure {
                    let v = self.to_kv(&v, &ret);
                    self.line(&format!("return {v};"));
                } else if self.is_void(&ret) {
                    self.line("return;");
                } else {
                    self.line(&format!("return {v};"));
                }
            }
            Stmt::Break(_) => self.line("break;"),
            Stmt::Continue(_) => self.line("continue;"),
            Stmt::For {
                binding,
                iter,
                filter,
                body,
                ..
            } => self.for_loop(binding, iter, filter.as_ref(), body),
            Stmt::While { cond, body, .. } => {
                let (code, c) = self.capture(|g| g.expr(cond));
                if code.is_empty() {
                    self.open(&format!("while ({c})"));
                } else {
                    self.open("while (1)");
                    self.f.body.push_str(&code);
                    self.line(&format!("if (!({c})) break;"));
                }
                self.block(body, false);
                self.close();
            }
            Stmt::Expr(e) => self.discard(e),
        }
    }

    fn for_loop(&mut self, binding: &Ident, iter: &Expr, filter: Option<&Expr>, body: &Block) {
        let (item, elem) = match &iter.kind {
            ExprKind::Binary {
                op: BinaryOp::Range,
                lhs,
                rhs,
            } => {
                let lo = self.expr(lhs);
                let hi = self.expr(rhs);
                let int = Ty::Adt(self.prims.int, Vec::new());
                let h = self.tmp(&int);
                self.line(&format!("{h} = {hi};"));
                let i = self.tmp(&int);
                self.open(&format!("for ({i} = {lo}; {i} < {h}; {i}++)"));
                (i, int)
            }
            _ => {
                let it = self.ty(iter);
                let elem = match &it {
                    Ty::Adt(id, args) if *id == self.prims.list => args[0].clone(),
                    _ => {
                        self.unsupported(iter.span, "looping over this type");
                        return;
                    }
                };
                let l = self.expr(iter);
                let lt = self.tmp_c("klist *");
                self.line(&format!("{lt} = {l};"));
                let i = self.tmp_c("int64_t");
                self.open(&format!("for ({i} = 0; {i} < {lt}->len; {i}++)"));
                (self.unkv(&format!("{lt}->items[{i}]"), &elem), elem)
            }
        };
        self.push_scope();
        let ty = self
            .p
            .binding_types
            .get(&binding.span)
            .cloned()
            .unwrap_or(elem.clone());
        let c = self.new_local(&binding.name, &ty);
        let item = self.convert(&item, &elem, &ty);
        self.line(&format!("{c} = {item};"));
        self.bind(&binding.name, Local { c, ty, cell: None });
        if let Some(f) = filter {
            let fv = self.expr(f);
            self.line(&format!("if (!({fv})) continue;"));
        }
        self.block(body, false);
        self.pop_scope();
        self.close();
    }

    fn assign(&mut self, target: &Expr, op: AssignOp, value: &Expr) {
        let tt = self.ty(target);
        match &target.kind {
            ExprKind::Ident(name) => {
                let Some(local) = self.lookup(name) else {
                    self.unsupported(target.span, "assigning to this name");
                    return;
                };
                let v = self.expr(value);
                let v = self.convert(&v, &self.ty(value), &local.ty);
                let new = self.compound(op, &local.c, &local.ty, &v);
                self.line(&format!("{} = {new};", local.c));
            }
            ExprKind::Field { expr: base, name } => {
                let bt = self.ty(base);
                let Some(decl) = self.field_decl(&bt, &name.name) else {
                    self.unsupported(target.span, "assigning to this field");
                    return;
                };
                let b = self.expr(base);
                let raw = format!("({b})->f_{}", mangle(&name.name));
                let v = self.expr(value);
                let v = self.convert(&v, &self.ty(value), &tt);
                let cur = self.convert(&raw, &decl, &tt);
                let new = self.compound(op, &cur, &tt, &v);
                let new = self.convert(&new, &tt, &decl);
                self.line(&format!("{raw} = {new};"));
            }
            ExprKind::Index { expr: base, index } => {
                let bt = self.ty(base);
                if !self.is_adt(&bt, self.prims.list) {
                    self.unsupported(target.span, "assigning into maps");
                    return;
                }
                let b = self.expr(base);
                let i = self.expr(index);
                let v = self.expr(value);
                let v = self.convert(&v, &self.ty(value), &tt);
                let cur = self.unkv(&format!("k_list_at({b}, {i})"), &tt);
                let new = self.compound(op, &cur, &tt, &v);
                let new = self.to_kv(&new, &tt);
                self.line(&format!("k_list_set({b}, {i}, {new});"));
            }
            _ => {
                self.unsupported(target.span, "assigning to this expression");
            }
        }
    }

    fn compound(&self, op: AssignOp, cur: &str, ty: &Ty, v: &str) -> String {
        let bop = match op {
            AssignOp::Set => return v.to_string(),
            AssignOp::Add => BinaryOp::Add,
            AssignOp::Sub => BinaryOp::Sub,
            AssignOp::Mul => BinaryOp::Mul,
            AssignOp::Div => BinaryOp::Div,
        };
        self.arith(bop, cur, v, ty)
    }

    fn arith(&self, op: BinaryOp, l: &str, r: &str, ty: &Ty) -> String {
        if self.is_string(ty) {
            return format!("k_concat({l}, {r})");
        }
        if self.rep(ty) == Rep::Float {
            return match op {
                BinaryOp::Add => format!("(({l}) + ({r}))"),
                BinaryOp::Sub => format!("(({l}) - ({r}))"),
                BinaryOp::Mul => format!("(({l}) * ({r}))"),
                BinaryOp::Div => format!("(({l}) / ({r}))"),
                _ => format!("fmod({l}, {r})"),
            };
        }
        let f = match op {
            BinaryOp::Add => "k_add",
            BinaryOp::Sub => "k_sub",
            BinaryOp::Mul => "k_mul",
            BinaryOp::Div => "k_div",
            _ => "k_rem",
        };
        format!("{f}({l}, {r})")
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
                self.if_chain(cond, then, otherwise.as_deref(), None, &Ty::Unit);
            }
            ExprKind::Match { scrutinee, arms } => {
                self.match_expr(e, scrutinee, arms, false);
            }
            ExprKind::Block(b) => {
                self.block(b, false);
            }
            ExprKind::Catch {
                expr,
                binding,
                handler,
            } => {
                self.catch(e, expr, binding.as_ref(), handler, false);
            }
            _ => {
                self.expr(e);
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(s) => {
                let digits: String = s.chars().filter(|c| *c != '_').collect();
                match digits.parse::<i64>() {
                    Ok(n) => format!("INT64_C({n})"),
                    Err(_) => self.unsupported(e.span, "an integer this large"),
                }
            }
            ExprKind::Float(s) => {
                let mut digits: String = s.chars().filter(|c| *c != '_').collect();
                if !digits.contains(['.', 'e', 'E']) {
                    digits.push_str(".0");
                }
                digits
            }
            ExprKind::Duration(s) => {
                let split = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
                let n: i64 = s[..split].replace('_', "").parse().unwrap_or(0);
                let unit: i64 = match &s[split..] {
                    "ms" => 1_000_000,
                    "s" => 1_000_000_000,
                    "m" => 60_000_000_000,
                    "h" => 3_600_000_000_000,
                    _ => 1,
                };
                match n.checked_mul(unit) {
                    Some(ns) => format!("INT64_C({ns})"),
                    None => self.unsupported(e.span, "a duration this long"),
                }
            }
            ExprKind::Str(pieces) => {
                let mut parts = Vec::new();
                for piece in pieces {
                    match piece {
                        StrPiece::Text(t) => parts.push(c_string(t)),
                        StrPiece::Expr(inner) => {
                            let v = self.expr(inner);
                            let shown = self.display(&v, &self.ty(inner), inner.span);
                            parts.push(shown);
                        }
                    }
                }
                let mut parts = parts.into_iter();
                let first = parts.next().unwrap_or_else(|| c_string(""));
                parts.fold(first, |acc, p| format!("k_concat({acc}, {p})"))
            }
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::None => "NULL".into(),
            ExprKind::Ident(name) => self.ident(name, e.span),
            ExprKind::List(items) => {
                let elem = match self.ty(e) {
                    Ty::Adt(_, args) if !args.is_empty() => args[0].clone(),
                    _ => Ty::Unknown,
                };
                let mut vals = Vec::new();
                for item in items {
                    let v = self.expr(item);
                    let v = self.convert(&v, &self.ty(item), &elem);
                    vals.push(self.to_kv(&v, &elem));
                }
                let t = self.tmp_c("klist *");
                if vals.is_empty() {
                    self.line(&format!("{t} = k_list(0, NULL);"));
                } else {
                    self.line(&format!(
                        "{t} = k_list({}, (kv[]){{{}}});",
                        vals.len(),
                        vals.join(", ")
                    ));
                }
                t
            }
            ExprKind::Record(_) => self.unsupported(e.span, "an anonymous record"),
            ExprKind::Unary { op, expr } => {
                let v = self.expr(expr);
                match op {
                    UnaryOp::Not => format!("(!({v}))"),
                    UnaryOp::Neg if self.rep(&self.ty(e)) == Rep::Float => format!("(-({v}))"),
                    UnaryOp::Neg => format!("k_neg({v})"),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, lhs, rhs),
            ExprKind::Call { .. } => {
                let rt = self.ty(e);
                let (call, _) = self.call(e, &rt);
                self.store(&call, &rt)
            }
            ExprKind::Field { expr: base, name } => self.field(e, base, name),
            ExprKind::Index { expr: base, index } => {
                let bt = self.ty(base);
                match &bt {
                    Ty::Adt(id, args) if *id == self.prims.list => {
                        let elem = args[0].clone();
                        let b = self.expr(base);
                        let i = self.expr(index);
                        self.unkv(&format!("k_list_at({b}, {i})"), &elem)
                    }
                    _ => self.unsupported(e.span, "indexing into a map"),
                }
            }
            ExprKind::FieldShorthand(name) => self.shorthand(e, name),
            ExprKind::Closure { params, body, .. } => self.closure(e, params, body),
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                let rt = self.ty(e);
                let r = (!self.is_void(&rt)).then(|| self.tmp(&rt));
                self.if_chain(cond, then, otherwise.as_deref(), r.as_deref(), &rt);
                r.unwrap_or_else(|| "0".into())
            }
            ExprKind::Block(b) => self.block(b, true),
            ExprKind::Match { scrutinee, arms } => self.match_expr(e, scrutinee, arms, true),
            ExprKind::Try(inner) => {
                let rt = self.ty(e);
                let (call, throws) = self.call(inner, &rt);
                let v = self.store(&call, &rt);
                if let Some(err) = throws {
                    self.propagate(&err);
                }
                v
            }
            ExprKind::Catch {
                expr,
                binding,
                handler,
            } => self.catch(e, expr, binding.as_ref(), handler, true),
            ExprKind::Throw(value) => {
                self.throw(value);
                "0".into()
            }
            ExprKind::Is { expr, pattern } => {
                let vt = self.ty(expr);
                let v = self.expr(expr);
                let s = self.tmp(&vt);
                self.line(&format!("{s} = {v};"));
                let mut conds = Vec::new();
                let mut binds = Vec::new();
                self.pattern(pattern, &s, &vt, &mut conds, &mut binds);
                let mut assigns = Vec::new();
                for (name, ty, val) in binds {
                    let c = self.new_local(&name, &ty);
                    assigns.push(format!("{c} = {val}"));
                    self.bind(&name, Local { c, ty, cell: None });
                }
                if !assigns.is_empty() {
                    conds.push(format!("({}, 1)", assigns.join(", ")));
                }
                if conds.is_empty() {
                    "1".into()
                } else {
                    format!("({})", conds.join(" && "))
                }
            }
            ExprKind::Error => "0".into(),
        }
    }

    /// Evaluates a call now, returning a temporary with its value.
    fn store(&mut self, call: &str, ty: &Ty) -> String {
        if self.is_void(ty) {
            self.line(&format!("{call};"));
            return "0".into();
        }
        let t = self.tmp(ty);
        self.line(&format!("{t} = {call};"));
        t
    }

    fn ident(&mut self, name: &str, span: Span) -> String {
        if let Some(local) = self.lookup(name) {
            return local.c;
        }
        if self.fns.contains_key(name) {
            return self.fn_value(name, span);
        }
        if self.p.consts.contains_key(name) {
            return format!("kc_{}", mangle(name));
        }
        self.unsupported(span, format!("using `{name}` as a value"))
    }

    /// A top-level function used as a value, wrapped as a closure.
    fn fn_value(&mut self, name: &str, span: Span) -> String {
        let key = format!("fn:{name}");
        let wrapper = format!("kw_{}", mangle(name));
        if let std::collections::hash_map::Entry::Vacant(slot) = self.helper_names.entry(key) {
            slot.insert(wrapper.clone());
            let Some(sig) = self.p.global_fns.get(name).cloned() else {
                return self.unsupported(span, format!("using `{name}` as a value"));
            };
            let args: Vec<String> = sig
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| self.unkv(&format!("args[{i}]"), &p.ty))
                .collect();
            let call = format!("kf_{}({})", mangle(name), args.join(", "));
            let body = if self.is_void(&sig.ret) {
                format!("    {call};\n    return 0;\n")
            } else {
                format!("    return {};\n", self.to_kv(&call, &sig.ret))
            };
            let header = format!("static kv {wrapper}(void *env, kv *args)");
            let _ = writeln!(self.protos, "{header};");
            let _ = write!(
                self.helpers,
                "{header} {{\n    (void)env; (void)args;\n{body}}}\n\n"
            );
        }
        format!("k_closure({wrapper}, NULL)")
    }

    fn field_decl(&self, base: &Ty, name: &str) -> Option<Ty> {
        let Ty::Adt(id, _) = base else { return None };
        let AdtKind::Struct(fields) = &self.p.adts[*id].kind else {
            return None;
        };
        fields.iter().find(|f| f.name == name).map(|f| f.ty.clone())
    }

    fn field(&mut self, e: &Expr, base: &Expr, name: &Ident) -> String {
        if let ExprKind::Ident(n) = &base.kind {
            if self.lookup(n).is_none() {
                if let Some(module) = self.p.imports.get(n) {
                    let what = format!("`{module}.{}`", name.name);
                    return self.unsupported(e.span, what);
                }
                if let Some(&id) = self.p.global_types.get(n) {
                    if let AdtKind::Enum(variants) = &self.p.adts[id].kind {
                        let idx = variants
                            .iter()
                            .position(|v| v.name == name.name)
                            .unwrap_or(0);
                        return format!("k_enum({idx}, 0, NULL)");
                    }
                }
            }
        }
        let bt = self.ty(base);
        let Some(decl) = self.field_decl(&bt, &name.name) else {
            return self.unsupported(e.span, "this field access");
        };
        let b = self.expr(base);
        let raw = format!("({b})->f_{}", mangle(&name.name));
        self.convert(&raw, &decl, &self.ty(e))
    }

    fn binary(&mut self, e: &Expr, op: BinaryOp, lhs: &Expr, rhs: &Expr) -> String {
        match op {
            BinaryOp::And | BinaryOp::Or => {
                let l = self.expr(lhs);
                let (code, r) = self.capture(|g| g.expr(rhs));
                let c_op = if op == BinaryOp::And { "&&" } else { "||" };
                if code.is_empty() {
                    return format!("(({l}) {c_op} ({r}))");
                }
                let t = self.tmp_c("bool");
                self.line(&format!("{t} = {l};"));
                self.open(&if op == BinaryOp::And {
                    format!("if ({t})")
                } else {
                    format!("if (!{t})")
                });
                self.f.body.push_str(&code);
                self.line(&format!("{t} = {r};"));
                self.close();
                t
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let (lt, rt) = (self.ty(lhs), self.ty(rhs));
                let l = self.expr(lhs);
                let r = self.expr(rhs);
                let eq = if matches!(rhs.kind, ExprKind::None) {
                    format!("(({l}) == NULL)")
                } else if matches!(lhs.kind, ExprKind::None) {
                    format!("(({r}) == NULL)")
                } else {
                    self.eq_expr(&l, &lt, &r, &rt, e.span)
                };
                if op == BinaryOp::Eq {
                    eq
                } else {
                    format!("(!{eq})")
                }
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let lt = self.ty(lhs);
                let l = self.expr(lhs);
                let r = self.expr(rhs);
                let c_op = match op {
                    BinaryOp::Lt => "<",
                    BinaryOp::Le => "<=",
                    BinaryOp::Gt => ">",
                    _ => ">=",
                };
                if self.is_string(&lt) {
                    format!("(k_str_cmp({l}, {r}) {c_op} 0)")
                } else {
                    format!("(({l}) {c_op} ({r}))")
                }
            }
            BinaryOp::Coalesce => {
                let rt = self.ty(e);
                let inner = match self.ty(lhs) {
                    Ty::Opt(inner) => *inner,
                    _ => Ty::Unknown,
                };
                let l = self.expr(lhs);
                let t = self.tmp_c("kbox *");
                self.line(&format!("{t} = {l};"));
                let r = (!self.is_void(&rt)).then(|| self.tmp(&rt));
                let (code, rv) = self.capture(|g| g.expr(rhs));
                self.open(&format!("if ({t})"));
                if let Some(r) = &r {
                    let v = self.unkv(&format!("{t}->v"), &inner);
                    let v = self.convert(&v, &inner, &rt);
                    self.line(&format!("{r} = {v};"));
                }
                self.reopen("else");
                self.f.body.push_str(&code);
                let rhs_ty = self.ty(rhs);
                if let (Some(r), false) = (&r, matches!(rhs_ty, Ty::Never)) {
                    let v = self.convert(&rv, &rhs_ty, &rt);
                    self.line(&format!("{r} = {v};"));
                }
                self.close();
                r.unwrap_or_else(|| "0".into())
            }
            BinaryOp::Range => {
                let l = self.expr(lhs);
                let r = self.expr(rhs);
                format!("k_range({l}, {r})")
            }
            _ => {
                let ty = self.ty(e);
                let l = self.expr(lhs);
                let r = self.expr(rhs);
                self.arith(op, &l, &r, &ty)
            }
        }
    }

    fn if_chain(
        &mut self,
        cond: &Expr,
        then: &Block,
        otherwise: Option<&Expr>,
        r: Option<&str>,
        rt: &Ty,
    ) {
        self.push_scope();
        let c = self.expr(cond);
        self.open(&format!("if ({c})"));
        let v = self.block(then, r.is_some());
        if let Some(r) = r {
            let bt = self.block_ty(then);
            self.assign_branch(r, &v, &bt, rt);
        }
        self.pop_scope();
        if let Some(other) = otherwise {
            self.reopen("else");
            match &other.kind {
                ExprKind::If {
                    cond,
                    then,
                    otherwise,
                } => self.if_chain(cond, then, otherwise.as_deref(), r, rt),
                ExprKind::Block(b) => {
                    let v = self.block(b, r.is_some());
                    if let Some(r) = r {
                        let bt = self.block_ty(b);
                        self.assign_branch(r, &v, &bt, rt);
                    }
                }
                _ => {
                    let v = self.expr(other);
                    if let Some(r) = r {
                        let ot = self.ty(other);
                        self.assign_branch(r, &v, &ot, rt);
                    }
                }
            }
        }
        self.close();
    }

    fn assign_branch(&mut self, r: &str, v: &str, vt: &Ty, rt: &Ty) {
        if self.is_void(vt) {
            return;
        }
        let v = self.convert(v, vt, rt);
        self.line(&format!("{r} = {v};"));
    }

    fn match_expr(&mut self, e: &Expr, scrutinee: &Expr, arms: &[MatchArm], want: bool) -> String {
        let rt = self.ty(e);
        let st = self.ty(scrutinee);
        let sv = self.expr(scrutinee);
        let s = self.tmp(&st);
        self.line(&format!("{s} = {sv};"));
        let m = self.tmp_c("bool");
        let r = (want && !self.is_void(&rt)).then(|| self.tmp(&rt));
        for arm in arms {
            self.push_scope();
            let mut conds = Vec::new();
            let mut binds = Vec::new();
            self.pattern(&arm.pattern, &s, &st, &mut conds, &mut binds);
            let cond = if conds.is_empty() {
                format!("!{m}")
            } else {
                format!("!{m} && {}", conds.join(" && "))
            };
            self.open(&format!("if ({cond})"));
            for (name, ty, val) in binds {
                let c = self.new_local(&name, &ty);
                self.line(&format!("{c} = {val};"));
                self.bind(&name, Local { c, ty, cell: None });
            }
            if let Some(guard) = &arm.guard {
                let g = self.expr(guard);
                self.open(&format!("if ({g})"));
            }
            self.line(&format!("{m} = true;"));
            match &r {
                Some(r) => {
                    let v = self.expr(&arm.body);
                    let bt = self.ty(&arm.body);
                    self.assign_branch(r, &v, &bt, &rt);
                }
                None => self.discard(&arm.body),
            }
            if arm.guard.is_some() {
                self.close();
            }
            self.close();
            self.pop_scope();
        }
        self.line(&format!(
            "if (!{m}) k_panic_c(\"no arm of this match matched\");"
        ));
        r.unwrap_or_else(|| "0".into())
    }

    /// Compiles a pattern against the value `val` of type `ty` into
    /// conditions, checked in order, and the names it binds.
    fn pattern(
        &mut self,
        p: &Pattern,
        val: &str,
        ty: &Ty,
        conds: &mut Vec<String>,
        binds: &mut Vec<(String, Ty, String)>,
    ) {
        match &p.kind {
            PatternKind::Wildcard | PatternKind::Error => {}
            PatternKind::Binding(name) => binds.push((name.clone(), ty.clone(), val.to_string())),
            PatternKind::Literal(lit) => {
                let l = self.expr(lit);
                let lt = self.ty(lit);
                let eq = self.eq_expr(val, ty, &l, &lt, p.span);
                conds.push(eq);
            }
            PatternKind::None => conds.push(format!("{val} == NULL")),
            PatternKind::Variant { path, args } => {
                let last = &path[path.len() - 1].name;
                if path.len() == 1 && last == "some" {
                    if let Ty::Opt(inner) = ty {
                        conds.push(format!("{val} != NULL"));
                        let v = self.unkv(&format!("({val})->v"), inner);
                        if let Some(a) = args.first() {
                            self.pattern(a, &v, inner, conds, binds);
                        }
                    }
                    return;
                }
                let Ty::Adt(id, targs) = ty else { return };
                let AdtKind::Enum(variants) = &self.p.adts[*id].kind else {
                    return;
                };
                let Some(idx) = variants.iter().position(|v| v.name == *last) else {
                    return;
                };
                conds.push(format!("({val})->tag == {idx}"));
                let map = self.type_map(*id, targs);
                let fields: Vec<Ty> = variants[idx]
                    .fields
                    .iter()
                    .map(|(_, t)| substitute(t, &map))
                    .collect();
                for (i, (a, fty)) in args.iter().zip(&fields).enumerate() {
                    let v = self.unkv(&format!("({val})->f[{i}]"), fty);
                    self.pattern(a, &v, fty, conds, binds);
                }
            }
        }
    }

    fn closure(&mut self, e: &Expr, params: &[Param], body: &Block) -> String {
        let Ty::Fn(fty) = self.ty(e) else {
            return self.unsupported(e.span, "this function value");
        };
        let mut names = BTreeSet::new();
        names_in_block(body, &mut names);
        let caps: Vec<(String, Local)> = names
            .into_iter()
            .filter_map(|n| self.lookup(&n).map(|l| (n, l)))
            .collect();
        let name = format!("kl_{}", self.next());
        let saved = std::mem::replace(
            &mut self.f,
            FnState::new(fty.ret.clone(), fty.throws.clone(), true),
        );
        for (i, (n, local)) in caps.iter().enumerate() {
            match &local.cell {
                Some(_) => {
                    let cty = self.c_type(&local.ty);
                    let cell = self.tmp_c(&format!("{cty} *"));
                    self.line(&format!("{cell} = K_KV2P({cty} *, env[{i}]);"));
                    let l = Local {
                        c: format!("(*{cell})"),
                        ty: local.ty.clone(),
                        cell: Some(cell),
                    };
                    self.bind(n, l);
                }
                None => {
                    let c = self.new_local(n, &local.ty);
                    let v = self.unkv(&format!("env[{i}]"), &local.ty);
                    self.line(&format!("{c} = {v};"));
                    let l = Local {
                        c,
                        ty: local.ty.clone(),
                        cell: None,
                    };
                    self.bind(n, l);
                }
            }
        }
        for (i, p) in params.iter().enumerate() {
            let ty = self
                .p
                .binding_types
                .get(&p.name.span)
                .cloned()
                .or_else(|| fty.params.get(i).cloned())
                .unwrap_or(Ty::Unknown);
            let c = self.new_local(&p.name.name, &ty);
            let v = self.unkv(&format!("args[{i}]"), &ty);
            self.line(&format!("{c} = {v};"));
            self.bind(&p.name.name, Local { c, ty, cell: None });
        }
        if self.is_void(&fty.ret) {
            self.block(body, false);
        } else {
            let v = self.block(body, true);
            let bt = self.block_ty(body);
            if !matches!(bt, Ty::Never) {
                let v = self.convert(&v, &bt, &fty.ret);
                let v = self.to_kv(&v, &fty.ret);
                self.line(&format!("return {v};"));
            }
        }
        self.finish_closure(&name, saved);
        if caps.is_empty() {
            return format!("k_closure({name}, NULL)");
        }
        let env = self.tmp_c("kv *");
        self.line(&format!("{env} = k_alloc(sizeof(kv) * {});", caps.len()));
        for (i, (_, local)) in caps.iter().enumerate() {
            let v = match &local.cell {
                Some(cell) => format!("K_P2KV({cell})"),
                None => self.to_kv(&local.c, &local.ty),
            };
            self.line(&format!("{env}[{i}] = {v};"));
        }
        format!("k_closure({name}, {env})")
    }

    fn finish_closure(&mut self, name: &str, saved: FnState) {
        let f = std::mem::replace(&mut self.f, saved);
        let header = format!("static kv {name}(void *envp, kv *args)");
        let _ = writeln!(self.protos, "{header};");
        let _ = write!(
            self.bodies,
            "{header} {{\n    kv *env = envp;\n    (void)env; (void)args;\n{}{}    return 0;\n}}\n\n",
            f.decls, f.body
        );
    }

    /// `.title` or `.upper`, as a closure.
    fn shorthand(&mut self, e: &Expr, name: &Ident) -> String {
        let Ty::Fn(fty) = self.ty(e) else {
            return self.unsupported(e.span, "this field shorthand");
        };
        let (Some(param), ret) = (fty.params.first().cloned(), fty.ret.clone()) else {
            return self.unsupported(e.span, "this field shorthand");
        };
        let fname = format!("kl_{}", self.next());
        let saved = std::mem::replace(&mut self.f, FnState::new(ret.clone(), None, true));
        let x = self.new_local("it", &param);
        let v = self.unkv("args[0]", &param);
        self.line(&format!("{x} = {v};"));
        let value = match self.field_decl(&param, &name.name) {
            Some(decl) => {
                let raw = format!("({x})->f_{}", mangle(&name.name));
                self.convert(&raw, &decl, &ret)
            }
            None => match self.call_method(&x, &param, &name.name, &[], &ret, e.span) {
                Some((call, _)) => self.store(&call, &ret),
                None => self.unsupported(e.span, "this field shorthand"),
            },
        };
        let value = self.to_kv(&value, &ret);
        self.line(&format!("return {value};"));
        self.finish_closure(&fname, saved);
        format!("k_closure({fname}, NULL)")
    }

    // ----- errors -----

    /// After a call that failed, passes its error (of type `err`) on to
    /// the caller.
    fn propagate(&mut self, err: &Ty) {
        self.open("if (k_failed)");
        let target = self.f.throws.clone();
        if target.as_ref().is_some_and(|t| self.is_error(t)) && !self.is_error(err) {
            let w = self.new_error();
            self.line(&format!("{w}->f_message = k_error_show(k_error);"));
            let show = self.error_message_fn();
            self.line(&format!("k_error = K_P2KV({w});"));
            self.line(&format!("k_error_show = {show};"));
        }
        self.fail_return();
        self.close();
    }

    fn new_error(&mut self) -> String {
        let ty = Ty::Adt(self.prims.error, Vec::new());
        let w = self.tmp(&ty);
        let name = self.struct_name(self.prims.error);
        self.line(&format!("{w} = k_alloc(sizeof({name}));"));
        w
    }

    fn throw(&mut self, value: &Expr) {
        let mut et = self.ty(value);
        let mut v = self.expr(value);
        let target = self.f.throws.clone();
        if target.as_ref().is_some_and(|t| self.is_error(t)) && !self.is_error(&et) {
            let shown = self.display(&v, &et, value.span);
            let w = self.new_error();
            self.line(&format!("{w}->f_message = {shown};"));
            v = w;
            et = Ty::Adt(self.prims.error, Vec::new());
        }
        let show = if self.is_error(&et) {
            self.error_message_fn()
        } else {
            self.show_fn(&et, false, value.span)
        };
        let kv = self.to_kv(&v, &et);
        self.line(&format!("k_fail({kv}, {show});"));
        self.fail_return();
    }

    fn catch(
        &mut self,
        e: &Expr,
        expr: &Expr,
        binding: Option<&Ident>,
        handler: &Block,
        want: bool,
    ) -> String {
        let rt = self.ty(e);
        let (call, throws) = if matches!(expr.kind, ExprKind::Call { .. }) {
            self.call(expr, &rt)
        } else {
            (self.expr(expr), None)
        };
        let r = if self.is_void(&rt) {
            self.line(&format!("{call};"));
            None
        } else {
            let t = self.tmp(&rt);
            self.line(&format!("{t} = {call};"));
            Some(t)
        };
        let et = throws.unwrap_or(Ty::Unknown);
        self.open("if (k_failed)");
        self.push_scope();
        match binding {
            Some(b) => {
                let c = self.new_local(&b.name, &et);
                let v = self.unkv("k_take_error()", &et);
                self.line(&format!("{c} = {v};"));
                self.bind(
                    &b.name,
                    Local {
                        c,
                        ty: et,
                        cell: None,
                    },
                );
            }
            None => self.line("k_take_error();"),
        }
        let v = self.block(handler, r.is_some());
        if let Some(r) = &r {
            let bt = self.block_ty(handler);
            self.assign_branch(r, &v, &bt, &rt);
        }
        self.pop_scope();
        self.close();
        let _ = want;
        r.unwrap_or_else(|| "0".into())
    }

    // ----- calls -----

    /// Generates the arguments of a call into the parameters they fill, in
    /// source order, converted to the parameters' declared types.
    fn arrange(
        &mut self,
        params: &[ParamSig],
        defaults: &[Option<&Expr>],
        args: &[Arg],
    ) -> Vec<String> {
        let mut slots: Vec<Option<String>> = vec![None; params.len()];
        let mut next = 0;
        for arg in args {
            let index = match &arg.name {
                Some(name) => params
                    .iter()
                    .position(|p| p.name.as_deref() == Some(&name.name)),
                None => {
                    while next < params.len() && slots[next].is_some() {
                        next += 1;
                    }
                    (next < params.len()).then_some(next)
                }
            };
            let v = self.expr(&arg.value);
            if let Some(i) = index {
                let v = self.convert(&v, &self.ty(&arg.value), &params[i].ty);
                slots[i] = Some(v);
            }
        }
        let mut out = Vec::new();
        for (i, slot) in slots.into_iter().enumerate() {
            let v = match slot {
                Some(v) => v,
                None => match defaults.get(i).copied().flatten() {
                    Some(d) => {
                        let v = self.expr(d);
                        self.convert(&v, &self.ty(d), &params[i].ty)
                    }
                    None => "0".into(),
                },
            };
            out.push(v);
        }
        out
    }

    /// Generates a call, returning a C expression of type `rt` that makes
    /// the call when evaluated, and the error type if the call can fail.
    fn call(&mut self, e: &Expr, rt: &Ty) -> (String, Option<Ty>) {
        let ExprKind::Call { callee, args, .. } = &e.kind else {
            return (self.expr(e), None);
        };
        match &callee.kind {
            ExprKind::Field { expr: base, name } => {
                if let ExprKind::Ident(m) = &base.kind {
                    if self.lookup(m).is_none() {
                        if let Some(module) = self.p.imports.get(m).cloned() {
                            return self.module_call(&module, name, args, e.span);
                        }
                        if let Some(&id) = self.p.global_types.get(m) {
                            if !self.fns.contains_key(m) {
                                return self.type_call(id, name, args, rt, e.span);
                            }
                        }
                    }
                }
                if let ExprKind::Field { expr: inner, .. } = &base.kind {
                    if let ExprKind::Ident(m) = &inner.kind {
                        if self.lookup(m).is_none() && self.p.imports.contains_key(m) {
                            let what = format!("`{}.{}`", self.p.imports[m], name.name);
                            return (self.unsupported(e.span, what), None);
                        }
                    }
                }
                let bt = self.ty(base);
                let b = self.expr(base);
                if let Some(result) = self.call_method(&b, &bt, &name.name, args, rt, e.span) {
                    return result;
                }
                let Some(decl) = self.field_decl(&bt, &name.name) else {
                    return (self.unsupported(e.span, "this call"), None);
                };
                let field_ty = match &bt {
                    Ty::Adt(id, targs) => substitute(&decl, &self.type_map(*id, targs)),
                    _ => decl.clone(),
                };
                let raw = format!("({b})->f_{}", mangle(&name.name));
                let f = self.convert(&raw, &decl, &field_ty);
                self.call_closure(&f, &field_ty, args, rt, e.span)
            }
            ExprKind::Ident(n) if self.lookup(n).is_none() => {
                if let Some(decl) = self.fns.get(n).copied() {
                    let sig = self.p.global_fns[n].clone();
                    let name = format!("kf_{}", mangle(n));
                    return self.user_call(&name, &sig, Some(decl), None, args, rt);
                }
                match n.as_str() {
                    "print" | "env" | "panic" if args.len() == 1 => {
                        let a = &args[0].value;
                        let v = self.expr(a);
                        let at = self.ty(a);
                        let call = match n.as_str() {
                            "print" => format!("k_print({})", self.display(&v, &at, a.span)),
                            "env" => format!("k_env({v})"),
                            _ => format!("k_panic({v})"),
                        };
                        return (call, None);
                    }
                    _ => {}
                }
                if let Some(&id) = self.p.global_types.get(n) {
                    return (self.construct(id, args, e.span), None);
                }
                let ct = self.ty(callee);
                let c = self.expr(callee);
                self.call_closure(&c, &ct, args, rt, e.span)
            }
            _ => {
                let ct = self.ty(callee);
                let c = self.expr(callee);
                self.call_closure(&c, &ct, args, rt, e.span)
            }
        }
    }

    fn user_call(
        &mut self,
        name: &str,
        sig: &FnSig,
        decl: Option<&FnDecl>,
        recv: Option<&str>,
        args: &[Arg],
        rt: &Ty,
    ) -> (String, Option<Ty>) {
        let defaults: Vec<Option<&Expr>> = match decl {
            Some(d) => d
                .params
                .iter()
                .skip(usize::from(sig.has_self))
                .map(|p| p.default.as_ref())
                .collect(),
            None => Vec::new(),
        };
        let mut vals = self.arrange(&sig.params, &defaults, args);
        if let Some(r) = recv {
            vals.insert(0, r.to_string());
        }
        let call = format!("{name}({})", vals.join(", "));
        let call = if self.is_void(&sig.ret) {
            call
        } else {
            self.convert(&call, &sig.ret, rt)
        };
        (call, sig.throws.clone())
    }

    /// `Type.function(...)` or `Enum.Variant(...)`.
    fn type_call(
        &mut self,
        id: AdtId,
        name: &Ident,
        args: &[Arg],
        rt: &Ty,
        span: Span,
    ) -> (String, Option<Ty>) {
        if let Some(decl) = self.methods.get(&(id, name.name.clone())).copied() {
            let sig = self.p.adts[id]
                .methods
                .iter()
                .find(|m| m.name == name.name)
                .cloned();
            if let Some(sig) = sig.filter(|s| !s.has_self) {
                let cname = self.method_name(id, &name.name);
                return self.user_call(&cname, &sig, Some(decl), None, args, rt);
            }
        }
        let AdtKind::Enum(variants) = &self.p.adts[id].kind else {
            return (self.unsupported(span, "this call"), None);
        };
        let Some(idx) = variants.iter().position(|v| v.name == name.name) else {
            return (self.unsupported(span, "this call"), None);
        };
        let params: Vec<ParamSig> = variants[idx]
            .fields
            .iter()
            .map(|(n, t)| ParamSig {
                name: n.clone(),
                ty: t.clone(),
                has_default: false,
            })
            .collect();
        let vals = self.arrange(&params, &[], args);
        if vals.is_empty() {
            return (format!("k_enum({idx}, 0, NULL)"), None);
        }
        let kvs: Vec<String> = vals
            .iter()
            .zip(&params)
            .map(|(v, p)| self.to_kv(v, &p.ty))
            .collect();
        let call = format!("k_enum({idx}, {}, (kv[]){{{}}})", kvs.len(), kvs.join(", "));
        (call, None)
    }

    /// `Post(title: "Hi", ...)`
    fn construct(&mut self, id: AdtId, args: &[Arg], span: Span) -> String {
        let fields = match &self.p.adts[id].kind {
            AdtKind::Struct(fields) => fields.clone(),
            AdtKind::Enum(_) => return self.unsupported(span, "this call"),
        };
        if fields.is_empty() {
            if self.is_prim(id) {
                let what = format!("creating a `{}`", self.p.adts[id].name);
                return self.unsupported(span, what);
            }
            return "NULL".into();
        }
        let decl = self.structs.get(&id).copied();
        let defaults: Vec<Option<&Expr>> = fields
            .iter()
            .map(|f| {
                decl.and_then(|d| d.fields.iter().find(|df| df.name.name == f.name))
                    .and_then(|df| df.default.as_ref())
            })
            .collect();
        let params: Vec<ParamSig> = fields
            .iter()
            .map(|f| ParamSig {
                name: Some(f.name.clone()),
                ty: f.ty.clone(),
                has_default: f.has_default,
            })
            .collect();
        let vals = self.arrange(&params, &defaults, args);
        let ty = Ty::Adt(id, Vec::new());
        let t = self.tmp(&ty);
        let name = self.struct_name(id);
        self.line(&format!("{t} = k_alloc(sizeof({name}));"));
        for (f, v) in fields.iter().zip(vals) {
            self.line(&format!("{t}->f_{} = {v};", mangle(&f.name)));
        }
        t
    }

    fn call_closure(
        &mut self,
        c: &str,
        ct: &Ty,
        args: &[Arg],
        rt: &Ty,
        span: Span,
    ) -> (String, Option<Ty>) {
        let Ty::Fn(f) = ct else {
            return (self.unsupported(span, "this call"), None);
        };
        let f: &FnTy = f;
        let t = self.tmp_c("kclosure *");
        self.line(&format!("{t} = {c};"));
        let params: Vec<ParamSig> = f
            .params
            .iter()
            .map(|ty| ParamSig {
                name: None,
                ty: ty.clone(),
                has_default: false,
            })
            .collect();
        let vals = self.arrange(&params, &[], args);
        let kvs: Vec<String> = vals
            .iter()
            .zip(&f.params)
            .map(|(v, ty)| self.to_kv(v, ty))
            .collect();
        let kvs = if kvs.is_empty() {
            "0".to_string()
        } else {
            kvs.join(", ")
        };
        let call = format!("{t}->fn({t}->env, (kv[]){{{kvs}}})");
        let call = if self.is_void(&f.ret) {
            call
        } else {
            let v = self.unkv(&call, &f.ret);
            self.convert(&v, &f.ret, rt)
        };
        (call, f.throws.clone())
    }

    /// A method call on a value, or `None` if `name` is not a method.
    fn call_method(
        &mut self,
        recv: &str,
        recv_ty: &Ty,
        name: &str,
        args: &[Arg],
        rt: &Ty,
        span: Span,
    ) -> Option<(String, Option<Ty>)> {
        let Ty::Adt(id, targs) = recv_ty else {
            return None;
        };
        let sig = self.p.adts[*id]
            .methods
            .iter()
            .find(|m| m.name == name)
            .cloned()?;
        if let Some(decl) = self.methods.get(&(*id, name.to_string())).copied() {
            let cname = self.method_name(*id, name);
            return Some(self.user_call(&cname, &sig, Some(decl), Some(recv), args, rt));
        }
        let vals = self.arrange(&sig.params, &[], args);
        let call = self.builtin_method(recv, *id, targs, name, &vals, args, span);
        let call = if self.is_void(&sig.ret) {
            call
        } else {
            self.convert(&call, &sig.ret, rt)
        };
        Some((call, None))
    }

    #[allow(clippy::too_many_arguments)]
    fn builtin_method(
        &mut self,
        r: &str,
        id: AdtId,
        targs: &[Ty],
        name: &str,
        a: &[String],
        args: &[Arg],
        span: Span,
    ) -> String {
        let arg = |i: usize| a.get(i).cloned().unwrap_or_else(|| "0".into());
        let p = &self.prims;
        if id == p.int {
            return match name {
                "to_string" => format!("k_int_str({r})"),
                "to_float" => format!("((double)({r}))"),
                "abs" => format!("k_abs({r})"),
                "min" => format!("k_min({r}, {})", arg(0)),
                "max" => format!("k_max({r}, {})", arg(0)),
                _ => self.unsupported(span, format!("`Int.{name}`")),
            };
        }
        if id == p.float {
            return match name {
                "to_string" => format!("k_float_str({r})"),
                "round" | "floor" | "ceil" => format!("k_float_to_int({name}({r}))"),
                _ => self.unsupported(span, format!("`Float.{name}`")),
            };
        }
        if id == p.string {
            return match name {
                "len" => format!("k_str_len({r})"),
                "is_empty" => format!("(({r})->len == 0)"),
                "upper" | "lower" | "trim" | "to_int" => format!("k_str_{name}({r})"),
                "contains" | "starts_with" | "ends_with" | "split" => {
                    format!("k_str_{name}({r}, {})", arg(0))
                }
                "replace" => format!("k_str_replace({r}, {}, {})", arg(0), arg(1)),
                _ => self.unsupported(span, format!("`String.{name}`")),
            };
        }
        if id == p.list {
            let elem = targs.first().cloned().unwrap_or(Ty::Unknown);
            return match name {
                "len" => format!("(({r})->len)"),
                "is_empty" => format!("(({r})->len == 0)"),
                "first" | "last" | "reverse" => format!("k_list_{name}({r})"),
                "get" | "push" | "filter" | "map" | "find" | "any" | "all" => {
                    format!("k_list_{name}({r}, {})", arg(0))
                }
                "contains" => {
                    let eq = self.eq_fn(&elem, span);
                    format!("k_list_contains({r}, {}, {eq})", arg(0))
                }
                "sort_by" => {
                    let key = match args.first().map(|a| self.ty(&a.value)) {
                        Some(Ty::Fn(f)) => f.ret.clone(),
                        _ => Ty::Unknown,
                    };
                    let cmp = self.cmp_fn(&key, span);
                    format!("k_list_sort_by({r}, {}, {cmp})", arg(0))
                }
                "join" => {
                    let show = self.show_fn(&elem, false, span);
                    format!("k_list_join({r}, {}, {show})", arg(0))
                }
                _ => self.unsupported(span, format!("`List.{name}`")),
            };
        }
        let what = format!("`{}.{name}`", self.p.adts[id].name);
        self.unsupported(span, what)
    }

    fn module_call(
        &mut self,
        module: &str,
        name: &Ident,
        args: &[Arg],
        span: Span,
    ) -> (String, Option<Ty>) {
        if module == "log" && matches!(name.name.as_str(), "debug" | "info" | "warn" | "error") {
            let mut shown = Vec::new();
            for a in args {
                let v = self.expr(&a.value);
                shown.push(self.display(&v, &self.ty(&a.value), a.value.span));
            }
            if shown.is_empty() {
                return (self.unsupported(span, "this call"), None);
            }
            let message = shown.remove(0);
            let data = if shown.is_empty() {
                "NULL".to_string()
            } else {
                format!("(kstr *[]){{{}}}", shown.join(", "))
            };
            let call = format!(
                "k_log(\"{}\", {message}, {}, {data})",
                name.name,
                shown.len()
            );
            return (call, None);
        }
        let what = format!("`{module}.{}`", name.name);
        (self.unsupported(span, what), None)
    }

    // ----- showing, comparing and ordering values -----

    /// A `kstr *` showing the value, as `print` and interpolation do.
    fn display(&mut self, e: &str, ty: &Ty, span: Span) -> String {
        match ty {
            _ if self.is_string(ty) => e.to_string(),
            Ty::Adt(id, _) if *id == self.prims.int => format!("k_int_str({e})"),
            Ty::Adt(id, _) if *id == self.prims.float => format!("k_float_str({e})"),
            Ty::Adt(id, _) if *id == self.prims.bool_ => format!("k_bool_str({e})"),
            Ty::Adt(id, _) if *id == self.prims.duration => format!("k_duration_str({e})"),
            _ => {
                let f = self.show_fn(ty, false, span);
                format!("{f}({})", self.to_kv(e, ty))
            }
        }
    }

    fn helper(&mut self, key: String, prefix: &str) -> (String, bool) {
        if let Some(name) = self.helper_names.get(&key) {
            return (name.clone(), false);
        }
        let name = format!("{prefix}{}", self.next());
        self.helper_names.insert(key, name.clone());
        (name, true)
    }

    fn error_message_fn(&mut self) -> String {
        let (name, fresh) = self.helper("error-message".into(), "kerr_message");
        if fresh {
            let s = self.struct_name(self.prims.error);
            let _ = writeln!(self.protos, "static kstr *{name}(kv v);");
            let _ = write!(
                self.helpers,
                "static kstr *{name}(kv v) {{ return K_KV2P({s} *, v)->f_message; }}\n\n"
            );
        }
        name
    }

    /// A `kstr *(kv)` function that shows values of `ty`. Strings are
    /// quoted when `quote` is set, as they are inside lists and structs.
    fn show_fn(&mut self, ty: &Ty, quote: bool, span: Span) -> String {
        let (name, fresh) = self.helper(format!("show:{quote}:{ty:?}"), "ksh");
        if !fresh {
            return name;
        }
        let _ = writeln!(self.protos, "static kstr *{name}(kv v);");
        let body = match ty {
            _ if self.is_string(ty) => {
                if quote {
                    "    return k_str_quote(K_KV2P(kstr *, v));\n".to_string()
                } else {
                    "    return K_KV2P(kstr *, v);\n".to_string()
                }
            }
            Ty::Adt(id, _) if self.rep(ty) != Rep::Ptr => {
                let v = self.unkv("v", ty);
                let f = if *id == self.prims.float {
                    "k_float_str"
                } else if *id == self.prims.bool_ {
                    "k_bool_str"
                } else if *id == self.prims.duration {
                    "k_duration_str"
                } else {
                    "k_int_str"
                };
                format!("    return {f}({v});\n")
            }
            Ty::Opt(inner) => {
                let f = self.show_fn(inner, quote, span);
                format!("    kbox *b = K_KV2P(kbox *, v);\n    return b ? {f}(b->v) : k_str(\"none\");\n")
            }
            Ty::Fn(_) => "    return k_str(\"<fn>\");\n".to_string(),
            Ty::Adt(id, args) if *id == self.prims.list => {
                let f = self.show_fn(&args[0], true, span);
                format!("    return k_list_show(K_KV2P(klist *, v), {f});\n")
            }
            Ty::Adt(id, args) if !self.is_prim(*id) => {
                let adt = self.p.adts[*id].clone();
                let map = self.type_map(*id, args);
                match &adt.kind {
                    AdtKind::Struct(fields) if fields.is_empty() => {
                        format!("    return {};\n", c_string(&adt.name))
                    }
                    AdtKind::Struct(fields) => {
                        let s = self.struct_name(*id);
                        let mut b = format!(
                            "    {s} *x = K_KV2P({s} *, v);\n    kstr *r = {};\n",
                            c_string(&format!("{}(", adt.name))
                        );
                        for (i, f) in fields.iter().enumerate() {
                            let fty = substitute(&f.ty, &map);
                            let show = self.show_fn(&fty, true, span);
                            let raw = self.to_kv(&format!("x->f_{}", mangle(&f.name)), &f.ty);
                            let label = if i == 0 {
                                format!("{}: ", f.name)
                            } else {
                                format!(", {}: ", f.name)
                            };
                            let _ = writeln!(
                                b,
                                "    r = k_concat(r, {});\n    r = k_concat(r, {show}({raw}));",
                                c_string(&label)
                            );
                        }
                        b.push_str("    return k_concat(r, k_str(\")\"));\n");
                        b
                    }
                    AdtKind::Enum(variants) => {
                        let mut b = "    kenum *x = K_KV2P(kenum *, v);\n    switch (x->tag) {\n"
                            .to_string();
                        for (i, var) in variants.iter().enumerate() {
                            if var.fields.is_empty() {
                                let _ =
                                    writeln!(b, "    case {i}: return {};", c_string(&var.name));
                                continue;
                            }
                            let _ = writeln!(
                                b,
                                "    case {i}: {{\n        kstr *r = {};",
                                c_string(&format!("{}(", var.name))
                            );
                            for (j, (fname, fty)) in var.fields.iter().enumerate() {
                                let fty = substitute(fty, &map);
                                let show = self.show_fn(&fty, true, span);
                                let sep = if j == 0 { "" } else { ", " };
                                let label = match fname {
                                    Some(n) => format!("{sep}{n}: "),
                                    None => sep.to_string(),
                                };
                                let _ = writeln!(
                                    b,
                                    "        r = k_concat(r, {});\n        r = k_concat(r, {show}(x->f[{j}]));",
                                    c_string(&label)
                                );
                            }
                            b.push_str("        return k_concat(r, k_str(\")\"));\n    }\n");
                        }
                        b.push_str("    }\n    return k_str(\"?\");\n");
                        b
                    }
                }
            }
            Ty::Param(_) | Ty::Var(_) | Ty::Any => {
                self.unsupported(span, "showing a value whose type is a generic parameter");
                "    return k_str(\"\");\n".to_string()
            }
            _ => {
                self.unsupported(span, "showing a value of this type");
                "    return k_str(\"\");\n".to_string()
            }
        };
        let _ = write!(self.helpers, "static kstr *{name}(kv v) {{\n{body}}}\n\n");
        name
    }

    /// Whether two values are equal, as a C expression.
    fn eq_expr(&mut self, a: &str, at: &Ty, b: &str, bt: &Ty, span: Span) -> String {
        let (a, at, b) = match (at, bt) {
            (Ty::Opt(_), other) if !matches!(other, Ty::Opt(_)) => {
                (a.to_string(), at.clone(), self.convert(b, bt, at))
            }
            (other, Ty::Opt(_)) if !matches!(other, Ty::Opt(_)) => {
                (self.convert(a, at, bt), bt.clone(), b.to_string())
            }
            _ => (a.to_string(), at.clone(), b.to_string()),
        };
        if self.is_string(&at) {
            return format!("k_str_eq({a}, {b})");
        }
        match self.rep(&at) {
            Rep::Int | Rep::Bool | Rep::Float => format!("(({a}) == ({b}))"),
            _ => {
                let f = self.eq_fn(&at, span);
                format!("{f}({}, {})", self.to_kv(&a, &at), self.to_kv(&b, &at))
            }
        }
    }

    /// A `bool (kv, kv)` function that compares values of `ty`.
    fn eq_fn(&mut self, ty: &Ty, span: Span) -> String {
        let (name, fresh) = self.helper(format!("eq:{ty:?}"), "keq");
        if !fresh {
            return name;
        }
        let _ = writeln!(self.protos, "static bool {name}(kv a, kv b);");
        let body = match ty {
            _ if self.is_string(ty) => {
                "    return k_str_eq(K_KV2P(kstr *, a), K_KV2P(kstr *, b));\n".to_string()
            }
            Ty::Adt(id, _) if *id == self.prims.float => {
                "    return k_kv2f(a) == k_kv2f(b);\n".to_string()
            }
            Ty::Adt(..) if self.rep(ty) != Rep::Ptr => "    return a == b;\n".to_string(),
            Ty::Fn(_) => "    return a == b;\n".to_string(),
            Ty::Opt(inner) => {
                let f = self.eq_fn(inner, span);
                format!("    kbox *x = K_KV2P(kbox *, a), *y = K_KV2P(kbox *, b);\n    if (!x || !y) return x == y;\n    return {f}(x->v, y->v);\n")
            }
            Ty::Adt(id, args) if *id == self.prims.list => {
                let f = self.eq_fn(&args[0], span);
                format!("    klist *x = K_KV2P(klist *, a), *y = K_KV2P(klist *, b);\n    if (x->len != y->len) return false;\n    for (int64_t i = 0; i < x->len; i++)\n        if (!{f}(x->items[i], y->items[i])) return false;\n    return true;\n")
            }
            Ty::Adt(id, args) if !self.is_prim(*id) => {
                let adt = self.p.adts[*id].clone();
                let map = self.type_map(*id, args);
                match &adt.kind {
                    AdtKind::Struct(fields) if fields.is_empty() => {
                        "    return true;\n".to_string()
                    }
                    AdtKind::Struct(fields) => {
                        let s = self.struct_name(*id);
                        let mut b =
                            format!("    {s} *x = K_KV2P({s} *, a), *y = K_KV2P({s} *, b);\n");
                        for f in fields {
                            let fty = substitute(&f.ty, &map);
                            let eq = self.eq_fn(&fty, span);
                            let fname = format!("f_{}", mangle(&f.name));
                            let xa = self.to_kv(&format!("x->{fname}"), &f.ty);
                            let ya = self.to_kv(&format!("y->{fname}"), &f.ty);
                            let _ = writeln!(b, "    if (!{eq}({xa}, {ya})) return false;");
                        }
                        b.push_str("    return true;\n");
                        b
                    }
                    AdtKind::Enum(variants) => {
                        let mut b = "    kenum *x = K_KV2P(kenum *, a), *y = K_KV2P(kenum *, b);\n    if (x->tag != y->tag) return false;\n    switch (x->tag) {\n".to_string();
                        for (i, var) in variants.iter().enumerate() {
                            if var.fields.is_empty() {
                                continue;
                            }
                            let _ = writeln!(b, "    case {i}:");
                            for (j, (_, fty)) in var.fields.iter().enumerate() {
                                let fty = substitute(fty, &map);
                                let eq = self.eq_fn(&fty, span);
                                let _ = writeln!(
                                    b,
                                    "        if (!{eq}(x->f[{j}], y->f[{j}])) return false;"
                                );
                            }
                            b.push_str("        return true;\n");
                        }
                        b.push_str("    }\n    return true;\n");
                        b
                    }
                }
            }
            Ty::Param(_) | Ty::Var(_) | Ty::Any => {
                self.unsupported(span, "comparing values whose type is a generic parameter");
                "    return false;\n".to_string()
            }
            _ => {
                self.unsupported(span, "comparing values of this type");
                "    return false;\n".to_string()
            }
        };
        let _ = write!(
            self.helpers,
            "static bool {name}(kv a, kv b) {{\n{body}}}\n\n"
        );
        name
    }

    /// An `int (kv, kv)` function that orders values of `ty`.
    fn cmp_fn(&mut self, ty: &Ty, span: Span) -> String {
        let (name, fresh) = self.helper(format!("cmp:{ty:?}"), "kcmp");
        if !fresh {
            return name;
        }
        let _ = writeln!(self.protos, "static int {name}(kv a, kv b);");
        let body = match self.rep(ty) {
            _ if self.is_string(ty) => {
                "    return k_str_cmp(K_KV2P(kstr *, a), K_KV2P(kstr *, b));\n".to_string()
            }
            Rep::Float => {
                "    double x = k_kv2f(a), y = k_kv2f(b);\n    return x < y ? -1 : x > y;\n"
                    .to_string()
            }
            Rep::Int => {
                "    int64_t x = (int64_t)a, y = (int64_t)b;\n    return x < y ? -1 : x > y;\n"
                    .to_string()
            }
            Rep::Bool => "    return (int)(a != 0) - (int)(b != 0);\n".to_string(),
            _ => {
                self.unsupported(span, "sorting by a key of this type");
                "    return 0;\n".to_string()
            }
        };
        let _ = write!(
            self.helpers,
            "static int {name}(kv a, kv b) {{\n{body}}}\n\n"
        );
        name
    }
}

/// A C identifier for a Kiln name.
fn mangle(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            let _ = write!(out, "_u{:x}", c as u32);
        }
    }
    out
}

/// A `kstr *` expression for a string constant.
fn c_string(s: &str) -> String {
    let mut lit = String::new();
    for b in s.bytes() {
        match b {
            b'"' => lit.push_str("\\\""),
            b'\\' => lit.push_str("\\\\"),
            b'?' => lit.push_str("\\?"),
            0x20..=0x7e => lit.push(b as char),
            _ => {
                let _ = write!(lit, "\\{b:03o}");
            }
        }
    }
    format!("k_str_n(\"{lit}\", {})", s.len())
}

// ----- names used inside closures -----

/// Collects the names used inside closures anywhere in `b` (or every name,
/// when `inside` is set).
fn closure_names_block(b: &Block, inside: bool, out: &mut HashSet<String>) {
    for stmt in &b.stmts {
        match stmt {
            Stmt::Let { value, .. } => closure_names(value, inside, out),
            Stmt::Assign { target, value, .. } => {
                closure_names(target, inside, out);
                closure_names(value, inside, out);
            }
            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    closure_names(v, inside, out);
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::For {
                iter, filter, body, ..
            } => {
                closure_names(iter, inside, out);
                if let Some(f) = filter {
                    closure_names(f, inside, out);
                }
                closure_names_block(body, inside, out);
            }
            Stmt::While { cond, body, .. } => {
                closure_names(cond, inside, out);
                closure_names_block(body, inside, out);
            }
            Stmt::Expr(e) => closure_names(e, inside, out),
        }
    }
}

fn closure_names(e: &Expr, inside: bool, out: &mut HashSet<String>) {
    match &e.kind {
        ExprKind::Ident(name) => {
            if inside {
                out.insert(name.clone());
            }
        }
        ExprKind::Closure { body, .. } => closure_names_block(body, true, out),
        ExprKind::Record(fields) if inside => {
            for f in fields {
                match &f.value {
                    Some(v) => closure_names(v, inside, out),
                    None => {
                        out.insert(f.name.name.clone());
                    }
                }
            }
        }
        _ => {
            let mut exprs = Vec::new();
            let mut blocks = Vec::new();
            for_each_child(e, &mut |x| exprs.push(x), &mut |b| blocks.push(b));
            for x in exprs {
                closure_names(x, inside, out);
            }
            for b in blocks {
                closure_names_block(b, inside, out);
            }
        }
    }
}

/// Every name used in `b`, including inside nested closures.
fn names_in_block(b: &Block, out: &mut BTreeSet<String>) {
    let mut set = HashSet::new();
    closure_names_block(b, true, &mut set);
    out.extend(set);
}

/// Calls `expr` on each child expression of `e` and `block` on each child
/// block, except for closures, which the callers handle.
fn for_each_child<'e>(
    e: &'e Expr,
    expr: &mut dyn FnMut(&'e Expr),
    block: &mut dyn FnMut(&'e Block),
) {
    match &e.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Duration(_)
        | ExprKind::Bool(_)
        | ExprKind::None
        | ExprKind::Ident(_)
        | ExprKind::FieldShorthand(_)
        | ExprKind::Closure { .. }
        | ExprKind::Error => {}
        ExprKind::Str(pieces) => {
            for p in pieces {
                if let StrPiece::Expr(inner) = p {
                    expr(inner);
                }
            }
        }
        ExprKind::List(items) => {
            for item in items {
                expr(item);
            }
        }
        ExprKind::Record(fields) => {
            for f in fields {
                if let Some(v) = &f.value {
                    expr(v);
                }
            }
        }
        ExprKind::Unary { expr: inner, .. } => expr(inner),
        ExprKind::Binary { lhs, rhs, .. } => {
            expr(lhs);
            expr(rhs);
        }
        ExprKind::Call { callee, args, .. } => {
            expr(callee);
            for a in args {
                expr(&a.value);
            }
        }
        ExprKind::Field { expr: inner, .. } => expr(inner),
        ExprKind::Index { expr: inner, index } => {
            expr(inner);
            expr(index);
        }
        ExprKind::If {
            cond,
            then,
            otherwise,
        } => {
            expr(cond);
            block(then);
            if let Some(o) = otherwise {
                expr(o);
            }
        }
        ExprKind::Block(b) => block(b),
        ExprKind::Match { scrutinee, arms } => {
            expr(scrutinee);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    expr(g);
                }
                expr(&arm.body);
            }
        }
        ExprKind::Try(inner) | ExprKind::Throw(inner) => expr(inner),
        ExprKind::Catch {
            expr: inner,
            handler,
            ..
        } => {
            expr(inner);
            block(handler);
        }
        ExprKind::Is { expr: inner, .. } => expr(inner),
    }
}
