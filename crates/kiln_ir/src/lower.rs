//! Lowers a checked syntax tree to the IR.
//!
//! The lowering reads the types the checker recorded for every expression
//! and binding (by span), resolves every name, and makes implicit work
//! explicit: coercions, argument order, defaults, captures and string
//! interpolation. Code the compiler cannot build yet is reported as E0401
//! here, at the spot that uses it.

use crate::*;
use kiln_check::style::StyleExpr;
use kiln_check::types::{substitute, FnSig, ParamSig};
use kiln_diagnostics::{codes, Diagnostic};
use kiln_syntax::ast;
use kiln_syntax::ast::{
    AssignOp, BinaryOp, ExprKind as E, PatternKind, StrPiece, TypeKind, UnaryOp,
};
use std::collections::HashMap;
use std::rc::Rc;

/// Lowers a program that type-checked without errors. Returns E0401 for
/// the parts the compiler does not support yet, and E0402 when there is
/// no `main`.
/// A parameter whose default is the text `"@caller"` gets, when it is left
/// out, where the call is: `kiln test` gives its assertions an `at:` this
/// way, so a failure names the line of the test. The CLI says how an
/// offset in the program's text is shown; without that, it is `""`.
pub const CALLER_DEFAULT: &str = "@caller";

type CallerLocation = Box<dyn Fn(u32) -> String + Send>;
static CALLER_LOCATION: std::sync::Mutex<Option<CallerLocation>> = std::sync::Mutex::new(None);

/// Sets how [`CALLER_DEFAULT`] parameters show a call's place.
pub fn set_caller_location(f: Option<CallerLocation>) {
    *CALLER_LOCATION.lock().unwrap() = f;
}

fn caller_location(offset: u32) -> String {
    CALLER_LOCATION
        .lock()
        .unwrap()
        .as_ref()
        .map(|f| f(offset))
        .unwrap_or_default()
}

pub fn lower(
    module: &ast::Module,
    program: &kiln_check::Program,
) -> Result<Program, Vec<Diagnostic>> {
    let mut l = Lowerer::new(module, program);
    l.program();
    if !l.diags.is_empty() {
        let mut diags = l.diags;
        diags.sort_by_key(|d| (d.span.start, d.span.end));
        diags.dedup_by(|a, b| a.span == b.span && a.message == b.message);
        return Err(diags);
    }
    let builtin = (0..program.adts.len())
        .map(|id| {
            id < program.std_adts
                && program.global_types.get(&program.adts[id].name) == Some(&id)
                && matches!(&program.adts[id].kind, AdtKind::Struct(f) if f.is_empty())
        })
        .collect();
    // A type from a folder module carries its module in its name; a value
    // is shown with the name its source gave it.
    let mut adts = program.adts.clone();
    for adt in &mut adts {
        if let Some(name) = source_name(&adt.name) {
            adt.name = name;
        }
    }
    Ok(Program {
        adts,
        std_adts: program.std_adts,
        builtin,
        prims: l.prims,
        globals: l.globals,
        stores: l.stores,
        funcs: l
            .funcs
            .into_iter()
            .map(|f| f.expect("every function is lowered"))
            .collect(),
        init: l.init,
        main: l.main,
    })
}

/// A function being lowered.
struct FnCtx {
    locals: Vec<Local>,
    /// Whether a closure captures each local.
    captured: Vec<bool>,
    scopes: Vec<HashMap<String, LocalId>>,
    /// For closures: (local in the enclosing function, local here).
    captures: Vec<(LocalId, LocalId)>,
    closure: bool,
    ret: Ty,
    throws: Option<Ty>,
    /// In a copy of a generic function (and its closures): the type each
    /// type parameter stands for.
    subst: Rc<HashMap<String, Ty>>,
}

/// A generic function or method, compiled once per set of type arguments.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Template {
    Fn(String),
    Method(AdtId, String),
}

/// A copy of a generic function still to lower: its id, template, type
/// parameters with their types, and the type arguments in order.
type PendingCopy = (FuncId, Template, HashMap<String, Ty>, Vec<Ty>);

/// Stands for the id of a generic function or method in `fns` and
/// `methods`: each use asks [`Lowerer::instance`] for its copy.
const GENERIC: FuncId = usize::MAX;

/// How many copies one generic function may have, which stops a function
/// that calls itself with ever larger types.
const MAX_COPIES: usize = 256;

struct Lowerer<'a> {
    p: &'a kiln_check::Program,
    module: &'a ast::Module,
    prims: Prims,
    fns: HashMap<String, (FuncId, &'a ast::FnDecl)>,
    methods: HashMap<(AdtId, String), (FuncId, &'a ast::FnDecl)>,
    structs: HashMap<AdtId, &'a ast::StructDecl>,
    global_ids: HashMap<String, GlobalId>,
    globals: Vec<Global>,
    store_ids: HashMap<String, StoreId>,
    stores: Vec<Store>,
    funcs: Vec<Option<Func>>,
    ctxs: Vec<FnCtx>,
    diags: Vec<Diagnostic>,
    init: FuncId,
    main: FuncId,
    /// The copies of generic functions made so far, by template and type
    /// arguments.
    instances: HashMap<(Template, String), FuncId>,
    copies: HashMap<Template, usize>,
    pending: Vec<PendingCopy>,
    /// Standard library modules parsed for their default arguments.
    std_asts: HashMap<String, ast::Module>,
    /// Functions the lowering makes itself, such as the one each type of
    /// transaction runs through, by what they are for.
    helpers: HashMap<String, FuncId>,
    /// The call whose arguments are being arranged, for `CALLER_DEFAULT`.
    call_span: Span,
}

impl<'a> Lowerer<'a> {
    fn new(module: &'a ast::Module, p: &'a kiln_check::Program) -> Self {
        let id = |name: &str| p.global_types[name];
        Lowerer {
            p,
            module,
            prims: Prims {
                int: id("Int"),
                float: id("Float"),
                bool_: id("Bool"),
                string: id("String"),
                bytes: id("Bytes"),
                duration: id("Duration"),
                list: id("List"),
                map: id("Map"),
                set: id("Set"),
                error: id("Error"),
            },
            fns: HashMap::new(),
            methods: HashMap::new(),
            structs: HashMap::new(),
            global_ids: HashMap::new(),
            globals: Vec::new(),
            store_ids: HashMap::new(),
            stores: Vec::new(),
            funcs: Vec::new(),
            ctxs: Vec::new(),
            diags: Vec::new(),
            init: 0,
            main: 0,
            instances: HashMap::new(),
            std_asts: HashMap::new(),
            helpers: HashMap::new(),
            call_span: Span::new(0, 0),
            copies: HashMap::new(),
            pending: Vec::new(),
        }
    }

    // ----- the program -----

    fn program(&mut self) {
        let module = self.module;
        for item in &module.items {
            match item {
                ast::Item::Fn(f) => {
                    if let Some(sig) = self.p.global_fns.get(&f.name.name) {
                        let id = if is_generic(sig) {
                            GENERIC
                        } else {
                            self.reserve()
                        };
                        self.fns.insert(f.name.name.clone(), (id, f));
                    }
                }
                ast::Item::Struct(s) => {
                    if let Some(&id) = self.p.global_types.get(&s.name.name) {
                        self.structs.insert(id, s);
                    }
                }
                ast::Item::Impl(imp) => {
                    if let Some(adt) = self.impl_target(imp) {
                        for m in &imp.methods {
                            let sig = self.p.adts[adt]
                                .methods
                                .iter()
                                .find(|s| s.name == m.name.name);
                            if let Some(sig) = sig {
                                let id = if is_generic(sig) {
                                    GENERIC
                                } else {
                                    self.reserve()
                                };
                                self.methods.insert((adt, m.name.name.clone()), (id, m));
                            }
                        }
                    }
                }
                ast::Item::Const(c) if c.store.is_some() => {
                    let ty = self
                        .p
                        .consts
                        .get(&c.name.name)
                        .cloned()
                        .unwrap_or(Ty::Unknown);
                    let get = self.reserve();
                    self.store_ids
                        .insert(c.name.name.clone(), self.stores.len());
                    self.stores.push(Store {
                        name: c.name.name.clone(),
                        ty,
                        get,
                        persist: c
                            .store
                            .as_ref()
                            .and_then(|s| s.persist.as_ref())
                            .map(|p| p.0.clone()),
                    });
                }
                ast::Item::Const(c) => {
                    let ty = self
                        .p
                        .consts
                        .get(&c.name.name)
                        .cloned()
                        .unwrap_or(Ty::Unknown);
                    self.global_ids
                        .insert(c.name.name.clone(), self.globals.len());
                    self.globals.push(Global {
                        name: c.name.name.clone(),
                        ty,
                    });
                }
                _ => {}
            }
        }
        let main = self
            .fns
            .get("main")
            .map(|f| f.0)
            .filter(|&id| id != GENERIC);
        let Some(main) = main else {
            self.diags.push(
                Diagnostic::error(
                    codes::NO_MAIN,
                    "this program has no `main` function",
                    Span::new(0, 0),
                )
                .with_help("add `fn main() { ... }`; the program starts there"),
            );
            return;
        };
        self.main = main;
        self.init = self.reserve();
        self.init_fn();
        for item in &module.items {
            if let ast::Item::Const(c) = item {
                if let Some(&id) = self.store_ids.get(&c.name.name) {
                    self.store_fn(id, c);
                }
            }
        }
        for item in &module.items {
            match item {
                ast::Item::Fn(f) => {
                    if let Some(&(id, _)) = self.fns.get(&f.name.name) {
                        if id != GENERIC {
                            let sig = self.p.global_fns[&f.name.name].clone();
                            self.function(id, f, &sig, FuncKind::Fn, HashMap::new(), Vec::new());
                        }
                    }
                }
                ast::Item::Impl(imp) => {
                    if let Some(adt) = self.impl_target(imp) {
                        for m in &imp.methods {
                            let key = (adt, m.name.name.clone());
                            if let Some(&(id, _)) = self.methods.get(&key) {
                                if id == GENERIC {
                                    continue;
                                }
                                let sig = self.p.adts[adt]
                                    .methods
                                    .iter()
                                    .find(|s| s.name == m.name.name)
                                    .cloned()
                                    .expect("the method has a signature");
                                let kind = FuncKind::Method(adt);
                                self.function(id, m, &sig, kind, HashMap::new(), Vec::new());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // Copies of generic functions, which can ask for more copies.
        while let Some((id, template, map, args)) = self.pending.pop() {
            let (decl, sig, kind) = match &template {
                Template::Fn(name) => (
                    self.fns[name].1,
                    self.p.global_fns[name].clone(),
                    FuncKind::Fn,
                ),
                Template::Method(adt, name) => (
                    self.methods[&(*adt, name.clone())].1,
                    self.p.adts[*adt]
                        .methods
                        .iter()
                        .find(|s| &s.name == name)
                        .cloned()
                        .expect("the method has a signature"),
                    FuncKind::Method(*adt),
                ),
            };
            self.function(id, decl, &sig, kind, map, args);
        }
    }

    /// The copy of a generic function for one use of it, by the span of
    /// the call or name, and its signature with the type arguments filled
    /// in. A method's receiver type gives the `impl` target's arguments.
    /// Other functions are returned as they are.
    fn instance(
        &mut self,
        func: FuncId,
        template: Template,
        sig: &FnSig,
        span: Span,
        recv: Option<&Ty>,
    ) -> (FuncId, FnSig) {
        if func != GENERIC {
            return (func, sig.clone());
        }
        let mut map: HashMap<String, Ty> = self
            .p
            .instantiations
            .get(&span)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(k, t)| (k, self.subst_ty(t)))
            .collect();
        if let Some(Ty::Adt(_, args)) = recv {
            for (p, a) in sig.impl_params.iter().zip(args) {
                map.insert(p.clone(), a.clone());
            }
        }
        let names: Vec<String> = sig
            .impl_params
            .iter()
            .chain(&sig.generics)
            .cloned()
            .collect();
        let args: Vec<Ty> = names
            .iter()
            .map(|n| map.get(n).cloned().unwrap_or(Ty::Unknown))
            .collect();
        let inst = FnSig {
            generics: Vec::new(),
            impl_params: Vec::new(),
            bounds: Vec::new(),
            params: sig
                .params
                .iter()
                .map(|p| ParamSig {
                    ty: substitute(&p.ty, &map),
                    ..p.clone()
                })
                .collect(),
            ret: substitute(&sig.ret, &map),
            throws: sig.throws.as_ref().map(|t| substitute(t, &map)),
            ..sig.clone()
        };
        let key = (template.clone(), format!("{args:?}"));
        if let Some(&id) = self.instances.get(&key) {
            return (id, inst);
        }
        let copies = self.copies.entry(template.clone()).or_insert(0);
        *copies += 1;
        if *copies > MAX_COPIES {
            let what = format!("`{}` calling itself with ever larger types", sig.name);
            self.unsupported(span, what);
            return (func, inst);
        }
        let id = self.reserve();
        self.instances.insert(key, id);
        let map = names.into_iter().zip(args.iter().cloned()).collect();
        self.pending.push((id, template, map, args));
        (id, inst)
    }

    fn impl_target(&self, imp: &ast::ImplDecl) -> Option<AdtId> {
        match &imp.target.kind {
            TypeKind::Named { path, .. } => self.p.global_types.get(&path[0].name).copied(),
            _ => None,
        }
    }

    fn reserve(&mut self) -> FuncId {
        self.funcs.push(None);
        self.funcs.len() - 1
    }

    fn init_fn(&mut self) {
        self.enter(false, Ty::Unit, None);
        let mut stmts = Vec::new();
        let module = self.module;
        for item in &module.items {
            if let ast::Item::Const(c) = item {
                if c.store.is_some() {
                    continue;
                }
                let id = self.global_ids[&c.name.name];
                let ty = self.globals[id].ty.clone();
                let v = self.expr(&c.value);
                stmts.push(Stmt::SetGlobal(id, coerce(v, &ty)));
            }
        }
        let body = Block { stmts, value: None };
        self.finish(
            self.init,
            "<init>".into(),
            FuncKind::Init,
            Vec::new(),
            Ty::Unit,
            body,
        );
    }

    /// A store's `get`: its value in the running scope's table, which it
    /// starts from the declaration's value the first time.
    fn store_fn(&mut self, id: StoreId, c: &ast::ConstDecl) {
        let ty = self.stores[id].ty.clone();
        let opt = Ty::Opt(Box::new(ty.clone()));
        let span = c.span;
        self.enter(false, ty.clone(), None);
        let found = self.new_local("value", ty.clone(), false);
        let slot = mk(ExprKind::StoreSlot(id), opt.clone(), span);
        let is = mk(
            ExprKind::Is {
                value: Box::new(slot),
                pattern: Box::new(Pattern::Some(Box::new(Pattern::Bind(found)), ty.clone())),
            },
            Ty::Adt(self.prims.bool_, Vec::new()),
            span,
        );
        let first = self.new_local("first", ty.clone(), false);
        let v = self.expr(&c.value);
        let start = Block {
            stmts: vec![
                Stmt::Let(first, coerce(v, &ty)),
                Stmt::SetStore(
                    id,
                    coerce(mk(ExprKind::Local(first), ty.clone(), span), &opt),
                ),
            ],
            value: Some(Box::new(mk(ExprKind::Local(first), ty.clone(), span))),
        };
        let pick = mk(
            ExprKind::If {
                cond: Box::new(is),
                then: Block {
                    stmts: Vec::new(),
                    value: Some(Box::new(mk(ExprKind::Local(found), ty.clone(), span))),
                },
                otherwise: Some(start),
            },
            ty.clone(),
            span,
        );
        let body = Block {
            stmts: vec![Stmt::Return(Some(pick))],
            value: None,
        };
        let get = self.stores[id].get;
        let name = format!("<store {}>", c.name.name);
        self.finish(get, name, FuncKind::Fn, Vec::new(), ty, body);
    }

    /// `stores.scope(body)`, which runs `body` between entering and
    /// leaving a fresh table of stores, and `stores.__ready(store)`.
    fn stores_call(&mut self, name: &ast::Ident, args: &[ast::Arg], span: Span) -> Expr {
        let bool_ = Ty::Adt(self.prims.bool_, Vec::new());
        match (name.name.as_str(), args) {
            ("__ready", [arg]) => {
                let id = match &arg.value.kind {
                    E::Ident(n) if self.lookup(n).is_none() => self.store_ids.get(n).copied(),
                    _ => None,
                };
                let Some(id) = id else {
                    return self
                        .unsupported(span, "`stores.__ready` of something other than a store");
                };
                let opt = Ty::Opt(Box::new(self.stores[id].ty.clone()));
                let slot = mk(ExprKind::StoreSlot(id), opt, span);
                let none = mk(ExprKind::IsNone(Box::new(slot)), bool_.clone(), span);
                mk(ExprKind::Not(Box::new(none)), bool_, span)
            }
            ("scope", [arg]) => {
                let body = self.expr(&arg.value);
                let lib = |name: &str| {
                    mk(
                        ExprKind::Lib {
                            module: "stores".into(),
                            name: name.into(),
                            args: Vec::new(),
                        },
                        Ty::Unit,
                        span,
                    )
                };
                let call = mk(
                    ExprKind::CallClosure {
                        callee: Box::new(body),
                        args: Vec::new(),
                    },
                    Ty::Unit,
                    span,
                );
                mk(
                    ExprKind::Block(Block {
                        stmts: vec![
                            Stmt::Expr(lib("enter")),
                            Stmt::Expr(call),
                            Stmt::Expr(lib("leave")),
                        ],
                        value: None,
                    }),
                    Ty::Unit,
                    span,
                )
            }
            _ => self.unsupported(span, format!("`stores.{}`", name.name)),
        }
    }

    /// Reading a store: a call of its `get`.
    fn store_read(&mut self, id: StoreId, span: Span) -> Expr {
        let store = &self.stores[id];
        mk(
            ExprKind::Call {
                func: store.get,
                args: Vec::new(),
            },
            store.ty.clone(),
            span,
        )
    }

    fn function(
        &mut self,
        id: FuncId,
        decl: &ast::FnDecl,
        sig: &FnSig,
        kind: FuncKind,
        subst: HashMap<String, Ty>,
        type_args: Vec<Ty>,
    ) {
        let ret = substitute(&sig.ret, &subst);
        let throws = sig.throws.as_ref().map(|t| substitute(t, &subst));
        self.enter(false, ret.clone(), throws);
        self.ctx().subst = Rc::new(subst);
        let params = decl
            .params
            .iter()
            .map(|p| {
                let ty = self.binding_ty(p.name.span).unwrap_or(Ty::Unknown);
                let local = self.new_local(&p.name.name, ty, false);
                self.bind(&p.name.name, local);
                local
            })
            .collect();
        let body = self.block(&decl.body);
        let body = coerce_block(body, &ret);
        self.finish(id, decl.name.name.clone(), kind, params, ret, body);
        if let Some(f) = self.funcs[id].as_mut() {
            f.type_args = type_args;
            f.server = decl.is_server;
        }
    }

    fn enter(&mut self, closure: bool, ret: Ty, throws: Option<Ty>) {
        // Closures see the type arguments of the function around them.
        let subst = match self.ctxs.last() {
            Some(outer) if closure => outer.subst.clone(),
            _ => Rc::default(),
        };
        self.ctxs.push(FnCtx {
            subst,
            locals: Vec::new(),
            captured: Vec::new(),
            scopes: vec![HashMap::new()],
            captures: Vec::new(),
            closure,
            ret,
            throws,
        });
    }

    /// Ends the current function and stores it. Returns the locals of the
    /// enclosing function that it captures.
    fn finish(
        &mut self,
        id: FuncId,
        name: String,
        kind: FuncKind,
        params: Vec<LocalId>,
        ret: Ty,
        body: Block,
    ) -> Vec<LocalId> {
        let ctx = self.ctxs.pop().expect("a function is being lowered");
        debug_assert!(ctx.ret == ret || matches!(kind, FuncKind::Init));
        let mut locals = ctx.locals;
        let inner: Vec<LocalId> = ctx.captures.iter().map(|c| c.1).collect();
        for (i, local) in locals.iter_mut().enumerate() {
            local.cell = local.mutable && (ctx.captured[i] || inner.contains(&i));
        }
        self.funcs[id] = Some(Func {
            name,
            kind,
            type_args: Vec::new(),
            params,
            captures: inner,
            ret,
            throws: ctx.throws,
            locals,
            body,
            server: false,
        });
        ctx.captures.iter().map(|c| c.0).collect()
    }

    // ----- names -----

    fn ctx(&mut self) -> &mut FnCtx {
        self.ctxs.last_mut().expect("a function is being lowered")
    }

    fn new_local(&mut self, name: &str, ty: Ty, mutable: bool) -> LocalId {
        let ctx = self.ctx();
        ctx.locals.push(Local {
            name: name.to_string(),
            ty,
            mutable,
            cell: false,
        });
        ctx.captured.push(false);
        ctx.locals.len() - 1
    }

    fn bind(&mut self, name: &str, local: LocalId) {
        self.ctx()
            .scopes
            .last_mut()
            .unwrap()
            .insert(name.to_string(), local);
    }

    fn push_scope(&mut self) {
        self.ctx().scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.ctx().scopes.pop();
    }

    /// The local a name refers to, capturing it from enclosing functions
    /// when the current function is a closure.
    fn lookup(&mut self, name: &str) -> Option<LocalId> {
        self.lookup_at(self.ctxs.len() - 1, name)
    }

    fn lookup_at(&mut self, depth: usize, name: &str) -> Option<LocalId> {
        let ctx = &self.ctxs[depth];
        if let Some(&id) = ctx.scopes.iter().rev().find_map(|s| s.get(name)) {
            return Some(id);
        }
        if !ctx.closure || depth == 0 {
            return None;
        }
        let outer = self.lookup_at(depth - 1, name)?;
        let parent = &mut self.ctxs[depth - 1];
        parent.captured[outer] = true;
        let decl = parent.locals[outer].clone();
        let ctx = &mut self.ctxs[depth];
        ctx.locals.push(Local {
            cell: false,
            ..decl
        });
        ctx.captured.push(false);
        let inner = ctx.locals.len() - 1;
        ctx.captures.push((outer, inner));
        ctx.scopes[0].insert(name.to_string(), inner);
        Some(inner)
    }

    // ----- types -----

    fn ty(&self, e: &ast::Expr) -> Ty {
        let ty = self
            .p
            .expr_types
            .get(&e.span)
            .cloned()
            .unwrap_or(Ty::Unknown);
        self.subst_ty(ty)
    }

    fn binding_ty(&self, span: Span) -> Option<Ty> {
        let ty = self.p.binding_types.get(&span).cloned()?;
        Some(self.subst_ty(ty))
    }

    /// A type the checker recorded, with the type arguments of the copy
    /// being lowered filled in.
    fn subst_ty(&self, ty: Ty) -> Ty {
        match self.ctxs.last() {
            Some(ctx) if !ctx.subst.is_empty() => substitute(&ty, &ctx.subst),
            _ => ty,
        }
    }

    fn is_adt(&self, ty: &Ty, id: AdtId) -> bool {
        matches!(ty, Ty::Adt(i, _) if *i == id)
    }

    fn is_error(&self, ty: &Ty) -> bool {
        self.is_adt(ty, self.prims.error)
    }

    fn type_map(&self, id: AdtId, args: &[Ty]) -> HashMap<String, Ty> {
        self.p.adts[id]
            .params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect()
    }

    /// The declared type and index of a struct field. A record's fields
    /// live in slots, so their declared type is `Any`.
    fn field_decl(&self, base: &Ty, name: &str) -> Option<(usize, Ty)> {
        if let Ty::Record(fields) = base {
            return fields
                .iter()
                .position(|f| f.0 == name)
                .map(|i| (i, Ty::Any));
        }
        let Ty::Adt(id, _) = base else { return None };
        let AdtKind::Struct(fields) = &self.p.adts[*id].kind else {
            return None;
        };
        fields
            .iter()
            .position(|f| f.name == name)
            .map(|i| (i, fields[i].ty.clone()))
    }

    /// Whether errors of type `err` are wrapped in `Error` when they leave
    /// the current function.
    fn wraps(&mut self, err: &Ty) -> bool {
        let target = self.ctx().throws.clone();
        target.as_ref().is_some_and(|t| self.is_error(t)) && !self.is_error(err)
    }

    fn unsupported(&mut self, span: Span, what: impl Into<String>) -> Expr {
        self.diags.push(
            Diagnostic::error(
                codes::NOT_COMPILED_YET,
                format!("{} is not supported by the compiler yet", what.into()),
                span,
            )
            .with_help("`kiln check` accepts this; building it needs a later version of Kiln"),
        );
        mk(ExprKind::Error, Ty::Unknown, span)
    }

    // ----- statements -----

    fn block(&mut self, b: &ast::Block) -> Block {
        self.push_scope();
        let mut out = Block::default();
        let last = b.stmts.len().saturating_sub(1);
        for (i, stmt) in b.stmts.iter().enumerate() {
            match stmt {
                ast::Stmt::Expr(e) if i == last => out.value = Some(Box::new(self.expr(e))),
                _ => {
                    if let Some(s) = self.stmt(stmt) {
                        out.stmts.push(s);
                    }
                }
            }
        }
        self.pop_scope();
        out
    }

    fn stmt(&mut self, stmt: &ast::Stmt) -> Option<Stmt> {
        Some(match stmt {
            ast::Stmt::Let {
                mutable,
                name,
                value,
                ..
            } => {
                let v = self.expr(value);
                let ty = self.binding_ty(name.span).unwrap_or_else(|| v.ty.clone());
                let v = coerce(v, &ty);
                let local = self.new_local(&name.name, ty, *mutable);
                self.bind(&name.name, local);
                Stmt::Let(local, v)
            }
            ast::Stmt::Assign {
                target, op, value, ..
            } => return self.assign(target, *op, value),
            ast::Stmt::Return { value, .. } => {
                let ret = self.current_ret();
                Stmt::Return(value.as_ref().map(|v| {
                    let x = self.expr(v);
                    coerce(x, &ret)
                }))
            }
            ast::Stmt::Break(_) => Stmt::Break,
            ast::Stmt::Continue(_) => Stmt::Continue,
            ast::Stmt::For {
                binding,
                iter,
                filter,
                body,
                ..
            } => return self.for_loop(binding, iter, filter.as_ref(), body),
            ast::Stmt::While { cond, body, .. } => Stmt::While {
                cond: self.expr(cond),
                body: self.block(body),
            },
            ast::Stmt::Expr(e) => Stmt::Expr(self.expr(e)),
        })
    }

    fn current_ret(&mut self) -> Ty {
        self.ctx().ret.clone()
    }

    fn for_loop(
        &mut self,
        binding: &ast::Ident,
        iter: &ast::Expr,
        filter: Option<&ast::Expr>,
        body: &ast::Block,
    ) -> Option<Stmt> {
        if let E::Binary {
            op: BinaryOp::Range,
            lhs,
            rhs,
        } = &iter.kind
        {
            let lo = self.expr(lhs);
            let hi = self.expr(rhs);
            self.push_scope();
            let int = Ty::Adt(self.prims.int, Vec::new());
            let ty = self.binding_ty(binding.span).unwrap_or(int);
            let var = self.new_local(&binding.name, ty, false);
            self.bind(&binding.name, var);
            let filter = filter.map(|f| self.expr(f));
            let body = self.block(body);
            self.pop_scope();
            return Some(Stmt::ForRange {
                var,
                lo,
                hi,
                filter,
                body,
            });
        }
        let it = self.ty(iter);
        let elem = match &it {
            Ty::Adt(id, args) if *id == self.prims.list || *id == self.prims.set => args[0].clone(),
            _ => {
                self.unsupported(iter.span, "looping over this type");
                return None;
            }
        };
        let iter = self.expr(iter);
        self.push_scope();
        let ty = self.binding_ty(binding.span).unwrap_or(elem);
        let var = self.new_local(&binding.name, ty, false);
        self.bind(&binding.name, var);
        let filter = filter.map(|f| self.expr(f));
        let body = self.block(body);
        self.pop_scope();
        Some(Stmt::ForEach {
            var,
            iter,
            filter,
            body,
        })
    }

    fn assign(&mut self, target: &ast::Expr, op: AssignOp, value: &ast::Expr) -> Option<Stmt> {
        let op = match op {
            AssignOp::Set => None,
            AssignOp::Add => Some(ArithOp::Add),
            AssignOp::Sub => Some(ArithOp::Sub),
            AssignOp::Mul => Some(ArithOp::Mul),
            AssignOp::Div => Some(ArithOp::Div),
        };
        let tt = self.ty(target);
        let (place, value) = match &target.kind {
            E::Ident(name) if self.lookup(name).is_none() && self.store_ids.contains_key(name) => {
                let id = self.store_ids[name];
                let st = self.stores[id].ty.clone();
                let v = self.expr(value);
                let v = coerce(v, &st);
                let v = match op {
                    None => v,
                    Some(op) => {
                        let old = self.store_read(id, target.span);
                        mk(
                            ExprKind::Arith(op, Box::new(old), Box::new(v)),
                            st.clone(),
                            target.span,
                        )
                    }
                };
                let opt = Ty::Opt(Box::new(st));
                return Some(Stmt::SetStore(id, coerce(v, &opt)));
            }
            E::Ident(name) => {
                let Some(local) = self.lookup(name) else {
                    self.unsupported(target.span, "assigning to this name");
                    return None;
                };
                let lt = self.ctx().locals[local].ty.clone();
                let v = self.expr(value);
                (Place::Local(local), coerce(v, &lt))
            }
            E::Field { expr: base, name } => {
                let bt = self.ty(base);
                let Some((field, _)) = self.field_decl(&bt, &name.name) else {
                    self.unsupported(target.span, "assigning to this field");
                    return None;
                };
                let base = self.expr(base);
                let v = self.expr(value);
                (
                    Place::Field {
                        base,
                        field,
                        ty: tt.clone(),
                    },
                    coerce(v, &tt),
                )
            }
            E::Index { expr: base, index } => {
                let bt = self.ty(base);
                match &bt {
                    Ty::Adt(id, targs) if *id == self.prims.map => {
                        let map = self.expr(base);
                        let k = self.expr(index);
                        let key = coerce(k, &targs[0]);
                        let v = self.expr(value);
                        (Place::MapEntry { map, key }, coerce(v, &targs[1]))
                    }
                    Ty::Adt(id, _) if *id == self.prims.list => {
                        let list = self.expr(base);
                        let index = self.expr(index);
                        let v = self.expr(value);
                        (
                            Place::ListItem {
                                list,
                                index,
                                ty: tt.clone(),
                            },
                            coerce(v, &tt),
                        )
                    }
                    _ => {
                        self.unsupported(target.span, "assigning into this value");
                        return None;
                    }
                }
            }
            _ => {
                self.unsupported(target.span, "assigning to this expression");
                return None;
            }
        };
        Some(Stmt::Assign { place, op, value })
    }

    // ----- expressions -----

    fn expr(&mut self, e: &ast::Expr) -> Expr {
        if let Some(style) = self.p.styles.get(&e.span) {
            return self.style(e, style.clone());
        }
        let ty = self.ty(e);
        let span = e.span;
        let kind = match &e.kind {
            E::Int(s) => {
                let digits: String = s.chars().filter(|c| *c != '_').collect();
                match digits.parse::<i64>() {
                    Ok(n) => ExprKind::Int(n),
                    Err(_) => return self.unsupported(span, "an integer this large"),
                }
            }
            E::Float(s) => {
                let digits: String = s.chars().filter(|c| *c != '_').collect();
                ExprKind::Float(digits.parse().unwrap_or(0.0))
            }
            E::Duration(s) => match duration_ns(s) {
                Some(ns) => ExprKind::Int(ns),
                None => return self.unsupported(span, "a duration this long"),
            },
            E::Str(pieces) => {
                let string = Ty::Adt(self.prims.string, Vec::new());
                let mut parts: Vec<Expr> = pieces
                    .iter()
                    .map(|piece| match piece {
                        StrPiece::Text(t) => mk(ExprKind::Str(t.clone()), string.clone(), span),
                        StrPiece::Expr(inner) => {
                            let v = self.expr(inner);
                            let s = inner.span;
                            mk(ExprKind::Show(Box::new(v)), string.clone(), s)
                        }
                    })
                    .collect();
                match parts.len() {
                    0 => ExprKind::Str(String::new()),
                    1 => return parts.remove(0),
                    _ => ExprKind::Concat(parts),
                }
            }
            E::Bool(b) => ExprKind::Bool(*b),
            E::None => ExprKind::None,
            E::Ident(name) => return self.ident(name, ty, span),
            E::List(items) => {
                let elem = match &ty {
                    Ty::Adt(_, args) if !args.is_empty() => args[0].clone(),
                    _ => Ty::Unknown,
                };
                ExprKind::List(
                    items
                        .iter()
                        .map(|item| {
                            let v = self.expr(item);
                            coerce(v, &elem)
                        })
                        .collect(),
                )
            }
            E::Map(entries) => {
                let (kt, vt) = match &ty {
                    Ty::Adt(_, args) if args.len() == 2 => (args[0].clone(), args[1].clone()),
                    _ => (Ty::Unknown, Ty::Unknown),
                };
                let map = self.new_local("map", ty.clone(), false);
                let mut stmts = vec![Stmt::Let(map, mk(ExprKind::NewMap, ty.clone(), span))];
                for (k, v) in entries {
                    let key = self.expr(k);
                    let value = self.expr(v);
                    stmts.push(Stmt::Assign {
                        place: Place::MapEntry {
                            map: mk(ExprKind::Local(map), ty.clone(), span),
                            key: coerce(key, &kt),
                        },
                        op: None,
                        value: coerce(value, &vt),
                    });
                }
                ExprKind::Block(Block {
                    stmts,
                    value: Some(Box::new(mk(ExprKind::Local(map), ty.clone(), span))),
                })
            }
            E::Spread(_) => return self.unsupported(span, "`..` here"),
            E::Record(fields) if fields.first().is_some_and(|f| f.name.name == "..") => {
                let Some(E::Spread(b)) = fields[0].value.as_ref().map(|v| &v.kind) else {
                    return self.unsupported(span, "this record");
                };
                let Ty::Record(fs) = ty.clone() else {
                    return self.unsupported(span, "this record");
                };
                let mut stmts = Vec::new();
                let v = self.expr(b);
                let base = self.hoist(v, &mut stmts);
                let values = fs
                    .iter()
                    .enumerate()
                    .map(|(i, (name, fty))| {
                        match fields[1..].iter().find(|f| f.name.name == *name) {
                            Some(f) => {
                                let v = match &f.value {
                                    Some(v) => self.expr(v),
                                    None => self.ident(&f.name.name, fty.clone(), f.name.span),
                                };
                                coerce(v, &Ty::Any)
                            }
                            None => mk(
                                ExprKind::Field {
                                    base: Box::new(reread(&base)),
                                    field: i,
                                },
                                Ty::Any,
                                span,
                            ),
                        }
                    })
                    .collect();
                ExprKind::Block(Block {
                    stmts,
                    value: Some(Box::new(mk(ExprKind::Record(values), ty.clone(), span))),
                })
            }
            E::Record(fields) => {
                let tys: Vec<Ty> = match &ty {
                    Ty::Record(fs) => fs.iter().map(|f| f.1.clone()).collect(),
                    _ => Vec::new(),
                };
                let values = fields
                    .iter()
                    .zip(tys)
                    .map(|(f, fty)| {
                        let v = match &f.value {
                            Some(v) => self.expr(v),
                            None => self.ident(&f.name.name, fty, f.name.span),
                        };
                        coerce(v, &Ty::Any)
                    })
                    .collect();
                ExprKind::Record(values)
            }
            E::Unary { op, expr } => {
                let v = Box::new(self.expr(expr));
                match op {
                    UnaryOp::Not => ExprKind::Not(v),
                    UnaryOp::Neg => ExprKind::Neg(v),
                }
            }
            E::Binary { op, lhs, rhs } => return self.binary(*op, lhs, rhs, ty, span),
            E::Call { .. } => return self.call(e).0,
            E::Field { expr: base, name } => return self.field(base, name, ty, span),
            E::Index { expr: base, index } => {
                let bt = self.ty(base);
                match &bt {
                    Ty::Adt(id, args) if *id == self.prims.list => {
                        let elem = args[0].clone();
                        let list = Box::new(self.expr(base));
                        let index = Box::new(self.expr(index));
                        return mk(ExprKind::ListItem { list, index }, elem, span);
                    }
                    Ty::Adt(id, args) if *id == self.prims.map => {
                        let key_ty = args[0].clone();
                        let map = Box::new(self.expr(base));
                        let k = self.expr(index);
                        let key = Box::new(coerce(k, &key_ty));
                        ExprKind::MapGet { map, key }
                    }
                    _ => return self.unsupported(span, "indexing into this value"),
                }
            }
            E::FieldShorthand(name) => {
                if let Ty::Adt(id, _) = &ty {
                    if let AdtKind::Enum(variants) = &self.p.adts[*id].kind {
                        let index = variants
                            .iter()
                            .position(|v| v.name == name.name)
                            .unwrap_or(0);
                        let kind = ExprKind::Variant {
                            adt: *id,
                            index,
                            fields: Vec::new(),
                        };
                        return mk(kind, ty, span);
                    }
                }
                return self.shorthand(name, ty, span);
            }
            E::Closure { params, body, .. } => return self.closure(params, body, ty, span),
            E::If {
                cond,
                then,
                otherwise,
            } => return self.if_chain(cond, then, otherwise.as_deref(), ty, span),
            E::Block(b) => ExprKind::Block(self.block(b)),
            E::Match { scrutinee, arms } => {
                let st = self.ty(scrutinee);
                let scrutinee = Box::new(self.expr(scrutinee));
                let arms = arms
                    .iter()
                    .map(|arm| {
                        self.push_scope();
                        let pattern = self.pattern(&arm.pattern, &st);
                        let guard = arm.guard.as_ref().map(|g| self.expr(g));
                        let body = self.expr(&arm.body);
                        let body = coerce_value(body, &ty);
                        self.pop_scope();
                        Arm {
                            pattern,
                            guard,
                            body,
                        }
                    })
                    .collect();
                ExprKind::Match { scrutinee, arms }
            }
            E::Try(inner) => {
                let (call, throws) = self.call(inner);
                let Some(err) = throws else { return call };
                let wrap = self.wraps(&err);
                ExprKind::Try {
                    expr: Box::new(call),
                    wrap,
                }
            }
            E::Catch {
                expr,
                binding,
                handler,
            } => {
                let (call, throws) = self.call(expr);
                let err = throws.unwrap_or(Ty::Unknown);
                self.push_scope();
                let binding = binding.as_ref().map(|b| {
                    let local = self.new_local(&b.name, err.clone(), false);
                    self.bind(&b.name, local);
                    local
                });
                let handler = self.block(handler);
                let handler = coerce_block(handler, &ty);
                self.pop_scope();
                ExprKind::Catch {
                    expr: Box::new(call),
                    err,
                    binding,
                    handler,
                }
            }
            E::Throw(value) => {
                let v = self.expr(value);
                let wrap = self.wraps(&v.ty);
                ExprKind::Throw {
                    value: Box::new(v),
                    wrap,
                }
            }
            E::Is { expr, pattern } => {
                let v = self.expr(expr);
                let vt = v.ty.clone();
                let pattern = Box::new(self.pattern(pattern, &vt));
                ExprKind::Is {
                    value: Box::new(v),
                    pattern,
                }
            }
            E::Error => ExprKind::Error,
        };
        mk(kind, ty, span)
    }

    fn ident(&mut self, name: &str, ty: Ty, span: Span) -> Expr {
        if let Some(local) = self.lookup(name) {
            return mk(ExprKind::Local(local), ty, span);
        }
        if let Some(&(func, _)) = self.fns.get(name) {
            let sig = self.p.global_fns[name].clone();
            let template = Template::Fn(name.to_string());
            let (func, _) = self.instance(func, template, &sig, span, None);
            return mk(ExprKind::FuncRef(func), ty, span);
        }
        if let Some(&id) = self.global_ids.get(name) {
            return mk(ExprKind::Global(id), ty, span);
        }
        if let Some(&id) = self.store_ids.get(name) {
            return coerce(self.store_read(id, span), &ty);
        }
        self.unsupported(span, format!("using `{name}` as a value"))
    }

    fn field(&mut self, base: &ast::Expr, name: &ast::Ident, ty: Ty, span: Span) -> Expr {
        if let E::Ident(n) = &base.kind {
            if self.lookup(n).is_none() {
                if let Some(module) = self.p.imports.get(n) {
                    let what = format!("`{module}.{}`", name.name);
                    return self.unsupported(span, what);
                }
                if let Some(&id) = self.p.global_types.get(n) {
                    if let AdtKind::Enum(variants) = &self.p.adts[id].kind {
                        let index = variants
                            .iter()
                            .position(|v| v.name == name.name)
                            .unwrap_or(0);
                        let kind = ExprKind::Variant {
                            adt: id,
                            index,
                            fields: Vec::new(),
                        };
                        return mk(kind, ty, span);
                    }
                }
            }
        }
        let bt = self.ty(base);
        let Some((field, decl)) = self.field_decl(&bt, &name.name) else {
            return self.unsupported(span, "this field access");
        };
        let base = Box::new(self.expr(base));
        coerce(mk(ExprKind::Field { base, field }, decl, span), &ty)
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        lhs: &ast::Expr,
        rhs: &ast::Expr,
        ty: Ty,
        span: Span,
    ) -> Expr {
        let kind = match op {
            BinaryOp::And | BinaryOp::Or => {
                let l = Box::new(self.expr(lhs));
                let r = Box::new(self.expr(rhs));
                if op == BinaryOp::And {
                    ExprKind::And(l, r)
                } else {
                    ExprKind::Or(l, r)
                }
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let l = self.expr(lhs);
                let r = self.expr(rhs);
                let eq = if matches!(rhs.kind, E::None) {
                    ExprKind::IsNone(Box::new(l))
                } else if matches!(lhs.kind, E::None) {
                    ExprKind::IsNone(Box::new(r))
                } else {
                    let (l, r) = match (&l.ty, &r.ty) {
                        (Ty::Opt(_), other) if !matches!(other, Ty::Opt(_)) => {
                            let t = l.ty.clone();
                            (l, coerce(r, &t))
                        }
                        (other, Ty::Opt(_)) if !matches!(other, Ty::Opt(_)) => {
                            let t = r.ty.clone();
                            (coerce(l, &t), r)
                        }
                        _ => (l, r),
                    };
                    ExprKind::Eq(Box::new(l), Box::new(r))
                };
                if op == BinaryOp::Eq {
                    eq
                } else {
                    ExprKind::Not(Box::new(mk(eq, ty.clone(), span)))
                }
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let c = match op {
                    BinaryOp::Lt => CmpOp::Lt,
                    BinaryOp::Le => CmpOp::Le,
                    BinaryOp::Gt => CmpOp::Gt,
                    _ => CmpOp::Ge,
                };
                ExprKind::Cmp(c, Box::new(self.expr(lhs)), Box::new(self.expr(rhs)))
            }
            BinaryOp::Coalesce => {
                let l = Box::new(self.expr(lhs));
                let r = self.expr(rhs);
                ExprKind::Coalesce(l, Box::new(coerce_value(r, &ty)))
            }
            BinaryOp::Range => ExprKind::Range(Box::new(self.expr(lhs)), Box::new(self.expr(rhs))),
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                let a = match op {
                    BinaryOp::Add => ArithOp::Add,
                    BinaryOp::Sub => ArithOp::Sub,
                    BinaryOp::Mul => ArithOp::Mul,
                    BinaryOp::Div => ArithOp::Div,
                    _ => ArithOp::Rem,
                };
                ExprKind::Arith(a, Box::new(self.expr(lhs)), Box::new(self.expr(rhs)))
            }
        };
        mk(kind, ty, span)
    }

    fn if_chain(
        &mut self,
        cond: &ast::Expr,
        then: &ast::Block,
        otherwise: Option<&ast::Expr>,
        ty: Ty,
        span: Span,
    ) -> Expr {
        // Names bound by `is` in the condition are visible in `then` only.
        self.push_scope();
        let c = self.expr(cond);
        let then = self.block(then);
        let then = coerce_block(then, &ty);
        self.pop_scope();
        let otherwise = otherwise.map(|other| match &other.kind {
            E::If {
                cond,
                then,
                otherwise,
            } => {
                let nested =
                    self.if_chain(cond, then, otherwise.as_deref(), ty.clone(), other.span);
                Block {
                    stmts: Vec::new(),
                    value: Some(Box::new(nested)),
                }
            }
            E::Block(b) => {
                let b = self.block(b);
                coerce_block(b, &ty)
            }
            _ => {
                let v = self.expr(other);
                Block {
                    stmts: Vec::new(),
                    value: Some(Box::new(coerce_value(v, &ty))),
                }
            }
        });
        let kind = ExprKind::If {
            cond: Box::new(c),
            then,
            otherwise,
        };
        mk(kind, ty, span)
    }

    fn pattern(&mut self, p: &ast::Pattern, ty: &Ty) -> Pattern {
        match &p.kind {
            PatternKind::Wildcard | PatternKind::Error => Pattern::Wildcard,
            PatternKind::Binding(name) => {
                let local = self.new_local(name, ty.clone(), false);
                self.bind(name, local);
                Pattern::Bind(local)
            }
            PatternKind::Literal(lit) => {
                let l = self.expr(lit);
                let l = if matches!(ty, Ty::Opt(_)) && !matches!(l.ty, Ty::Opt(_)) {
                    coerce(l, ty)
                } else {
                    l
                };
                Pattern::Literal(l)
            }
            PatternKind::None => Pattern::None,
            PatternKind::Variant { path, args } => {
                let last = &path[path.len() - 1].name;
                if path.len() == 1 && last == "some" {
                    let Ty::Opt(inner) = ty else {
                        return Pattern::Wildcard;
                    };
                    let sub = match args.first() {
                        Some(a) => self.pattern(a, inner),
                        None => Pattern::Wildcard,
                    };
                    return Pattern::Some(Box::new(sub), (**inner).clone());
                }
                let Ty::Adt(id, targs) = ty else {
                    return Pattern::Wildcard;
                };
                let AdtKind::Enum(variants) = &self.p.adts[*id].kind else {
                    return Pattern::Wildcard;
                };
                let Some(index) = variants.iter().position(|v| v.name == *last) else {
                    return Pattern::Wildcard;
                };
                let map = self.type_map(*id, targs);
                let field_tys: Vec<Ty> = variants[index]
                    .fields
                    .iter()
                    .map(|(_, t)| substitute(t, &map))
                    .collect();
                let fields = args
                    .iter()
                    .zip(field_tys)
                    .map(|(a, fty)| (self.pattern(a, &fty), fty))
                    .collect();
                Pattern::Variant {
                    adt: *id,
                    index,
                    fields,
                }
            }
        }
    }

    fn closure(&mut self, params: &[ast::Param], body: &ast::Block, ty: Ty, span: Span) -> Expr {
        let Ty::Fn(fty) = &ty else {
            return self.unsupported(span, "this function value");
        };
        let fty = fty.clone();
        let id = self.reserve();
        self.enter(true, fty.ret.clone(), fty.throws.clone());
        let params = params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let pty = self
                    .binding_ty(p.name.span)
                    .or_else(|| fty.params.get(i).cloned())
                    .unwrap_or(Ty::Unknown);
                let local = self.new_local(&p.name.name, pty, false);
                self.bind(&p.name.name, local);
                local
            })
            .collect();
        let body = self.block(body);
        let body = coerce_block(body, &fty.ret);
        let captures = self.finish(
            id,
            "<closure>".into(),
            FuncKind::Closure,
            params,
            fty.ret.clone(),
            body,
        );
        mk(ExprKind::Closure { func: id, captures }, ty, span)
    }

    /// `.title` or `.upper`: a closure of one parameter.
    fn shorthand(&mut self, name: &ast::Ident, ty: Ty, span: Span) -> Expr {
        let Ty::Fn(fty) = &ty else {
            return self.unsupported(span, "this field shorthand");
        };
        let fty = fty.clone();
        let Some(param) = fty.params.first().cloned() else {
            return self.unsupported(span, "this field shorthand");
        };
        let id = self.reserve();
        self.enter(true, fty.ret.clone(), None);
        let it = self.new_local("it", param.clone(), false);
        let recv = mk(ExprKind::Local(it), param.clone(), span);
        let value = match self.field_decl(&param, &name.name) {
            Some((field, decl)) => {
                let f = mk(
                    ExprKind::Field {
                        base: Box::new(recv),
                        field,
                    },
                    decl,
                    span,
                );
                coerce(f, &fty.ret)
            }
            None => match self.method_call(recv, &name.name, &[], &fty.ret, span) {
                Some((call, _)) => call,
                None => self.unsupported(span, "this field shorthand"),
            },
        };
        let body = Block {
            stmts: Vec::new(),
            value: Some(Box::new(value)),
        };
        self.finish(
            id,
            "<closure>".into(),
            FuncKind::Closure,
            vec![it],
            fty.ret.clone(),
            body,
        );
        mk(
            ExprKind::Closure {
                func: id,
                captures: Vec::new(),
            },
            ty,
            span,
        )
    }

    // ----- calls -----

    /// Lowers a call, returning it with the error type when it can fail.
    /// Any other expression is lowered as it is, and cannot fail.
    fn call(&mut self, e: &ast::Expr) -> (Expr, Option<Ty>) {
        let E::Call { callee, args, .. } = &e.kind else {
            return (self.expr(e), None);
        };
        let rt = self.ty(e);
        let span = e.span;
        // A typed `orm` query: the checker worked out its SQL, so the
        // chain that built it is not lowered at all.
        if let Some(query) = self.p.queries.get(&span).cloned() {
            return self.orm_run(&query, &rt, span);
        }
        if let (E::FieldShorthand(name), Ty::Adt(id, _)) = (&callee.kind, &rt) {
            if matches!(self.p.adts[*id].kind, AdtKind::Enum(_)) {
                return self.type_call(*id, name, args, &rt, span);
            }
        }
        match &callee.kind {
            E::Field { expr: base, name } => {
                if let E::Ident(m) = &base.kind {
                    if self.lookup(m).is_none() {
                        if let Some(module) = self.p.imports.get(m).cloned() {
                            return self.module_call(&module, name, args, &rt, span);
                        }
                        if let Some(&id) = self.p.global_types.get(m) {
                            if !self.fns.contains_key(m) {
                                return self.type_call(id, name, args, &rt, span);
                            }
                        }
                    }
                }
                if let E::Field { expr: inner, .. } = &base.kind {
                    if let E::Ident(m) = &inner.kind {
                        if self.lookup(m).is_none() && self.p.imports.contains_key(m) {
                            let what = format!("`{}.{}`", self.p.imports[m], name.name);
                            return (self.unsupported(span, what), None);
                        }
                    }
                }
                let bt = self.ty(base);
                let recv = self.expr(base);
                if self.method_sig(&bt, &name.name).is_some() {
                    // The receiver has no type when it could not be lowered.
                    return self
                        .method_call(recv, &name.name, args, &rt, span)
                        .unwrap_or_else(|| (mk(ExprKind::Error, rt.clone(), span), None));
                }
                let Some((field, decl)) = self.field_decl(&bt, &name.name) else {
                    return (self.unsupported(span, "this call"), None);
                };
                let field_ty = match &bt {
                    Ty::Adt(id, targs) => substitute(&decl, &self.type_map(*id, targs)),
                    Ty::Record(fields) => fields[field].1.clone(),
                    _ => decl.clone(),
                };
                let f = mk(
                    ExprKind::Field {
                        base: Box::new(recv),
                        field,
                    },
                    decl,
                    callee.span,
                );
                let f = coerce(f, &field_ty);
                self.call_closure(f, args, &rt, span)
            }
            E::Ident(n) if self.lookup(n).is_none() => {
                if let Some(&(func, decl)) = self.fns.get(n) {
                    let sig = self.p.global_fns[n].clone();
                    let (func, sig) =
                        self.instance(func, Template::Fn(n.clone()), &sig, span, None);
                    return self.user_call(func, &sig, decl, None, args, &rt, span);
                }
                if matches!(n.as_str(), "print" | "env" | "panic") && args.len() == 1 {
                    let a = &args[0].value;
                    let v = self.expr(a);
                    let kind = match n.as_str() {
                        "print" => {
                            let string = Ty::Adt(self.prims.string, Vec::new());
                            ExprKind::Print(Box::new(mk(
                                ExprKind::Show(Box::new(v)),
                                string,
                                a.span,
                            )))
                        }
                        "env" => ExprKind::Env(Box::new(v)),
                        _ => ExprKind::Panic(Box::new(v)),
                    };
                    return (mk(kind, rt, span), None);
                }
                if let Some(&id) = self.p.global_types.get(n) {
                    return (self.construct(id, args, rt, span), None);
                }
                let c = self.expr(callee);
                self.call_closure(c, args, &rt, span)
            }
            _ => {
                let c = self.expr(callee);
                self.call_closure(c, args, &rt, span)
            }
        }
    }

    /// Lowers the arguments of a call into the parameters they fill. The
    /// arguments are evaluated in source order: when named arguments are
    /// out of order, they (and `lead`, the receiver, which goes first) are
    /// stored in locals before the call.
    fn arrange(
        &mut self,
        params: &[ParamSig],
        defaults: &[Option<&ast::Expr>],
        lead: Option<Expr>,
        args: &[ast::Arg],
    ) -> (Vec<Stmt>, Option<Expr>, Vec<Expr>) {
        // Lowering the arguments can arrange other calls.
        let call_span = self.call_span;
        let mut slots: Vec<Option<Expr>> = (0..params.len()).map(|_| None).collect();
        let mut order = Vec::new();
        let mut next = 0;
        let mut lowered = Vec::new();
        for arg in args {
            let index = match &arg.name {
                Some(name) => params
                    .iter()
                    .position(|p| p.name.as_deref() == Some(&name.name)),
                None => {
                    while next < params.len() && (slots[next].is_some() || order.contains(&next)) {
                        next += 1;
                    }
                    (next < params.len()).then_some(next)
                }
            };
            let v = self.expr(&arg.value);
            if let Some(i) = index {
                order.push(i);
                lowered.push((i, coerce(v, &params[i].ty)));
            }
        }
        let in_order = order.windows(2).all(|w| w[0] < w[1]);
        let mut prelude = Vec::new();
        let mut lead = lead;
        if in_order {
            for (i, v) in lowered {
                slots[i] = Some(v);
            }
        } else {
            if let Some(l) = lead.take() {
                lead = Some(self.hoist(l, &mut prelude));
            }
            for (i, v) in lowered {
                slots[i] = Some(self.hoist(v, &mut prelude));
            }
        }
        let mut out = Vec::new();
        for (i, slot) in slots.into_iter().enumerate() {
            let v = match slot {
                Some(v) => v,
                None => match defaults.get(i).copied().flatten() {
                    Some(d) if is_caller_default(d) => mk(
                        ExprKind::Str(caller_location(call_span.start)),
                        params[i].ty.clone(),
                        call_span,
                    ),
                    Some(d) => {
                        let v = self.expr(d);
                        coerce(v, &params[i].ty)
                    }
                    None => mk(ExprKind::Error, params[i].ty.clone(), Span::new(0, 0)),
                },
            };
            out.push(v);
        }
        (prelude, lead, out)
    }

    /// Stores a value in a new local, returning a read of it.
    fn hoist(&mut self, v: Expr, prelude: &mut Vec<Stmt>) -> Expr {
        let (ty, span) = (v.ty.clone(), v.span);
        let local = self.new_local("arg", ty.clone(), false);
        prelude.push(Stmt::Let(local, v));
        mk(ExprKind::Local(local), ty, span)
    }

    #[allow(clippy::too_many_arguments)]
    fn user_call(
        &mut self,
        func: FuncId,
        sig: &FnSig,
        decl: &ast::FnDecl,
        recv: Option<Expr>,
        args: &[ast::Arg],
        rt: &Ty,
        span: Span,
    ) -> (Expr, Option<Ty>) {
        let defaults: Vec<Option<&ast::Expr>> = decl
            .params
            .iter()
            .skip(usize::from(sig.has_self))
            .map(|p| p.default.as_ref())
            .collect();
        self.call_span = span;
        let (prelude, recv, mut vals) = self.arrange(&sig.params, &defaults, recv, args);
        if let Some(r) = recv {
            vals.insert(0, r);
        }
        let call = mk(ExprKind::Call { func, args: vals }, sig.ret.clone(), span);
        let call = coerce_value(call, rt);
        (with_prelude(prelude, call), sig.throws.clone())
    }

    /// `Type.function(...)` or `Enum.Variant(...)`.
    fn type_call(
        &mut self,
        id: AdtId,
        name: &ast::Ident,
        args: &[ast::Arg],
        rt: &Ty,
        span: Span,
    ) -> (Expr, Option<Ty>) {
        if let Some(&(func, decl)) = self.methods.get(&(id, name.name.clone())) {
            let sig = self.p.adts[id]
                .methods
                .iter()
                .find(|m| m.name == name.name)
                .cloned();
            if let Some(sig) = sig.filter(|s| !s.has_self) {
                let template = Template::Method(id, name.name.clone());
                let (func, sig) = self.instance(func, template, &sig, span, None);
                return self.user_call(func, &sig, decl, None, args, rt, span);
            }
        }
        let AdtKind::Enum(variants) = &self.p.adts[id].kind else {
            return (self.unsupported(span, "this call"), None);
        };
        let Some(index) = variants.iter().position(|v| v.name == name.name) else {
            return (self.unsupported(span, "this call"), None);
        };
        let params: Vec<ParamSig> = variants[index]
            .fields
            .iter()
            .map(|(n, t)| ParamSig {
                name: n.clone(),
                ty: t.clone(),
                has_default: false,
            })
            .collect();
        let (prelude, _, fields) = self.arrange(&params, &[], None, args);
        let v = mk(
            ExprKind::Variant {
                adt: id,
                index,
                fields,
            },
            rt.clone(),
            span,
        );
        (with_prelude(prelude, v), None)
    }

    /// `Post(title: "Hi", ...)`, `Map()` or `Set()`.
    fn construct(&mut self, id: AdtId, args: &[ast::Arg], rt: Ty, span: Span) -> Expr {
        if id == self.prims.map || id == self.prims.set {
            return mk(ExprKind::NewMap, rt, span);
        }
        let fields = match &self.p.adts[id].kind {
            AdtKind::Struct(fields) => fields.clone(),
            AdtKind::Enum(_) => return self.unsupported(span, "this call"),
        };
        let builtin =
            id < self.p.std_adts && self.p.global_types.get(&self.p.adts[id].name) == Some(&id);
        if fields.is_empty() && builtin {
            let what = format!("creating a `{}`", self.p.adts[id].name);
            return self.unsupported(span, what);
        }
        let decl = self.structs.get(&id).copied();
        // A struct of the standard library is declared in a file the
        // lowering never sees, so a field left out of one has no default
        // to fall back on here.
        if decl.is_none() {
            let left_out = fields.iter().enumerate().find(|(i, f)| {
                f.has_default
                    && !args.iter().enumerate().any(|(k, a)| match &a.name {
                        Some(n) => n.name == f.name,
                        None => k == *i,
                    })
            });
            if let Some((_, f)) = left_out {
                let what = format!("a `{}` without `{}`", self.p.adts[id].name, f.name);
                return self.unsupported(span, what);
            }
        }
        // `Post(..post, title: t)`: `post` first, then the fields given;
        // the others are read from it.
        let mut spread_prelude = Vec::new();
        let spread = args.iter().find_map(|a| match &a.value.kind {
            E::Spread(base) if a.name.is_none() => Some(base),
            _ => None,
        });
        let base = spread.map(|b| {
            let v = self.expr(b);
            self.hoist(v, &mut spread_prelude)
        });
        let own: Vec<ast::Arg> = args
            .iter()
            .filter(|a| !(a.name.is_none() && matches!(a.value.kind, E::Spread(_))))
            .cloned()
            .collect();
        let args = &own[..];
        let defaults: Vec<Option<&ast::Expr>> = fields
            .iter()
            .map(|f| {
                if base.is_some() {
                    return None;
                }
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
        let (mut prelude, _, mut vals) = self.arrange(&params, &defaults, None, args);
        if let Some(base) = base {
            for (i, f) in fields.iter().enumerate() {
                let given = args
                    .iter()
                    .any(|a| a.name.as_ref().is_some_and(|n| n.name == f.name));
                if !given {
                    let read = ExprKind::Field {
                        base: Box::new(reread(&base)),
                        field: i,
                    };
                    vals[i] = mk(read, f.ty.clone(), span);
                }
            }
            spread_prelude.append(&mut prelude);
            prelude = spread_prelude;
        }
        let s = mk(
            ExprKind::Struct {
                adt: id,
                fields: vals,
            },
            rt,
            span,
        );
        with_prelude(prelude, s)
    }

    fn call_closure(
        &mut self,
        callee: Expr,
        args: &[ast::Arg],
        rt: &Ty,
        span: Span,
    ) -> (Expr, Option<Ty>) {
        let Ty::Fn(f) = callee.ty.clone() else {
            return (self.unsupported(span, "this call"), None);
        };
        let params: Vec<ParamSig> = f
            .params
            .iter()
            .map(|ty| ParamSig {
                name: None,
                ty: ty.clone(),
                has_default: false,
            })
            .collect();
        let (prelude, callee, vals) = self.arrange(&params, &[], Some(callee), args);
        let callee = Box::new(callee.expect("the callee is kept"));
        let call = mk(
            ExprKind::CallClosure { callee, args: vals },
            f.ret.clone(),
            span,
        );
        let call = coerce_value(call, rt);
        (with_prelude(prelude, call), f.throws.clone())
    }

    fn method_sig(&self, recv_ty: &Ty, name: &str) -> Option<FnSig> {
        let Ty::Adt(id, _) = recv_ty else { return None };
        self.p.adts[*id]
            .methods
            .iter()
            .find(|m| m.name == name)
            .cloned()
    }

    /// A method call on a value, or `None` if `name` is not a method.
    fn method_call(
        &mut self,
        recv: Expr,
        name: &str,
        args: &[ast::Arg],
        rt: &Ty,
        span: Span,
    ) -> Option<(Expr, Option<Ty>)> {
        let sig = self.method_sig(&recv.ty, name)?;
        let Ty::Adt(id, _) = recv.ty else { return None };
        if let Some(&(func, decl)) = self.methods.get(&(id, name.to_string())) {
            let template = Template::Method(id, name.to_string());
            let recv_ty = recv.ty.clone();
            let (func, sig) = self.instance(func, template, &sig, span, Some(&recv_ty));
            return Some(self.user_call(func, &sig, decl, Some(recv), args, rt, span));
        }
        if let Some(type_name) = self.lib_type(id) {
            return Some(self.lib_method(&type_name, recv, &sig, name, args, rt, span));
        }
        // A built-in method takes items of a type parameter in slots. Move
        // each argument to the receiver's item type first, so that, say,
        // an `Int` pushed onto a `List<Int?>` is boxed before it goes in.
        let concrete: Vec<ParamSig> = match &recv.ty {
            Ty::Adt(id, targs) => {
                let map = self.type_map(*id, targs);
                sig.params
                    .iter()
                    .map(|p| ParamSig {
                        ty: match &p.ty {
                            Ty::Param(_) => substitute(&p.ty, &map),
                            _ => p.ty.clone(),
                        },
                        ..p.clone()
                    })
                    .collect()
            }
            _ => sig.params.clone(),
        };
        let (prelude, recv, vals) = self.arrange(&concrete, &[], Some(recv), args);
        let vals = vals
            .into_iter()
            .zip(&sig.params)
            .map(|(v, p)| coerce(v, &p.ty))
            .collect();
        // `Bytes` methods, and `String.bytes`, are in the Rust runtime.
        let full = format!("{}.{name}", self.p.adts[id].name);
        if (id == self.prims.bytes || id == self.prims.string)
            && crate::rt::rt_fn("prelude", &full).is_some()
        {
            let mut all = vec![recv.expect("the receiver is kept")];
            all.extend(vals);
            let call = mk(
                ExprKind::Lib {
                    module: "prelude".into(),
                    name: full,
                    args: all,
                },
                sig.ret.clone(),
                span,
            );
            let call = coerce_value(call, rt);
            return Some((with_prelude(prelude, call), sig.throws.clone()));
        }
        let recv = Box::new(recv.expect("the receiver is kept"));
        let call = mk(
            ExprKind::Method {
                name: name.to_string(),
                recv,
                args: vals,
            },
            sig.ret.clone(),
            span,
        );
        let call = coerce_value(call, rt);
        Some((with_prelude(prelude, call), None))
    }

    /// For a type in [`LIB_TYPES`], its name.
    fn lib_type(&self, id: AdtId) -> Option<String> {
        LIB_TYPES.iter().find_map(|(module, ty)| {
            let m = self.p.modules.get(*module)?;
            (m.types.get(*ty) == Some(&id)).then(|| ty.to_string())
        })
    }

    /// A method of a [`LIB_TYPES`] type, such as `app.get(...)`.
    #[allow(clippy::too_many_arguments)]
    fn lib_method(
        &mut self,
        type_name: &str,
        recv: Expr,
        sig: &FnSig,
        name: &str,
        args: &[ast::Arg],
        rt: &Ty,
        span: Span,
    ) -> (Expr, Option<Ty>) {
        if type_name == "Event" && name == "form" {
            return self.event_form(recv, span);
        }
        if type_name == "Db" {
            if name == "transaction" {
                return self.sql_transaction(recv, sig, args, rt, span);
            }
            if let Some(k) = sig.params.iter().position(|p| p.ty == Ty::Rest) {
                return self.sql_call(recv, sig, name, k, args, rt, span);
            }
        }
        let (prelude, recv, mut vals) = self.arrange(&sig.params, &[], Some(recv), args);
        let module = LIB_TYPES
            .iter()
            .find(|(_, t)| *t == type_name)
            .map_or("", |(m, _)| *m);
        self.std_defaults(module, Some(type_name), name, &sig.params, &mut vals, span);
        // A route handler keeps its own type: its parameters come from the
        // route pattern, not from the `Rest` in the declared type.
        let vals = vals.into_iter().map(|v| match v.kind {
            ExprKind::Coerce(inner) if matches!(inner.ty, Ty::Fn(_)) => *inner,
            kind => Expr { kind, ..v },
        });
        let mut all = vec![recv.expect("the receiver is kept")];
        all.extend(vals);
        let call = mk(
            ExprKind::Lib {
                module: module.to_string(),
                name: format!("{type_name}.{name}"),
                args: all,
            },
            lib_ret(sig, self.p.instantiations.get(&span)),
            span,
        );
        let call = coerce_value(call, rt);
        (with_prelude(prelude, call), sig.throws.clone())
    }

    /// `event.form<T>()`: a call of the reader the compiler wrote for `T`
    /// (see [`crate::forms`]) with the event's fields.
    fn event_form(&mut self, recv: Expr, span: Span) -> (Expr, Option<Ty>) {
        let target = self
            .p
            .instantiations
            .get(&span)
            .and_then(|m| m.get("T"))
            .cloned();
        let Some(Ty::Adt(id, _)) = target else {
            return (
                self.unsupported(span, "`event.form` of a type that is not a struct"),
                None,
            );
        };
        let fname = crate::forms::reader(&self.p.adts[id].name);
        let Some(&(func, _)) = self.fns.get(&fname) else {
            return (self.unsupported(span, "`event.form` of this type"), None);
        };
        let Ty::Adt(event, _) = recv.ty.clone() else {
            return (self.unsupported(span, "`event.form`"), None);
        };
        let AdtKind::Struct(fields) = &self.p.adts[event].kind else {
            return (self.unsupported(span, "`event.form`"), None);
        };
        let Some(index) = fields.iter().position(|f| f.name == "fields") else {
            return (self.unsupported(span, "`event.form`"), None);
        };
        let field_ty = fields[index].ty.clone();
        let fields = mk(
            ExprKind::Field {
                base: Box::new(recv),
                field: index,
            },
            field_ty,
            span,
        );
        let sig = self.p.global_fns[&fname].clone();
        let (func, sig) = self.instance(func, Template::Fn(fname), &sig, span, None);
        let call = mk(
            ExprKind::Call {
                func,
                args: vec![fields],
            },
            sig.ret.clone(),
            span,
        );
        (call, sig.throws.clone())
    }

    /// A query method of `sql.Db`, whose arguments after the first `k`
    /// fill the query's placeholders: they are passed as one
    /// `List<sql.Value>`.
    #[allow(clippy::too_many_arguments)]
    fn sql_call(
        &mut self,
        recv: Expr,
        sig: &FnSig,
        name: &str,
        k: usize,
        args: &[ast::Arg],
        rt: &Ty,
        span: Span,
    ) -> (Expr, Option<Ty>) {
        let split = k.min(args.len());
        let (prelude, recv, vals) = self.arrange(&sig.params[..k], &[], Some(recv), &args[..split]);
        let list = self.sql_params(&args[split..], span);
        let mut all = vec![recv.expect("the receiver is kept")];
        all.extend(vals);
        all.push(list);
        let call = mk(
            ExprKind::Lib {
                module: "sql".into(),
                name: format!("Db.{name}"),
                args: all,
            },
            lib_ret(sig, self.p.instantiations.get(&span)),
            span,
        );
        let call = coerce_value(call, rt);
        (with_prelude(prelude, call), sig.throws.clone())
    }

    /// The type `module.name` of the standard library.
    fn std_type(&self, module: &str, name: &str) -> Ty {
        let id = self.p.modules[module].types[name];
        Ty::Adt(id, Vec::new())
    }

    /// Query arguments as a `List<sql.Value>`, each turned into a
    /// `sql.Value` by the runtime from its type.
    fn sql_params(&mut self, args: &[ast::Arg], span: Span) -> Expr {
        let value_ty = self.std_type("sql", "Value");
        let mut items = Vec::new();
        for a in args {
            let v = self.expr(&a.value);
            if v.ty == value_ty {
                items.push(v);
                continue;
            }
            let vspan = v.span;
            items.push(mk(
                ExprKind::Lib {
                    module: "sql".into(),
                    name: "__value".into(),
                    args: vec![coerce(v, &Ty::Any)],
                },
                value_ty.clone(),
                vspan,
            ));
        }
        let list = Ty::Adt(self.prims.list, vec![value_ty]);
        mk(ExprKind::List(items), list, span)
    }

    /// `db.transaction(body)`: a call of `__sql_transaction`, made once for
    /// each type `body` returns, which begins, runs `body` with the
    /// transaction's handle, and commits, or rolls back and fails with
    /// `body`'s error.
    fn sql_transaction(
        &mut self,
        recv: Expr,
        sig: &FnSig,
        args: &[ast::Arg],
        rt: &Ty,
        span: Span,
    ) -> (Expr, Option<Ty>) {
        let Some(arg) = args.first() else {
            return (self.unsupported(span, "this transaction"), None);
        };
        let body = self.expr(&arg.value);
        let t = lib_ret(sig, self.p.instantiations.get(&span));
        let helper = self.transaction_fn(&t, &recv.ty, &body.ty, span);
        let err = Ty::Adt(self.prims.error, Vec::new());
        let call = mk(
            ExprKind::Call {
                func: helper,
                args: vec![recv, body],
            },
            t,
            span,
        );
        (coerce_value(call, rt), Some(err))
    }

    fn transaction_fn(&mut self, t: &Ty, db_ty: &Ty, body_ty: &Ty, span: Span) -> FuncId {
        let key = format!("__sql_transaction:{t:?}");
        if let Some(&id) = self.helpers.get(&key) {
            return id;
        }
        let id = self.reserve();
        self.helpers.insert(key, id);
        let err = Ty::Adt(self.prims.error, Vec::new());
        self.enter(false, t.clone(), Some(err.clone()));
        let db = self.new_local("db", db_ty.clone(), false);
        let body = self.new_local("body", body_ty.clone(), false);
        let tx = self.new_local("tx", db_ty.clone(), false);
        let e = self.new_local("err", err.clone(), false);
        let local = |l: LocalId, ty: &Ty| mk(ExprKind::Local(l), ty.clone(), span);
        let lib = |name: &str, args: Vec<Expr>, ty: Ty| {
            mk(
                ExprKind::Lib {
                    module: "sql".into(),
                    name: name.into(),
                    args,
                },
                ty,
                span,
            )
        };
        let mut stmts = Vec::new();
        let begin = lib("Db.__begin", vec![local(db, db_ty)], db_ty.clone());
        stmts.push(Stmt::Let(
            tx,
            mk(
                ExprKind::Try {
                    expr: Box::new(begin),
                    wrap: false,
                },
                db_ty.clone(),
                span,
            ),
        ));
        let run = mk(
            ExprKind::CallClosure {
                callee: Box::new(local(body, body_ty)),
                args: vec![local(tx, db_ty)],
            },
            t.clone(),
            span,
        );
        let rollback = lib("Db.__rollback", vec![local(tx, db_ty)], Ty::Unit);
        let rethrow = mk(
            ExprKind::Throw {
                value: Box::new(local(e, &err)),
                wrap: false,
            },
            Ty::Never,
            span,
        );
        let caught = mk(
            ExprKind::Catch {
                expr: Box::new(run),
                err: err.clone(),
                binding: Some(e),
                handler: Block {
                    stmts: vec![Stmt::Expr(rollback)],
                    value: Some(Box::new(rethrow)),
                },
            },
            t.clone(),
            span,
        );
        let result = if matches!(t, Ty::Unit) {
            stmts.push(Stmt::Expr(caught));
            None
        } else {
            let r = self.new_local("result", t.clone(), false);
            stmts.push(Stmt::Let(r, caught));
            Some(r)
        };
        let commit = lib("Db.__commit", vec![local(tx, db_ty)], Ty::Unit);
        stmts.push(Stmt::Expr(mk(
            ExprKind::Try {
                expr: Box::new(commit),
                wrap: false,
            },
            Ty::Unit,
            span,
        )));
        let block = Block {
            stmts,
            value: result.map(|r| Box::new(local(r, t))),
        };
        let name = format!("__sql_transaction_{}", self.helpers.len());
        self.finish(id, name, FuncKind::Fn, vec![db, body], t.clone(), block);
        id
    }

    // ----- typed queries -----

    /// Runs a query the checker compiled ([`kiln_check::Query`]): the SQL
    /// it worked out, the values it binds, and the relations to load.
    fn orm_run(&mut self, q: &kiln_check::Query, rt: &Ty, span: Span) -> (Expr, Option<Ty>) {
        let err = Ty::Adt(self.prims.error, Vec::new());
        let model = Ty::Adt(q.plan.model.adt, Vec::new());
        let list_ty = Ty::Adt(self.prims.list, vec![model.clone()]);
        let int = Ty::Adt(self.prims.int, Vec::new());
        let string = Ty::Adt(self.prims.string, Vec::new());
        let db_ty = self.std_type("sql", "Db");
        match q.terminal.as_str() {
            "insert" | "update" | "delete" if q.value.is_some() => {
                let (sql, fields) = match q.terminal.as_str() {
                    "insert" => q.plan.insert_sql(),
                    "update" => q.plan.update_sql(),
                    _ => q.plan.delete_row_sql(),
                };
                let mut prelude = Vec::new();
                let db = self.expr(&q.plan.db);
                let db = self.hoist(db, &mut prelude);
                let value = self.expr(q.value.as_ref().expect("a row"));
                let value = self.hoist(value, &mut prelude);
                let params = self.field_params(&value, &fields, span);
                let call = if q.terminal == "insert" {
                    // The key the database generates comes back in the row.
                    let key_column = q.plan.model.key_column();
                    let key = if key_column.generated {
                        key_column.field as i64
                    } else {
                        -1
                    };
                    mk(
                        ExprKind::Lib {
                            module: "orm".into(),
                            name: "__insert".into(),
                            args: vec![
                                db,
                                coerce(value, &Ty::Any),
                                mk(ExprKind::Str(sql), string, span),
                                mk(ExprKind::Int(key), int, span),
                                params,
                            ],
                        },
                        model,
                        span,
                    )
                } else {
                    self.sql_lib(
                        "Db.exec",
                        vec![db, mk(ExprKind::Str(sql), string, span), params],
                        int,
                        span,
                    )
                };
                let _ = db_ty;
                (with_prelude(prelude, coerce_value(call, rt)), Some(err))
            }
            "sql" => {
                let (sql, _) = q.plan.rows_sql();
                (mk(ExprKind::Str(sql), string, span), None)
            }
            "count" | "delete" => {
                let (sql, params) = q.sql();
                let db = self.expr(&q.plan.db);
                let values = self.sql_values(&params, span);
                let name = if q.terminal == "count" {
                    "Db.scalar"
                } else {
                    "Db.exec"
                };
                let call = self.sql_lib(
                    name,
                    vec![db, mk(ExprKind::Str(sql), string, span), values],
                    int,
                    span,
                );
                (coerce_value(call, rt), Some(err))
            }
            _ => {
                // `all`, `first` and `get`: the rows, then each relation.
                let (sql, params) = q.sql();
                let one = q.terminal != "all";
                let steps = {
                    let models = |adt: AdtId| self.p.models.iter().find(|m| m.adt == adt).cloned();
                    q.plan.include_steps(&models)
                };
                let mut prelude = Vec::new();
                let db = self.expr(&q.plan.db);
                let db = if steps.is_empty() {
                    db
                } else {
                    self.hoist(db, &mut prelude)
                };
                let values = self.sql_values(&params, span);
                if steps.is_empty() {
                    let (name, ty) = if one {
                        ("Db.one", Ty::Opt(Box::new(model)))
                    } else {
                        ("Db.query", list_ty)
                    };
                    let call = self.sql_lib(
                        name,
                        vec![db, mk(ExprKind::Str(sql), string, span), values],
                        ty,
                        span,
                    );
                    return (with_prelude(prelude, coerce_value(call, rt)), Some(err));
                }
                let rows_call = self.sql_lib(
                    "Db.query",
                    vec![
                        reread(&db),
                        mk(ExprKind::Str(sql), string.clone(), span),
                        values,
                    ],
                    list_ty.clone(),
                    span,
                );
                let rows = self.new_local("rows", list_ty.clone(), false);
                let read = || mk(ExprKind::Local(rows), list_ty.clone(), span);
                prelude.push(Stmt::Let(
                    rows,
                    mk(
                        ExprKind::Try {
                            expr: Box::new(rows_call),
                            wrap: false,
                        },
                        list_ty.clone(),
                        span,
                    ),
                ));
                for step in steps {
                    let call = mk(
                        ExprKind::Lib {
                            module: "orm".into(),
                            name: "__include".into(),
                            args: vec![
                                reread(&db),
                                coerce(read(), &Ty::Any),
                                mk(ExprKind::Int(step.field as i64), int.clone(), span),
                                mk(ExprKind::Str(step.sql), string.clone(), span),
                                mk(ExprKind::Int(step.key_field as i64), int.clone(), span),
                                mk(ExprKind::Int(step.fk_field as i64), int.clone(), span),
                                mk(ExprKind::Bool(step.list), self.bool_ty(), span),
                            ],
                        },
                        Ty::Unit,
                        span,
                    );
                    prelude.push(Stmt::Expr(mk(
                        ExprKind::Try {
                            expr: Box::new(call),
                            wrap: false,
                        },
                        Ty::Unit,
                        span,
                    )));
                }
                let value = if one {
                    // `first` and `get` read the row the query kept.
                    let row = mk(
                        ExprKind::Method {
                            name: "get".into(),
                            recv: Box::new(read()),
                            args: vec![mk(ExprKind::Int(0), int, span)],
                        },
                        Ty::Opt(Box::new(model)),
                        span,
                    );
                    coerce_value(row, rt)
                } else {
                    read()
                };
                (with_prelude(prelude, value), Some(err))
            }
        }
    }

    fn bool_ty(&self) -> Ty {
        Ty::Adt(self.prims.bool_, Vec::new())
    }

    /// A call of a `sql.Db` method with the arguments already lowered.
    fn sql_lib(&mut self, name: &str, args: Vec<Expr>, ty: Ty, span: Span) -> Expr {
        mk(
            ExprKind::Lib {
                module: "sql".into(),
                name: name.into(),
                args,
            },
            ty,
            span,
        )
    }

    /// The values a query binds, as a `List<sql.Value>`.
    fn sql_values(&mut self, params: &[kiln_check::orm::Param], span: Span) -> Expr {
        let value_ty = self.std_type("sql", "Value");
        let string = Ty::Adt(self.prims.string, Vec::new());
        let int = Ty::Adt(self.prims.int, Vec::new());
        let mut items = Vec::new();
        for p in params {
            let (v, ty) = match p {
                kiln_check::orm::Param::Value(e) => {
                    let v = self.expr(e);
                    let ty = v.ty.clone();
                    (v, ty)
                }
                kiln_check::orm::Param::Like(e, mode) => {
                    let v = self.expr(e);
                    let pattern = mk(
                        ExprKind::Lib {
                            module: "orm".into(),
                            name: "__like".into(),
                            args: vec![v, mk(ExprKind::Int(*mode as i64), int.clone(), span)],
                        },
                        string.clone(),
                        span,
                    );
                    (pattern, string.clone())
                }
            };
            let _ = ty;
            items.push(self.sql_value(v, span, &value_ty));
        }
        let list = Ty::Adt(self.prims.list, vec![value_ty]);
        mk(ExprKind::List(items), list, span)
    }

    /// The values of some of a row's fields, as a `List<sql.Value>`.
    fn field_params(&mut self, value: &Expr, fields: &[usize], span: Span) -> Expr {
        let value_ty = self.std_type("sql", "Value");
        let mut items = Vec::new();
        for &f in fields {
            let ty = match &value.ty {
                Ty::Adt(id, args) => match &self.p.adts[*id].kind {
                    AdtKind::Struct(decl) => substitute(&decl[f].ty, &self.type_map(*id, args)),
                    _ => Ty::Unknown,
                },
                _ => Ty::Unknown,
            };
            let read = mk(
                ExprKind::Field {
                    base: Box::new(reread(value)),
                    field: f,
                },
                ty,
                span,
            );
            items.push(self.sql_value(read, span, &value_ty));
        }
        let list = Ty::Adt(self.prims.list, vec![value_ty.clone()]);
        mk(ExprKind::List(items), list, span)
    }

    /// One value as a `sql.Value`.
    fn sql_value(&mut self, v: Expr, span: Span, value_ty: &Ty) -> Expr {
        if v.ty == *value_ty {
            return v;
        }
        mk(
            ExprKind::Lib {
                module: "sql".into(),
                name: "__value".into(),
                args: vec![coerce(v, &Ty::Any)],
            },
            value_ty.clone(),
            span,
        )
    }

    /// Fills the arguments left out of a call to a standard library
    /// function with the defaults its declaration gives. `arrange` leaves
    /// each of them as an [`ExprKind::Error`] with an empty span.
    fn std_defaults(
        &mut self,
        module: &str,
        impl_type: Option<&str>,
        name: &str,
        params: &[ParamSig],
        vals: &mut [Expr],
        span: Span,
    ) {
        if !vals
            .iter()
            .any(|v| matches!(v.kind, ExprKind::Error) && v.span == Span::new(0, 0))
        {
            return;
        }
        if !self.std_asts.contains_key(module) {
            let Some(src) = kiln_check::std_source(module) else {
                return;
            };
            self.std_asts
                .insert(module.to_string(), kiln_syntax::parse(src).module);
        }
        let defaults: Vec<Option<ast::Expr>> = {
            let ast = &self.std_asts[module];
            let decl = ast.items.iter().find_map(|item| match (item, impl_type) {
                (ast::Item::Fn(f), None) if f.name.name == name => Some(f),
                (ast::Item::Impl(i), Some(t)) if type_expr_name(&i.target) == Some(t) => {
                    i.methods.iter().find(|m| m.name.name == name)
                }
                _ => None,
            });
            let Some(decl) = decl else { return };
            decl.params
                .iter()
                .filter(|p| p.name.name != "self")
                .map(|p| p.default.clone())
                .collect()
        };
        for (i, v) in vals.iter_mut().enumerate() {
            if !(matches!(v.kind, ExprKind::Error) && v.span == Span::new(0, 0)) {
                continue;
            }
            let Some(Some(d)) = defaults.get(i) else {
                continue;
            };
            let ty = params[i].ty.clone();
            let kind = match &d.kind {
                E::Int(text) => text.replace('_', "").parse().ok().map(ExprKind::Int),
                E::Bool(b) => Some(ExprKind::Bool(*b)),
                E::Duration(text) => duration_ns(text).map(ExprKind::Int),
                E::None => Some(ExprKind::None),
                E::List(items) if items.is_empty() => Some(ExprKind::List(Vec::new())),
                E::Call { callee, args, .. }
                    if args.is_empty() && matches!(&callee.kind, E::Ident(n) if n == "Map") =>
                {
                    Some(ExprKind::NewMap)
                }
                // A unit variant of the parameter's enum: `SameSite.Lax`.
                E::Field { name: variant, .. } => match &ty {
                    Ty::Adt(id, _) => match &self.p.adts[*id].kind {
                        AdtKind::Enum(variants) => variants
                            .iter()
                            .position(|v| v.name == variant.name)
                            .map(|index| ExprKind::Variant {
                                adt: *id,
                                index,
                                fields: Vec::new(),
                            }),
                        _ => None,
                    },
                    _ => None,
                },
                E::Str(pieces) => match pieces.as_slice() {
                    [] => Some(ExprKind::Str(String::new())),
                    [ast::StrPiece::Text(t)] => Some(ExprKind::Str(t.clone())),
                    _ => None,
                },
                _ => None,
            };
            *v = match kind {
                Some(kind) => mk(kind, ty, span),
                None => self.unsupported(span, "this default argument"),
            };
        }
    }

    fn module_call(
        &mut self,
        module: &str,
        name: &ast::Ident,
        args: &[ast::Arg],
        rt: &Ty,
        span: Span,
    ) -> (Expr, Option<Ty>) {
        if module == "stores" {
            return (self.stores_call(name, args, span), None);
        }
        if module == "log" && matches!(name.name.as_str(), "debug" | "info" | "warn" | "error") {
            if args.is_empty() {
                return (self.unsupported(span, "this call"), None);
            }
            let string = Ty::Adt(self.prims.string, Vec::new());
            let shown = args
                .iter()
                .map(|a| {
                    let v = self.expr(&a.value);
                    mk(ExprKind::Show(Box::new(v)), string.clone(), a.value.span)
                })
                .collect();
            let kind = ExprKind::Log {
                level: name.name.clone(),
                args: shown,
            };
            return (mk(kind, Ty::Unit, span), None);
        }
        if module == "io" && name.name == "print_err" && args.len() == 1 {
            // Like `print`, the value is shown first.
            let a = &args[0].value;
            let v = self.expr(a);
            let string = Ty::Adt(self.prims.string, Vec::new());
            let shown = mk(ExprKind::Show(Box::new(v)), string, a.span);
            let kind = ExprKind::Lib {
                module: "io".into(),
                name: "print_err".into(),
                args: vec![shown],
            };
            return (mk(kind, Ty::Unit, span), None);
        }
        let ty = self
            .p
            .modules
            .get(module)
            .and_then(|m| m.types.get(&name.name))
            .copied()
            .filter(|&id| matches!(&self.p.adts[id].kind, AdtKind::Struct(f) if !f.is_empty()));
        if let Some(id) = ty {
            return (self.construct(id, args, rt.clone(), span), None);
        }
        if module == "web" && name.name == "render" {
            return self.render(args, rt, span);
        }
        if module == "admin" && name.name == "pages" {
            return self.admin_pages(args, span);
        }
        if LIB_MODULES.contains(&module) {
            let sig = self
                .p
                .modules
                .get(module)
                .and_then(|m| m.fns.get(&name.name))
                .cloned();
            if let Some(sig) = sig {
                let (prelude, _, mut vals) = self.arrange(&sig.params, &[], None, args);
                self.std_defaults(module, None, &name.name, &sig.params, &mut vals, span);
                let call = mk(
                    ExprKind::Lib {
                        module: module.to_string(),
                        name: name.name.clone(),
                        args: vals,
                    },
                    lib_ret(&sig, self.p.instantiations.get(&span)),
                    span,
                );
                let call = if module == "web" && name.name == "app" {
                    self.with_assets(call)
                } else if module == "web" && RESPONSES.contains(&name.name.as_str()) {
                    with_headers(call)
                } else {
                    call
                };
                let call = coerce_value(call, rt);
                return (with_prelude(prelude, call), sig.throws.clone());
            }
        }
        // An npm package's function is called through the Kiln function
        // the build adds for it (see `kiln add`).
        if self.p.modules.get(module).is_some_and(|m| m.foreign) {
            let mangled = format!("__npm__{module}__{}", name.name);
            if let Some(&(func, decl)) = self.fns.get(&mangled) {
                let sig = self.p.global_fns[&mangled].clone();
                let (func, sig) =
                    self.instance(func, Template::Fn(mangled.clone()), &sig, span, None);
                return self.user_call(func, &sig, decl, None, args, rt, span);
            }
        }
        let what = format!("`{module}.{}`", name.name);
        (self.unsupported(span, what), None)
    }
}

impl Lowerer<'_> {
    /// A style, which is the string of its classes (see
    /// [`kiln_check::style`]).
    fn style(&mut self, e: &ast::Expr, style: StyleExpr) -> Expr {
        let string = Ty::Adt(self.prims.string, Vec::new());
        let span = e.span;
        let parts: Vec<&ast::Expr> = match (&style, &e.kind) {
            (StyleExpr::Classes(classes), _) => {
                return mk(ExprKind::Str(classes.clone()), string, span);
            }
            (StyleExpr::Record, E::Record(fields)) => {
                fields.iter().filter_map(|f| f.value.as_ref()).collect()
            }
            (StyleExpr::Same | StyleExpr::Merge, E::Call { args, .. }) => {
                args.iter().map(|a| &a.value).collect()
            }
            _ => return self.unsupported(span, "this style"),
        };
        let mut values = parts.into_iter().map(|p| self.expr(p));
        let Some(first) = values.next() else {
            return mk(ExprKind::Str(String::new()), string, span);
        };
        // Later values replace the classes of what they set.
        values.fold(first, |acc, next| {
            mk(
                ExprKind::Lib {
                    module: "ui".into(),
                    name: "merge".into(),
                    args: vec![acc, next],
                },
                string.clone(),
                span,
            )
        })
    }

    /// `web.app()` with the routes that serve the program's files (see
    /// [`crate::glaze::ASSETS_FN`]) added, when it has any.
    fn with_assets(&mut self, app: Expr) -> Expr {
        let name = crate::glaze::ASSETS_FN;
        let Some(&(func, _)) = self.fns.get(name) else {
            return app;
        };
        let sig = self.p.global_fns[name].clone();
        let (func, _) = self.instance(func, Template::Fn(name.to_string()), &sig, app.span, None);
        let (ty, span) = (app.ty.clone(), app.span);
        let local = self.new_local("app", ty.clone(), false);
        let get = || mk(ExprKind::Local(local), ty.clone(), span);
        let add = mk(
            ExprKind::Call {
                func,
                args: vec![get()],
            },
            Ty::Unit,
            span,
        );
        mk(
            ExprKind::Block(Block {
                stmts: vec![Stmt::Let(local, app), Stmt::Expr(add)],
                value: Some(Box::new(get())),
            }),
            ty,
            span,
        )
    }

    /// `admin.pages<Post>(app, prefix, site)` calls the function the
    /// compiler wrote for that model (see [`crate::admin`]).
    fn admin_pages(&mut self, args: &[ast::Arg], span: Span) -> (Expr, Option<Ty>) {
        let Some(model) = self.p.admins.get(&span).cloned() else {
            return (self.unsupported(span, "these admin pages"), None);
        };
        let fname = format!("__kiln_admin_{model}");
        let Some(&(func, decl)) = self.fns.get(&fname) else {
            return (
                self.unsupported(span, format!("the admin's pages for `{model}`")),
                None,
            );
        };
        let sig = self.p.global_fns[&fname].clone();
        let (func, sig) = self.instance(func, Template::Fn(fname), &sig, span, None);
        self.user_call(func, &sig, decl, None, args, &Ty::Unit, span)
    }

    /// `web.render("post.glaze", { post })` calls the function compiled from
    /// the glaze (see [`crate::glaze`]) with the data as named arguments,
    /// and makes an HTML response of what it returns.
    fn render(&mut self, args: &[ast::Arg], rt: &Ty, span: Span) -> (Expr, Option<Ty>) {
        let file = args.first().and_then(|a| match &a.value.kind {
            E::Str(pieces) => match pieces.as_slice() {
                [ast::StrPiece::Text(t)] => Some(t.clone()),
                _ => None,
            },
            _ => None,
        });
        let Some(file) = file else {
            let what = "`web.render` with a glaze name worked out at run time";
            return (self.unsupported(span, what), None);
        };
        let fname = crate::glaze::render_fn(&file);
        let Some(&(func, decl)) = self.fns.get(&fname) else {
            return (self.unsupported(span, format!("rendering `{file}`")), None);
        };
        let named = |n: &str| {
            args.iter()
                .find(|a| a.name.as_ref().is_some_and(|x| x.name == n))
        };
        let (status_arg, headers_arg) = (named("status"), named("headers"));
        let mut data = Vec::new();
        if let Some(arg) = args
            .get(1)
            .filter(|a| a.name.as_ref().is_none_or(|n| n.name == "data"))
        {
            let E::Record(fields) = &arg.value.kind else {
                return (
                    self.unsupported(arg.value.span, "render data that is not a record literal"),
                    None,
                );
            };
            for f in fields {
                let value = f.value.clone().unwrap_or_else(|| ast::Expr {
                    kind: E::Ident(f.name.name.clone()),
                    span: f.name.span,
                });
                data.push(ast::Arg {
                    name: Some(f.name.clone()),
                    value,
                });
            }
        }
        let sig = self.p.global_fns[&fname].clone();
        let string = Ty::Adt(self.prims.string, Vec::new());
        let (func, sig) = self.instance(func, Template::Fn(fname), &sig, span, None);
        let (html, _) = self.user_call(func, &sig, decl, None, &data, &string, span);
        let int = Ty::Adt(self.prims.int, Vec::new());
        let status = match status_arg {
            Some(a) => self.expr(&a.value),
            None => mk(ExprKind::Int(200), int, span),
        };
        let headers = match headers_arg {
            Some(a) => self.expr(&a.value),
            None => mk(ExprKind::NewMap, Ty::Unknown, span),
        };
        let kind = ExprKind::Lib {
            module: "web".into(),
            name: "html".into(),
            args: vec![html, status, headers],
        };
        (with_headers(mk(kind, rt.clone(), span)), None)
    }
}

/// `web` functions that make a response and take `status` and `headers`.
const RESPONSES: &[&str] = &["json", "text", "html", "redirect", "bytes"];

/// Takes a response call's last argument, its headers, off the call, and
/// adds them to the response it makes unless there are none to add.
fn with_headers(mut call: Expr) -> Expr {
    let ExprKind::Lib { args, .. } = &mut call.kind else {
        return call;
    };
    let headers = args.pop().expect("a response call has headers");
    if matches!(headers.kind, ExprKind::NewMap) {
        return call;
    }
    let (ty, span) = (call.ty.clone(), call.span);
    mk(
        ExprKind::Lib {
            module: "web".into(),
            name: "__with_headers".into(),
            args: vec![call, headers],
        },
        ty,
        span,
    )
}

/// Modules whose functions become [`ExprKind::Lib`] calls, for the
/// backend to build.
const LIB_MODULES: &[&str] = &[
    "time", "fs", "os", "io", "web", "json", "html", "base64", "markdown", "ember", "js",
    "validate", "browser", "sql", "auth", "admin",
];

/// Types of those modules whose methods are [`ExprKind::Lib`] calls too,
/// named `Type.method`, with the receiver as the first argument.
const LIB_TYPES: &[(&str, &str)] = &[
    ("web", "App"),
    ("web", "Request"),
    ("web", "Response"),
    ("ember", "Event"),
    ("js", "Value"),
    ("sql", "Db"),
    ("validate", "Checked"),
    ("auth", "Sessions"),
];

/// The type of a library call: its declared return type, with a generic
/// one's type arguments at this call filled in (`json.decode<Post>` has type
/// `Post`), so that the backend knows what to build.
fn lib_ret(sig: &FnSig, inst: Option<&HashMap<String, Ty>>) -> Ty {
    match inst {
        Some(map) if !sig.generics.is_empty() => substitute(&sig.ret, map),
        _ => sig.ret.clone(),
    }
}

fn is_generic(sig: &FnSig) -> bool {
    !sig.generics.is_empty() || !sig.impl_params.is_empty()
}

/// A second read of a local, such as one `hoist` stored a value in.
fn reread(e: &Expr) -> Expr {
    match &e.kind {
        ExprKind::Local(id) => mk(ExprKind::Local(*id), e.ty.clone(), e.span),
        _ => panic!("internal compiler error: only a local can be read again"),
    }
}

fn mk(kind: ExprKind, ty: Ty, span: Span) -> Expr {
    Expr { kind, ty, span }
}

/// The value moved to type `to`, with a [`ExprKind::Coerce`] node when the
/// types differ.
fn coerce(e: Expr, to: &Ty) -> Expr {
    if e.ty == *to
        || matches!(e.ty, Ty::Never | Ty::Unknown)
        || matches!(to, Ty::Never | Ty::Unknown | Ty::Unit)
    {
        return e;
    }
    let span = e.span;
    mk(ExprKind::Coerce(Box::new(e)), to.clone(), span)
}

/// Like [`coerce`], but leaves values of no type (`Unit`, `Never`) alone,
/// as branches of an `if` that is used as a value may have them.
fn coerce_value(e: Expr, to: &Ty) -> Expr {
    if matches!(e.ty, Ty::Unit | Ty::Never) || matches!(to, Ty::Unit | Ty::Never) {
        return e;
    }
    coerce(e, to)
}

fn coerce_block(mut b: Block, to: &Ty) -> Block {
    if let Some(v) = b.value.take() {
        b.value = Some(Box::new(coerce_value(*v, to)));
    }
    b
}

fn with_prelude(prelude: Vec<Stmt>, e: Expr) -> Expr {
    if prelude.is_empty() {
        return e;
    }
    let (ty, span) = (e.ty.clone(), e.span);
    mk(
        ExprKind::Block(Block {
            stmts: prelude,
            value: Some(Box::new(e)),
        }),
        ty,
        span,
    )
}

/// The name of a plain named type, such as `App` in `impl App`.
/// A duration literal such as `250ms` in nanoseconds, or `None` if it does
/// not fit.
fn duration_ns(s: &str) -> Option<i64> {
    let split = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
    let n: i64 = s[..split].replace('_', "").parse().unwrap_or(0);
    let unit: i64 = match &s[split..] {
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" => 60_000_000_000,
        "h" => 3_600_000_000_000,
        _ => 1,
    };
    n.checked_mul(unit)
}

fn type_expr_name(t: &ast::TypeExpr) -> Option<&str> {
    match &t.kind {
        ast::TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() => {
            Some(&path[0].name)
        }
        _ => None,
    }
}

/// `Post` for `KilnModule_5_posts_symbol_Post`, the name a folder
/// module's type has inside a program.
pub fn source_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("KilnModule_")?;
    let at = rest.rfind("_symbol_")?;
    Some(rest[at + "_symbol_".len()..].to_string())
}

/// Whether a parameter's default is [`CALLER_DEFAULT`].
fn is_caller_default(d: &ast::Expr) -> bool {
    matches!(&d.kind, ast::ExprKind::Str(pieces)
        if matches!(pieces.as_slice(), [ast::StrPiece::Text(t)] if t == CALLER_DEFAULT))
}
