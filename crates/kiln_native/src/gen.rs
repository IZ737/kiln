//! Generates machine code for a program in the IR with Cranelift.
//!
//! Every IR function becomes a function in one object file, using the C
//! calling convention, so the runtime can call closures and helpers
//! directly. Locals are Cranelift variables; a captured `var` lives in a
//! heap cell, as in the C backend. Values are stored as in the C backend
//! (see `runtime/kiln.c`): `Int` and `Duration` as 64-bit integers, `Float`
//! as a double, `Bool` as a byte, and everything else as a pointer, with
//! `kv` 64-bit slots for generic values. Struct fields, record fields and
//! enum fields are all slots here, so a struct is an array of `kv`.
//!
//! Showing, comparing, hashing and ordering values of compound types goes
//! through the runtime's `k_desc_*` functions, which walk a description of
//! the type that this module writes into the object's data (see
//! `runtime/native.c`).

use crate::runtime::{self, C};
use crate::Target;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    self, types, AbiParam, BlockArg, InstBuilder, MemFlagsData, Signature, StackSlotData,
    StackSlotKind, Type, Value,
};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId as ObjFn, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use kiln_check::types::substitute;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_ir::*;
use std::collections::{HashMap, HashSet};

pub(crate) fn compile(program: &Program, target: Target) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let mut gen = Gen::new(program, target);
    gen.program();
    if !gen.diags.is_empty() {
        let mut diags = gen.diags;
        diags.sort_by_key(|d| (d.span.start, d.span.end));
        diags.dedup_by(|a, b| a.span == b.span && a.message == b.message);
        return Err(diags);
    }
    gen.m.finish().emit().map_err(|e| {
        vec![Diagnostic::error(
            codes::NOT_COMPILED_YET,
            format!("internal compiler error: cannot write the object file: {e}"),
            Span::default(),
        )]
    })
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
    m: ObjectModule,
    /// The C calling convention of the target, used by every function.
    cc: CallConv,
    diags: Vec<Diagnostic>,
    /// The object's function for each IR function.
    funcs: Vec<ObjFn>,
    imports: HashMap<&'static str, ObjFn>,
    data_imports: HashMap<&'static str, DataId>,
    /// String constants by their bytes.
    strings: HashMap<Vec<u8>, DataId>,
    descs: HashMap<String, DataId>,
    /// Helper functions by what they do, such as `show:true:<type>`.
    helpers: HashMap<String, ObjFn>,
    /// The types already checked for each operation.
    checked: HashSet<(Op, String)>,
    globals: DataId,
    counter: usize,
}

impl<'a> Gen<'a> {
    fn new(p: &'a Program, target: Target) -> Self {
        let mut flags = settings::builder();
        flags.set("opt_level", "speed").unwrap();
        flags.set("is_pic", "false").unwrap();
        let triple = match target {
            Target::Linux => "x86_64-unknown-linux-gnu",
            Target::Windows => {
                // Windows grows the stack one guard page at a time, so a
                // frame larger than a page must touch each page in turn.
                flags.set("enable_probestack", "true").unwrap();
                flags.set("probestack_strategy", "inline").unwrap();
                "x86_64-pc-windows-gnu"
            }
        };
        let isa = cranelift_codegen::isa::lookup_by_name(triple)
            .unwrap()
            .finish(settings::Flags::new(flags))
            .unwrap();
        let cc = isa.default_call_conv();
        let builder =
            ObjectBuilder::new(isa, "kiln", cranelift_module::default_libcall_names()).unwrap();
        let mut m = ObjectModule::new(builder);
        let globals = m
            .declare_data("kiln_globals", Linkage::Export, true, false)
            .unwrap();
        Gen {
            p,
            prims: p.prims,
            m,
            cc,
            diags: Vec::new(),
            funcs: Vec::new(),
            imports: HashMap::new(),
            data_imports: HashMap::new(),
            strings: HashMap::new(),
            descs: HashMap::new(),
            helpers: HashMap::new(),
            checked: HashSet::new(),
            globals,
            counter: 0,
        }
    }

    fn next(&mut self) -> usize {
        self.counter += 1;
        self.counter
    }

    // ----- the program -----

    fn program(&mut self) {
        let mut globals = DataDescription::new();
        globals.set_align(8);
        globals.define_zeroinit(8 * self.p.globals.len().max(1));
        self.m.define_data(self.globals, &globals).unwrap();
        for id in 0..self.p.funcs.len() {
            let func = &self.p.funcs[id];
            let (name, linkage) = if id == self.p.init {
                ("kiln_init".to_string(), Linkage::Export)
            } else if id == self.p.main {
                ("kiln_main".to_string(), Linkage::Export)
            } else {
                (format!("kf_{id}_{}", mangle(&func.name)), Linkage::Local)
            };
            let sig = self.func_sig(id);
            let f = self.m.declare_function(&name, linkage, &sig).unwrap();
            self.funcs.push(f);
        }
        for id in 0..self.p.funcs.len() {
            self.function(id);
        }
    }

    fn func_sig(&self, id: FuncId) -> Signature {
        let func = &self.p.funcs[id];
        if func.kind == FuncKind::Closure {
            return sig(self.cc, &[types::I64, types::I64], Some(types::I64));
        }
        let params: Vec<Type> = func
            .params
            .iter()
            .map(|&l| self.cty(&func.locals[l].ty))
            .collect();
        let ret = (!is_void(&func.ret)).then(|| self.cty(&func.ret));
        sig(self.cc, &params, ret)
    }

    /// Generates a function's body with `body`.
    fn define(&mut self, id: ObjFn, signature: Signature, body: impl FnOnce(&mut Fx<'_, 'a, '_>)) {
        let mut ctx = self.m.make_context();
        ctx.func.signature = signature;
        let config = self.m.isa().frontend_config();
        let mut fbc = FunctionBuilderContext::new();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let entry = b.create_block();
            let mut fx = Fx {
                g: self,
                b,
                locals: Vec::new(),
                tys: Vec::new(),
                ret: Ty::Unit,
                closure: false,
                loops: Vec::new(),
                frefs: HashMap::new(),
                grefs: HashMap::new(),
                entry,
            };
            fx.b.append_block_params_for_function_params(entry);
            fx.b.switch_to_block(entry);
            body(&mut fx);
            fx.b.seal_all_blocks();
            fx.b.finalize(config);
        }
        if let Err(e) = self.m.define_function(id, &mut ctx) {
            panic!(
                "internal compiler error: Cranelift rejected a function: {e:?}\n{}",
                ctx.func.display()
            );
        }
    }

    fn function(&mut self, id: FuncId) {
        let p = self.p;
        let func = &p.funcs[id];
        let obj = self.funcs[id];
        let signature = self.func_sig(id);
        self.define(obj, signature, |fx| {
            let closure = func.kind == FuncKind::Closure;
            fx.ret = func.ret.clone();
            fx.closure = closure;
            fx.tys = func.locals.iter().map(|l| l.ty.clone()).collect();
            for local in &func.locals {
                let ty = fx.g.cty(&local.ty);
                let slot = if local.cell {
                    let v = fx.b.declare_var(types::I64);
                    let zero = fx.b.ins().iconst(types::I64, 0);
                    fx.b.def_var(v, zero);
                    Slot::Cell(v, ty)
                } else {
                    let v = fx.b.declare_var(ty);
                    let zero = fx.zero_of(ty);
                    fx.b.def_var(v, zero);
                    Slot::Var(v)
                };
                fx.locals.push(slot);
            }
            let params: Vec<Value> = fx.b.block_params(fx.entry).to_vec();
            if closure {
                let (env, args) = (params[0], params[1]);
                for (i, &l) in func.captures.iter().enumerate() {
                    let raw = fx.load(types::I64, env, 8 * i as i32);
                    match fx.locals[l] {
                        Slot::Cell(v, _) => fx.b.def_var(v, raw),
                        Slot::Var(_) => {
                            let v = fx.unkv(raw, &func.locals[l].ty);
                            fx.set_local(l, v);
                        }
                    }
                }
                for (i, &l) in func.params.iter().enumerate() {
                    let raw = fx.load(types::I64, args, 8 * i as i32);
                    let v = fx.unkv(raw, &func.locals[l].ty);
                    fx.init_local(l, v);
                }
            } else {
                for (&l, &v) in func.params.iter().zip(&params) {
                    fx.init_local(l, v);
                }
            }
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
            Ty::Param(_) | Ty::Var(_) | Ty::Any | Ty::Rest | Ty::Unknown => Rep::Kv,
            Ty::Unit | Ty::Never => Rep::Void,
        }
    }

    /// The Cranelift type a value of `ty` is held in. Unit values are never
    /// used; they are a 64-bit zero.
    fn cty(&self, ty: &Ty) -> Type {
        match self.rep(ty) {
            Rep::Float => types::F64,
            Rep::Bool => types::I8,
            _ => types::I64,
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

    fn import(&mut self, name: &'static str) -> ObjFn {
        if let Some(&f) = self.imports.get(name) {
            return f;
        }
        let (params, ret) = runtime::signature(name)
            .unwrap_or_else(|| panic!("internal compiler error: no signature for `{name}`"));
        let mut s = Signature::new(self.cc);
        let abi = |c: C| match c {
            C::I64 => AbiParam::new(types::I64),
            C::F64 => AbiParam::new(types::F64),
            C::Bool => AbiParam::new(types::I8).uext(),
            C::I32 => AbiParam::new(types::I32).sext(),
        };
        s.params.extend(params.iter().map(|&c| abi(c)));
        s.returns.extend(ret.map(abi));
        let f = self.m.declare_function(name, Linkage::Import, &s).unwrap();
        self.imports.insert(name, f);
        f
    }

    fn data_import(&mut self, name: &'static str) -> DataId {
        if let Some(&d) = self.data_imports.get(name) {
            return d;
        }
        let d = self
            .m
            .declare_data(name, Linkage::Import, true, false)
            .unwrap();
        self.data_imports.insert(name, d);
        d
    }

    /// Read-only bytes in the object.
    fn bytes(&mut self, bytes: Vec<u8>) -> DataId {
        if let Some(&d) = self.strings.get(&bytes) {
            return d;
        }
        let name = format!("ks_{}", self.next());
        let d = self
            .m
            .declare_data(&name, Linkage::Local, false, false)
            .unwrap();
        let mut data = DataDescription::new();
        data.define(bytes.clone().into_boxed_slice());
        self.m.define_data(d, &data).unwrap();
        self.strings.insert(bytes, d);
        d
    }

    /// A NUL-terminated string in the object.
    fn c_string(&mut self, s: &str) -> DataId {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        self.bytes(bytes)
    }

    /// A helper function, declared and generated the first time it is
    /// asked for.
    fn helper(
        &mut self,
        key: String,
        signature: Signature,
        body: impl FnOnce(&mut Fx<'_, 'a, '_>),
    ) -> ObjFn {
        if let Some(&f) = self.helpers.get(&key) {
            return f;
        }
        let name = format!("kh_{}", self.next());
        let f = self
            .m
            .declare_function(&name, Linkage::Local, &signature)
            .unwrap();
        self.helpers.insert(key, f);
        self.define(f, signature, body);
        f
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
            Op::Show | Op::Eq => {
                self.is_string(ty) || (matches!(ty, Ty::Adt(..)) && rep != Rep::Ptr)
            }
            Op::Hash | Op::Cmp => {
                self.is_string(ty) || matches!(rep, Rep::Int | Rep::Bool | Rep::Float)
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
    /// after it.
    fn desc(&mut self, ty: &Ty) -> DataId {
        let key = format!("{ty:?}");
        if let Some(&d) = self.descs.get(&key) {
            return d;
        }
        let name = format!("kd_{}", self.next());
        let d = self
            .m
            .declare_data(&name, Linkage::Local, false, false)
            .unwrap();
        // Recursive types refer back to this description.
        self.descs.insert(key, d);
        let p = self.prims;
        let leaf = |kind: i64| (kind, None, Vec::new(), Vec::new());
        let (kind, name, parts, labels): (i64, Option<String>, Vec<DataId>, Vec<Option<String>>) =
            match ty {
                _ if self.is_string(ty) => leaf(desc::STR),
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
        self.define_desc(d, kind, name.as_deref(), &parts, &labels);
        d
    }

    fn variant_desc(&mut self, name: &str, fields: &[(Option<String>, Ty)]) -> DataId {
        let id = format!("kd_{}", self.next());
        let d = self
            .m
            .declare_data(&id, Linkage::Local, false, false)
            .unwrap();
        let parts: Vec<DataId> = fields.iter().map(|(_, t)| self.desc(t)).collect();
        let labels: Vec<Option<String>> = fields.iter().map(|(n, _)| n.clone()).collect();
        self.define_desc(d, desc::VARIANT, Some(name), &parts, &labels);
        d
    }

    fn define_desc(
        &mut self,
        d: DataId,
        kind: i64,
        name: Option<&str>,
        parts: &[DataId],
        labels: &[Option<String>],
    ) {
        let n = parts.len();
        let mut bytes = vec![0u8; 8 * (5 + 2 * n)];
        bytes[0..8].copy_from_slice(&kind.to_le_bytes());
        bytes[8..16].copy_from_slice(&(n as i64).to_le_bytes());
        let mut data = DataDescription::new();
        data.set_align(8);
        let mut relocs: Vec<(u32, DataId, i64)> = Vec::new();
        if let Some(name) = name {
            relocs.push((16, self.c_string(name), 0));
        }
        relocs.push((24, d, 40));
        relocs.push((32, d, 40 + 8 * n as i64));
        for (i, &part) in parts.iter().enumerate() {
            relocs.push(((40 + 8 * i) as u32, part, 0));
        }
        for (i, label) in labels.iter().enumerate() {
            if let Some(label) = label {
                let s = self.c_string(label);
                relocs.push(((40 + 8 * (n + i)) as u32, s, 0));
            }
        }
        data.define(bytes.into_boxed_slice());
        for (offset, target, addend) in relocs {
            let gv = self.m.declare_data_in_data(target, &mut data);
            data.write_data_addr(offset, gv, addend);
        }
        self.m.define_data(d, &data).unwrap();
    }

    /// A `kstr *(kv)` function that shows values of `ty`, quoting strings
    /// with `quote`, as they are inside lists and structs.
    fn show_fn(&mut self, ty: &Ty, quote: bool, span: Span) -> ObjFn {
        self.check(Op::Show, ty, span);
        let d = self.desc(ty);
        let key = format!("show:{quote}:{ty:?}");
        self.helper(key, sig(self.cc, &[types::I64], Some(types::I64)), |fx| {
            let v = fx.param(0);
            let addr = fx.data_addr(d);
            let q = fx.b.ins().iconst(types::I8, i64::from(quote));
            let r = fx.call_rt("k_desc_show", &[addr, v, q]);
            fx.b.ins().return_(&[r]);
        })
    }

    /// A `bool (kv, kv)` function that compares values of `ty`.
    fn eq_fn(&mut self, ty: &Ty, span: Span) -> ObjFn {
        self.check(Op::Eq, ty, span);
        let d = self.desc(ty);
        let key = format!("eq:{ty:?}");
        self.helper(
            key,
            sig(self.cc, &[types::I64, types::I64], Some(types::I8)),
            |fx| {
                let (a, b) = (fx.param(0), fx.param(1));
                let addr = fx.data_addr(d);
                let r = fx.call_rt("k_desc_eq", &[addr, a, b]);
                fx.b.ins().return_(&[r]);
            },
        )
    }

    /// The hash function for map keys and set items of `ty`.
    fn hash_fn(&mut self, ty: &Ty, span: Span) -> ObjFn {
        if self.is_string(ty) {
            return self.import("k_hash_str");
        }
        match self.rep(ty) {
            Rep::Int | Rep::Bool => return self.import("k_hash_int"),
            Rep::Float => return self.import("k_hash_float"),
            _ => {}
        }
        self.check(Op::Hash, ty, span);
        let d = self.desc(ty);
        let key = format!("hash:{ty:?}");
        self.helper(key, sig(self.cc, &[types::I64], Some(types::I64)), |fx| {
            let v = fx.param(0);
            let addr = fx.data_addr(d);
            let r = fx.call_rt("k_desc_hash", &[addr, v]);
            fx.b.ins().return_(&[r]);
        })
    }

    /// An `int (kv, kv)` function that orders values of `ty`.
    fn cmp_fn(&mut self, ty: &Ty, span: Span) -> ObjFn {
        self.check(Op::Cmp, ty, span);
        let d = self.desc(ty);
        let key = format!("cmp:{ty:?}");
        self.helper(
            key,
            sig(self.cc, &[types::I64, types::I64], Some(types::I32)),
            |fx| {
                let (a, b) = (fx.param(0), fx.param(1));
                let addr = fx.data_addr(d);
                let r = fx.call_rt("k_desc_cmp", &[addr, a, b]);
                fx.b.ins().return_(&[r]);
            },
        )
    }

    /// The `kstr *(kv)` function that shows a standard `Error`: its
    /// message.
    fn error_message_fn(&mut self) -> ObjFn {
        let offset = 8 * self.field_index(self.prims.error, "message") as i32;
        self.helper(
            "error-message".into(),
            sig(self.cc, &[types::I64], Some(types::I64)),
            |fx| {
                let v = fx.param(0);
                let m = fx.load(types::I64, v, offset);
                fx.b.ins().return_(&[m]);
            },
        )
    }

    /// A top-level function as a closure function: `kv (void *, kv *)`.
    fn fn_wrapper(&mut self, id: FuncId) -> ObjFn {
        let p = self.p;
        let func = &p.funcs[id];
        let target = self.funcs[id];
        self.helper(
            format!("fn:{id}"),
            sig(self.cc, &[types::I64, types::I64], Some(types::I64)),
            |fx| {
                let args = fx.param(1);
                let mut vals = Vec::new();
                for (i, &l) in func.params.iter().enumerate() {
                    let raw = fx.load(types::I64, args, 8 * i as i32);
                    let v = fx.unkv(raw, &func.locals[l].ty);
                    let want = fx.g.cty(&func.locals[l].ty);
                    vals.push(fx.fit(v, want));
                }
                let r = fx.call(target, &vals);
                let out = match r {
                    Some(r) if !is_void(&func.ret) => fx.kv_of(r, &func.ret),
                    _ => fx.b.ins().iconst(types::I64, 0),
                };
                fx.b.ins().return_(&[out]);
            },
        )
    }
}

/// Where a local lives: a variable, or for a captured `var`, a variable
/// holding a pointer to its cell, with the cell's content type.
#[derive(Clone, Copy)]
enum Slot {
    Var(Variable),
    Cell(Variable, Type),
}

/// The state of the function being generated.
struct Fx<'g, 'a, 'b> {
    g: &'g mut Gen<'a>,
    b: FunctionBuilder<'b>,
    locals: Vec<Slot>,
    tys: Vec<Ty>,
    ret: Ty,
    closure: bool,
    /// For each loop around the current point: where `continue` and
    /// `break` go.
    loops: Vec<(ir::Block, ir::Block)>,
    frefs: HashMap<ObjFn, ir::FuncRef>,
    grefs: HashMap<DataId, ir::GlobalValue>,
    entry: ir::Block,
}

impl Fx<'_, '_, '_> {
    // ----- instructions -----

    fn param(&self, i: usize) -> Value {
        self.b.block_params(self.entry)[i]
    }

    fn iconst(&mut self, n: i64) -> Value {
        self.b.ins().iconst(types::I64, n)
    }

    fn zero_of(&mut self, ty: Type) -> Value {
        if ty == types::F64 {
            self.b.ins().f64const(0.0)
        } else {
            self.b.ins().iconst(ty, 0)
        }
    }

    fn zero(&mut self, ty: &Ty) -> Value {
        let t = self.g.cty(ty);
        self.zero_of(t)
    }

    fn load(&mut self, ty: Type, addr: Value, offset: i32) -> Value {
        self.b.ins().load(ty, MemFlagsData::trusted(), addr, offset)
    }

    fn store(&mut self, v: Value, addr: Value, offset: i32) {
        self.b.ins().store(MemFlagsData::trusted(), v, addr, offset);
    }

    fn fref(&mut self, f: ObjFn) -> ir::FuncRef {
        if let Some(&r) = self.frefs.get(&f) {
            return r;
        }
        let r = self.g.m.declare_func_in_func(f, self.b.func);
        self.frefs.insert(f, r);
        r
    }

    fn data_addr(&mut self, d: DataId) -> Value {
        let gv = match self.grefs.get(&d) {
            Some(&gv) => gv,
            None => {
                let gv = self.g.m.declare_data_in_func(d, self.b.func);
                self.grefs.insert(d, gv);
                gv
            }
        };
        self.b.ins().symbol_value(types::I64, gv)
    }

    fn func_addr(&mut self, f: ObjFn) -> Value {
        let r = self.fref(f);
        self.b.ins().func_addr(types::I64, r)
    }

    /// Calls a function in the object, fitting the arguments to its
    /// parameters. Returns its result, if it has one.
    fn call(&mut self, f: ObjFn, args: &[Value]) -> Option<Value> {
        let r = self.fref(f);
        let sig = self.b.func.dfg.ext_funcs[r].signature;
        let want: Vec<Type> = self.b.func.dfg.signatures[sig]
            .params
            .iter()
            .map(|p| p.value_type)
            .collect();
        let args: Vec<Value> = args
            .iter()
            .zip(want)
            .map(|(&a, t)| self.fit(a, t))
            .collect();
        let call = self.b.ins().call(r, &args);
        self.b.inst_results(call).first().copied()
    }

    /// Calls a runtime function. Returns its result, or a zero for `void`.
    fn call_rt(&mut self, name: &'static str, args: &[Value]) -> Value {
        let f = self.g.import(name);
        match self.call(f, args) {
            Some(v) => v,
            None => self.iconst(0),
        }
    }

    /// A C string constant's address.
    fn c_str(&mut self, s: &str) -> Value {
        let d = self.g.c_string(s);
        self.data_addr(d)
    }

    /// A new `kstr *` with the text `s`.
    fn str_const(&mut self, s: &str) -> Value {
        let d = self.g.bytes(s.as_bytes().to_vec());
        let addr = self.data_addr(d);
        let n = self.iconst(s.len() as i64);
        self.call_rt("k_str_n", &[addr, n])
    }

    /// Stores values in a new stack array of slots and returns its address.
    fn stack_array(&mut self, vals: &[Value]) -> Value {
        let size = 8 * vals.len().max(1) as u32;
        let slot = self.b.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            size,
            3,
        ));
        for (i, &v) in vals.iter().enumerate() {
            self.b.ins().stack_store(types::I64, v, slot, 8 * i as i32);
        }
        self.b.ins().stack_addr(types::I64, slot, 0)
    }

    /// Allocates `n` slots on the heap.
    fn alloc(&mut self, n: usize) -> Value {
        let size = self.iconst(8 * n.max(1) as i64);
        self.call_rt("k_alloc", &[size])
    }

    /// Continues in a new block that nothing jumps to, after a `return`,
    /// `break` or `continue`.
    fn dead(&mut self) {
        let block = self.b.create_block();
        self.b.switch_to_block(block);
    }

    fn jump(&mut self, to: ir::Block, args: &[Value]) {
        let args: Vec<BlockArg> = args.iter().map(|&v| BlockArg::Value(v)).collect();
        self.b.ins().jump(to, &args);
    }

    fn brif(&mut self, c: Value, then: ir::Block, otherwise: ir::Block) {
        self.b.ins().brif(c, then, &[], otherwise, &[]);
    }

    /// Moves a value to the Cranelift type `want`. Values of the right kind
    /// pass unchanged; this smooths over values whose type is `Never` or
    /// unit, which have no real value.
    fn fit(&mut self, v: Value, want: Type) -> Value {
        let have = self.b.func.dfg.value_type(v);
        if have == want {
            return v;
        }
        match (have, want) {
            (types::I8, types::I64) => self.b.ins().uextend(types::I64, v),
            (types::I64, types::I8) => self.b.ins().icmp_imm_s(IntCC::NotEqual, v, 0),
            (types::F64, types::I64) => self.b.ins().bitcast(types::I64, MemFlagsData::new(), v),
            (types::I64, types::F64) => self.b.ins().bitcast(types::F64, MemFlagsData::new(), v),
            (types::I32, types::I64) => self.b.ins().sextend(types::I64, v),
            _ => self.zero_of(want),
        }
    }

    // ----- values and slots -----

    /// A value of type `ty` stored in a `kv` slot.
    fn kv_of(&mut self, v: Value, ty: &Ty) -> Value {
        match self.g.rep(ty) {
            Rep::Void => self.iconst(0),
            _ => self.fit(v, types::I64),
        }
    }

    /// A `kv` slot read back as a value of type `ty`.
    fn unkv(&mut self, v: Value, ty: &Ty) -> Value {
        match self.g.rep(ty) {
            Rep::Void => self.iconst(0),
            Rep::Float => self.fit(v, types::F64),
            Rep::Bool => self.fit(v, types::I8),
            _ => v,
        }
    }

    /// Converts a value of type `from` to the representation of `to`: into
    /// or out of a `kv` slot, or into an optional.
    fn convert(&mut self, v: Value, from: &Ty, to: &Ty) -> Value {
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

    fn local(&mut self, l: LocalId) -> Value {
        match self.locals[l] {
            Slot::Var(v) => self.b.use_var(v),
            Slot::Cell(v, ty) => {
                let cell = self.b.use_var(v);
                self.load(ty, cell, 0)
            }
        }
    }

    fn set_local(&mut self, l: LocalId, v: Value) {
        let want = self.g.cty(&self.tys[l].clone());
        let v = self.fit(v, want);
        match self.locals[l] {
            Slot::Var(var) => self.b.def_var(var, v),
            Slot::Cell(var, _) => {
                let cell = self.b.use_var(var);
                self.store(v, cell, 0);
            }
        }
    }

    /// Gives a local its first value, allocating its cell if it has one.
    fn init_local(&mut self, l: LocalId, v: Value) {
        if let Slot::Cell(var, _) = self.locals[l] {
            let cell = self.alloc(1);
            self.b.def_var(var, cell);
        }
        self.set_local(l, v);
    }

    fn global_addr(&mut self, g: GlobalId) -> (Value, i32) {
        let d = self.g.globals;
        (self.data_addr(d), 8 * g as i32)
    }

    // ----- returns and errors -----

    fn ret_value(&mut self, v: Value) {
        if self.closure {
            let ret = self.ret.clone();
            let k = self.kv_of(v, &ret);
            self.b.ins().return_(&[k]);
        } else if is_void(&self.ret) {
            self.b.ins().return_(&[]);
        } else {
            let want = self.g.cty(&self.ret.clone());
            let v = self.fit(v, want);
            self.b.ins().return_(&[v]);
        }
    }

    /// Leaves the current function after an error, or at the end of a
    /// function whose value is unused.
    fn fail_return(&mut self) {
        if self.closure {
            let z = self.iconst(0);
            self.b.ins().return_(&[z]);
        } else if is_void(&self.ret) {
            self.b.ins().return_(&[]);
        } else {
            let z = self.zero(&self.ret.clone());
            self.b.ins().return_(&[z]);
        }
    }

    fn failed(&mut self) -> Value {
        let d = self.g.data_import("k_failed");
        let addr = self.data_addr(d);
        self.load(types::I8, addr, 0)
    }

    /// After a call that failed, passes its error on to the caller,
    /// wrapping it in the standard `Error` first with `wrap`.
    fn propagate(&mut self, wrap: bool) {
        let (fail, cont) = (self.b.create_block(), self.b.create_block());
        let f = self.failed();
        self.brif(f, fail, cont);
        self.b.switch_to_block(fail);
        if wrap {
            let show_d = self.g.data_import("k_error_show");
            let err_d = self.g.data_import("k_error");
            let show_addr = self.data_addr(show_d);
            let err_addr = self.data_addr(err_d);
            let show = self.load(types::I64, show_addr, 0);
            let err = self.load(types::I64, err_addr, 0);
            let sigref = self
                .b
                .import_signature(sig(self.g.cc, &[types::I64], Some(types::I64)));
            let call = self.b.ins().call_indirect(sigref, show, &[err]);
            let message = self.b.inst_results(call)[0];
            let w = self.new_error(message);
            self.store(w, err_addr, 0);
            let f = self.g.error_message_fn();
            let fa = self.func_addr(f);
            self.store(fa, show_addr, 0);
        }
        self.fail_return();
        self.b.switch_to_block(cont);
    }

    /// A new standard `Error` with the message.
    fn new_error(&mut self, message: Value) -> Value {
        let error = self.g.prims.error;
        let n = self.g.field_count(error);
        let w = self.alloc(n);
        let offset = 8 * self.g.field_index(error, "message") as i32;
        self.store(message, w, offset);
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
        let fa = self.func_addr(show);
        self.call_rt("k_fail", &[k, fa]);
        self.fail_return();
        self.dead();
    }

    fn catch(&mut self, e: &Expr, want: bool) -> Value {
        let ExprKind::Catch {
            expr,
            err,
            binding,
            handler,
        } = &e.kind
        else {
            return self.iconst(0);
        };
        let rt = e.ty.clone();
        let v = self.expr(expr);
        let keep = want && !is_void(&rt) && !is_void(&expr.ty);
        let merge = self.b.create_block();
        let r = if keep {
            let t = self.g.cty(&rt);
            Some(self.b.append_block_param(merge, t))
        } else {
            None
        };
        let value = if keep {
            vec![self.convert(v, &expr.ty, &rt)]
        } else {
            Vec::new()
        };
        let handle = self.b.create_block();
        let f = self.failed();
        let args: Vec<BlockArg> = value.iter().map(|&v| BlockArg::Value(v)).collect();
        self.b.ins().brif(f, handle, &[], merge, &args);
        self.b.switch_to_block(handle);
        let taken = self.call_rt("k_take_error", &[]);
        if let Some(b) = binding {
            let v = self.unkv(taken, err);
            self.set_local(*b, v);
        }
        let hv = self.block(handler, keep);
        if keep {
            let ht = block_ty(handler);
            let v = if is_void(&ht) {
                self.zero(&rt)
            } else {
                self.convert(hv, &ht, &rt)
            };
            self.jump(merge, &[v]);
        } else {
            self.jump(merge, &[]);
        }
        self.b.switch_to_block(merge);
        r.unwrap_or_else(|| self.iconst(0))
    }

    // ----- statements -----

    /// Generates a block. Returns its value when `want` is set; otherwise
    /// the value is generated as a statement.
    fn block(&mut self, b: &Block, want: bool) -> Value {
        for stmt in &b.stmts {
            self.stmt(stmt);
        }
        match &b.value {
            Some(v) if want => self.expr(v),
            Some(v) => {
                self.discard(v);
                self.iconst(0)
            }
            None => self.iconst(0),
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
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
                self.store(v, addr, offset);
            }
            Stmt::Assign { place, op, value } => self.assign(place, *op, value),
            Stmt::Return(value) => {
                let v = match value {
                    Some(v) => self.expr(v),
                    None => self.iconst(0),
                };
                if value.is_some() {
                    self.ret_value(v);
                } else {
                    self.fail_return();
                }
                self.dead();
            }
            Stmt::Break | Stmt::Continue => {
                if let Some(&(cont, brk)) = self.loops.last() {
                    let to = if matches!(stmt, Stmt::Break) {
                        brk
                    } else {
                        cont
                    };
                    self.jump(to, &[]);
                    self.dead();
                }
            }
            Stmt::While { cond, body } => {
                let (header, body_b, exit) = (
                    self.b.create_block(),
                    self.b.create_block(),
                    self.b.create_block(),
                );
                self.jump(header, &[]);
                self.b.switch_to_block(header);
                let c = self.expr(cond);
                self.brif(c, body_b, exit);
                self.b.switch_to_block(body_b);
                self.loops.push((header, exit));
                self.block(body, false);
                self.loops.pop();
                self.jump(header, &[]);
                self.b.switch_to_block(exit);
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
                let i = self.b.declare_var(types::I64);
                self.b.def_var(i, lo);
                let int = Ty::Adt(self.g.prims.int, Vec::new());
                self.counted_loop(
                    i,
                    |fx| {
                        let n = fx.b.use_var(i);
                        fx.b.ins().icmp(IntCC::SignedLessThan, n, hi)
                    },
                    |fx| {
                        let n = fx.b.use_var(i);
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
                }
                let i = self.b.declare_var(types::I64);
                let zero = self.iconst(0);
                self.b.def_var(i, zero);
                self.counted_loop(
                    i,
                    |fx| {
                        let len = fx.load(types::I64, l, 0);
                        let n = fx.b.use_var(i);
                        fx.b.ins().icmp(IntCC::SignedLessThan, n, len)
                    },
                    |fx| {
                        let items = fx.load(types::I64, l, 16);
                        let n = fx.b.use_var(i);
                        let off = fx.b.ins().ishl_imm_u(n, 3);
                        let addr = fx.b.ins().iadd(items, off);
                        let raw = fx.load(types::I64, addr, 0);
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

    /// A loop over the counter `i`, from its value now while `cond` holds,
    /// counting up by one. `start` sets the loop variable each time round.
    fn counted_loop(
        &mut self,
        i: Variable,
        cond: impl FnOnce(&mut Self) -> Value,
        start: impl FnOnce(&mut Self),
        filter: Option<&Expr>,
        body: &Block,
    ) {
        let (header, body_b, latch, exit) = (
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
        );
        self.jump(header, &[]);
        self.b.switch_to_block(header);
        let c = cond(self);
        self.brif(c, body_b, exit);
        self.b.switch_to_block(body_b);
        start(self);
        if let Some(f) = filter {
            let keep = self.expr(f);
            let run = self.b.create_block();
            self.brif(keep, run, latch);
            self.b.switch_to_block(run);
        }
        self.loops.push((latch, exit));
        self.block(body, false);
        self.loops.pop();
        self.jump(latch, &[]);
        self.b.switch_to_block(latch);
        let n = self.b.use_var(i);
        let n = self.b.ins().iadd_imm_s(n, 1);
        self.b.def_var(i, n);
        self.jump(header, &[]);
        self.b.switch_to_block(exit);
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
                let offset = 8 * *field as i32;
                let v = self.expr(value);
                let cur = match op {
                    Some(_) => {
                        let raw = self.load(types::I64, b, offset);
                        let raw = self.unkv(raw, &decl);
                        Some(self.convert(raw, &decl, ty))
                    }
                    None => None,
                };
                let new = self.compound(op, cur, ty, v);
                let new = self.convert(new, ty, &decl);
                let k = self.kv_of(new, &decl);
                self.store(k, b, offset);
            }
            Place::ListItem { list, index, ty } => {
                let b = self.expr(list);
                let i = self.expr(index);
                let v = self.expr(value);
                let cur = match op {
                    Some(_) => {
                        let raw = self.call_rt("k_list_at", &[b, i]);
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
                        let raw = self.call_rt("k_map_at", &[b, k]);
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

    fn compound(&mut self, op: Option<ArithOp>, cur: Option<Value>, ty: &Ty, v: Value) -> Value {
        match (op, cur) {
            (Some(op), Some(cur)) => self.arith(op, cur, v, ty),
            _ => v,
        }
    }

    fn arith(&mut self, op: ArithOp, l: Value, r: Value, ty: &Ty) -> Value {
        if self.g.is_string(ty) {
            return self.call_rt("k_concat", &[l, r]);
        }
        if self.g.rep(ty) == Rep::Float {
            let (l, r) = (self.fit(l, types::F64), self.fit(r, types::F64));
            return match op {
                ArithOp::Add => self.b.ins().fadd(l, r),
                ArithOp::Sub => self.b.ins().fsub(l, r),
                ArithOp::Mul => self.b.ins().fmul(l, r),
                ArithOp::Div => self.b.ins().fdiv(l, r),
                ArithOp::Rem => self.call_rt("fmod", &[l, r]),
            };
        }
        let (l, r) = (self.fit(l, types::I64), self.fit(r, types::I64));
        let (v, overflow) = match op {
            ArithOp::Add => self.b.ins().sadd_overflow(l, r),
            ArithOp::Sub => self.b.ins().ssub_overflow(l, r),
            ArithOp::Mul => self.b.ins().smul_overflow(l, r),
            ArithOp::Div => return self.call_rt("k_div", &[l, r]),
            ArithOp::Rem => return self.call_rt("k_rem", &[l, r]),
        };
        let (bad, ok) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(bad);
        self.brif(overflow, bad, ok);
        self.b.switch_to_block(bad);
        self.panic_c("integer overflow");
        self.b.switch_to_block(ok);
        v
    }

    /// Panics with a fixed message; the current block ends here.
    fn panic_c(&mut self, message: &str) {
        let m = self.c_str(message);
        self.call_rt("k_panic_c", &[m]);
        self.b.ins().trap(ir::TrapCode::unwrap_user(1));
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

    fn expr(&mut self, e: &Expr) -> Value {
        match &e.kind {
            ExprKind::Int(n) => self.iconst(*n),
            ExprKind::Float(x) => self.b.ins().f64const(*x),
            ExprKind::Bool(b) => self.b.ins().iconst(types::I8, i64::from(*b)),
            ExprKind::Str(s) => self.str_const(s),
            ExprKind::None => self.iconst(0),
            ExprKind::Local(l) => self.local(*l),
            ExprKind::Global(g) => {
                let ty = self.g.cty(&self.g.p.globals[*g].ty);
                let (addr, offset) = self.global_addr(*g);
                self.load(ty, addr, offset)
            }
            ExprKind::FuncRef(f) => {
                let w = self.g.fn_wrapper(*f);
                let fa = self.func_addr(w);
                let null = self.iconst(0);
                self.call_rt("k_closure", &[fa, null])
            }
            ExprKind::Concat(parts) => {
                let mut acc: Option<Value> = None;
                for part in parts {
                    let v = self.expr(part);
                    acc = Some(match acc {
                        None => v,
                        Some(a) => self.call_rt("k_concat", &[a, v]),
                    });
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
                let x = self.fit(x, types::I8);
                self.b.ins().icmp_imm_s(IntCC::Equal, x, 0)
            }
            ExprKind::Neg(v) => {
                let x = self.expr(v);
                if self.g.rep(&e.ty) == Rep::Float {
                    self.b.ins().fneg(x)
                } else {
                    self.call_rt("k_neg", &[x])
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
                    CmpOp::Lt => (IntCC::SignedLessThan, FloatCC::LessThan),
                    CmpOp::Le => (IntCC::SignedLessThanOrEqual, FloatCC::LessThanOrEqual),
                    CmpOp::Gt => (IntCC::SignedGreaterThan, FloatCC::GreaterThan),
                    CmpOp::Ge => (IntCC::SignedGreaterThanOrEqual, FloatCC::GreaterThanOrEqual),
                };
                if self.g.is_string(&a.ty) {
                    let c = self.call_rt("k_str_cmp", &[l, r]);
                    self.b.ins().icmp_imm_s(icc, c, 0)
                } else if self.g.rep(&a.ty) == Rep::Float {
                    self.b.ins().fcmp(fcc, l, r)
                } else {
                    let r = {
                        let t = self.b.func.dfg.value_type(l);
                        self.fit(r, t)
                    };
                    self.b.ins().icmp(icc, l, r)
                }
            }
            ExprKind::Eq(a, b) => {
                let l = self.expr(a);
                let r = self.expr(b);
                self.eq_expr(l, &a.ty, r, &b.ty, e.span)
            }
            ExprKind::IsNone(v) => {
                let x = self.expr(v);
                self.b.ins().icmp_imm_s(IntCC::Equal, x, 0)
            }
            ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                let and = matches!(e.kind, ExprKind::And(..));
                let l = self.expr(a);
                let l = self.fit(l, types::I8);
                let (rhs, merge) = (self.b.create_block(), self.b.create_block());
                let r = self.b.append_block_param(merge, types::I8);
                if and {
                    self.b.ins().brif(l, rhs, &[], merge, &[BlockArg::Value(l)]);
                } else {
                    self.b.ins().brif(l, merge, &[BlockArg::Value(l)], rhs, &[]);
                }
                self.b.switch_to_block(rhs);
                let v = self.expr(b);
                let v = self.fit(v, types::I8);
                self.jump(merge, &[v]);
                self.b.switch_to_block(merge);
                r
            }
            ExprKind::Coalesce(a, b) => {
                let rt = e.ty.clone();
                let inner = match &a.ty {
                    Ty::Opt(inner) => (**inner).clone(),
                    _ => Ty::Unknown,
                };
                let t = self.expr(a);
                let (some, none, merge) = (
                    self.b.create_block(),
                    self.b.create_block(),
                    self.b.create_block(),
                );
                let void = is_void(&rt);
                let r = (!void).then(|| {
                    let ty = self.g.cty(&rt);
                    self.b.append_block_param(merge, ty)
                });
                self.brif(t, some, none);
                self.b.switch_to_block(some);
                if void {
                    self.jump(merge, &[]);
                } else {
                    let raw = self.load(types::I64, t, 0);
                    let v = self.unkv(raw, &inner);
                    let v = self.convert(v, &inner, &rt);
                    self.jump(merge, &[v]);
                }
                self.b.switch_to_block(none);
                let rv = self.expr(b);
                if void {
                    self.jump(merge, &[]);
                } else {
                    let v = if is_void(&b.ty) {
                        self.zero(&rt)
                    } else {
                        self.convert(rv, &b.ty, &rt)
                    };
                    self.jump(merge, &[v]);
                }
                self.b.switch_to_block(merge);
                r.unwrap_or_else(|| self.iconst(0))
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
                let n = self.iconst(vals.len() as i64);
                let arr = if vals.is_empty() {
                    self.iconst(0)
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
                let (h, q) = (self.func_addr(hash), self.func_addr(eq));
                self.call_rt("k_map_new", &[h, q])
            }
            ExprKind::Struct { adt, fields } => {
                if fields.is_empty() {
                    return self.iconst(0);
                }
                let decls: Vec<Ty> = match &self.g.p.adts[*adt].kind {
                    AdtKind::Struct(fs) => fs.iter().map(|f| f.ty.clone()).collect(),
                    AdtKind::Enum(_) => Vec::new(),
                };
                let vals: Vec<Value> = fields
                    .iter()
                    .zip(&decls)
                    .map(|(f, ty)| {
                        let v = self.expr(f);
                        self.kv_of(v, ty)
                    })
                    .collect();
                let s = self.alloc(vals.len());
                for (i, v) in vals.into_iter().enumerate() {
                    self.store(v, s, 8 * i as i32);
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
                let tag = self.iconst(*index as i64);
                let n = self.b.ins().iconst(types::I32, kvs.len() as i64);
                let arr = if kvs.is_empty() {
                    self.iconst(0)
                } else {
                    self.stack_array(&kvs)
                };
                self.call_rt("k_enum", &[tag, n, arr])
            }
            ExprKind::Field { base, field } => {
                let b = self.expr(base);
                let raw = self.load(types::I64, b, 8 * *field as i32);
                self.unkv(raw, &e.ty)
            }
            ExprKind::Record(values) => {
                let vals: Vec<Value> = values
                    .iter()
                    .map(|v| {
                        let x = self.expr(v);
                        self.kv_of(x, &v.ty)
                    })
                    .collect();
                let n = self.b.ins().iconst(types::I32, vals.len() as i64);
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
                let vals: Vec<Value> = args.iter().map(|a| self.expr(a)).collect();
                let f = self.g.funcs[*func];
                match self.call(f, &vals) {
                    Some(v) => v,
                    None => self.iconst(0),
                }
            }
            ExprKind::CallClosure { callee, args } => {
                let Ty::Fn(f) = &callee.ty else {
                    self.g.unsupported(e.span, "this call");
                    return self.iconst(0);
                };
                let f = f.clone();
                let c = self.expr(callee);
                let mut kvs = Vec::new();
                for (a, ty) in args.iter().zip(&f.params) {
                    let v = self.expr(a);
                    kvs.push(self.kv_of(v, ty));
                }
                if kvs.is_empty() {
                    kvs.push(self.iconst(0));
                }
                let arr = self.stack_array(&kvs);
                let func = self.load(types::I64, c, 0);
                let env = self.load(types::I64, c, 8);
                let sigref = self.b.import_signature(sig(
                    self.g.cc,
                    &[types::I64, types::I64],
                    Some(types::I64),
                ));
                let call = self.b.ins().call_indirect(sigref, func, &[env, arr]);
                let raw = self.b.inst_results(call)[0];
                if is_void(&f.ret) {
                    return self.iconst(0);
                }
                self.unkv(raw, &f.ret)
            }
            ExprKind::Method { name, recv, args } => {
                let r = self.expr(recv);
                let vals: Vec<Value> = args.iter().map(|a| self.expr(a)).collect();
                self.builtin_method(r, &recv.ty, name, &vals, args, e.span)
            }
            ExprKind::Lib { module, name, args } => {
                let vals: Vec<Value> = args.iter().map(|a| self.expr(a)).collect();
                let tys: Vec<Ty> = args.iter().map(|a| a.uncoerced().ty.clone()).collect();
                self.lib_call(module, name, &vals, &tys, &e.ty, e.span)
            }
            ExprKind::Print(v) => {
                let x = self.expr(v);
                self.call_rt("k_print", &[x])
            }
            ExprKind::Env(v) => {
                let x = self.expr(v);
                self.call_rt("k_env", &[x])
            }
            ExprKind::Panic(v) => {
                let x = self.expr(v);
                self.call_rt("k_panic", &[x])
            }
            ExprKind::Log { level, args } => {
                let mut shown: Vec<Value> = args.iter().map(|a| self.expr(a)).collect();
                let message = shown.remove(0);
                let lv = self.c_str(level);
                let n = self.b.ins().iconst(types::I32, shown.len() as i64);
                let data = if shown.is_empty() {
                    self.iconst(0)
                } else {
                    self.stack_array(&shown)
                };
                self.call_rt("k_log", &[lv, message, n, data])
            }
            ExprKind::Closure { func, captures } => {
                let f = self.g.funcs[*func];
                let fa = self.func_addr(f);
                if captures.is_empty() {
                    let null = self.iconst(0);
                    return self.call_rt("k_closure", &[fa, null]);
                }
                let env = self.alloc(captures.len());
                for (i, &l) in captures.iter().enumerate() {
                    let v = match self.locals[l] {
                        Slot::Cell(var, _) => self.b.use_var(var),
                        Slot::Var(_) => {
                            let v = self.local(l);
                            let ty = self.tys[l].clone();
                            self.kv_of(v, &ty)
                        }
                    };
                    self.store(v, env, 8 * i as i32);
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
                let (fail, merge) = (self.b.create_block(), self.b.create_block());
                let r = self.b.append_block_param(merge, types::I8);
                self.pattern(pattern, s, &vt, fail);
                let yes = self.b.ins().iconst(types::I8, 1);
                self.jump(merge, &[yes]);
                self.b.switch_to_block(fail);
                let no = self.b.ins().iconst(types::I8, 0);
                self.jump(merge, &[no]);
                self.b.switch_to_block(merge);
                r
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
                self.iconst(0)
            }
            ExprKind::Coerce(v) => {
                let x = self.expr(v);
                self.convert(x, &v.ty, &e.ty)
            }
            ExprKind::Error => self.iconst(0),
        }
    }

    fn if_chain(
        &mut self,
        cond: &Expr,
        then: &Block,
        otherwise: Option<&Block>,
        rt: Option<&Ty>,
    ) -> Value {
        let c = self.expr(cond);
        let (then_b, else_b, merge) = (
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
        );
        let r = rt.map(|t| {
            let ty = self.g.cty(t);
            self.b.append_block_param(merge, ty)
        });
        self.brif(c, then_b, else_b);
        self.b.switch_to_block(then_b);
        let v = self.block(then, rt.is_some());
        self.branch_to(merge, v, &block_ty(then), rt);
        self.b.switch_to_block(else_b);
        match otherwise {
            Some(other) => {
                let v = self.block(other, rt.is_some());
                self.branch_to(merge, v, &block_ty(other), rt);
            }
            None => {
                let v = self.iconst(0);
                self.branch_to(merge, v, &Ty::Unit, rt);
            }
        }
        self.b.switch_to_block(merge);
        r.unwrap_or_else(|| self.iconst(0))
    }

    /// Jumps to a merge block, passing the branch's value moved to `rt`
    /// when the merge block takes one.
    fn branch_to(&mut self, merge: ir::Block, v: Value, vt: &Ty, rt: Option<&Ty>) {
        match rt {
            Some(rt) => {
                let v = if is_void(vt) {
                    self.zero(rt)
                } else {
                    self.convert(v, vt, rt)
                };
                self.jump(merge, &[v]);
            }
            None => self.jump(merge, &[]),
        }
    }

    fn match_expr(&mut self, e: &Expr, scrutinee: &Expr, arms: &[Arm], want: bool) -> Value {
        let rt = e.ty.clone();
        let st = scrutinee.ty.clone();
        let s = self.expr(scrutinee);
        let merge = self.b.create_block();
        let keep = want && !is_void(&rt);
        let r = keep.then(|| {
            let ty = self.g.cty(&rt);
            self.b.append_block_param(merge, ty)
        });
        let rt_opt = keep.then_some(&rt);
        for arm in arms {
            let next = self.b.create_block();
            self.pattern(&arm.pattern, s, &st, next);
            if let Some(guard) = &arm.guard {
                let g = self.expr(guard);
                let body = self.b.create_block();
                self.brif(g, body, next);
                self.b.switch_to_block(body);
            }
            if keep {
                let v = self.expr(&arm.body);
                self.branch_to(merge, v, &arm.body.ty, rt_opt);
            } else {
                self.discard(&arm.body);
                self.jump(merge, &[]);
            }
            self.b.switch_to_block(next);
        }
        self.panic_c("no arm of this match matched");
        self.b.switch_to_block(merge);
        r.unwrap_or_else(|| self.iconst(0))
    }

    /// Tests the value `val` of type `ty` against a pattern, jumping to
    /// `fail` when it does not match, and binds the pattern's names when it
    /// does.
    fn pattern(&mut self, p: &Pattern, val: Value, ty: &Ty, fail: ir::Block) {
        match p {
            Pattern::Wildcard => {}
            Pattern::Bind(l) => self.set_local(*l, val),
            Pattern::Literal(lit) => {
                let l = self.expr(lit);
                let eq = self.eq_expr(val, ty, l, &lit.ty, lit.span);
                let ok = self.b.create_block();
                self.brif(eq, ok, fail);
                self.b.switch_to_block(ok);
            }
            Pattern::None => {
                let ok = self.b.create_block();
                self.brif(val, fail, ok);
                self.b.switch_to_block(ok);
            }
            Pattern::Some(inner, ity) => {
                let ok = self.b.create_block();
                self.brif(val, ok, fail);
                self.b.switch_to_block(ok);
                let raw = self.load(types::I64, val, 0);
                let v = self.unkv(raw, ity);
                self.pattern(inner, v, ity, fail);
            }
            Pattern::Variant { index, fields, .. } => {
                let tag = self.load(types::I64, val, 0);
                let same = self.b.ins().icmp_imm_s(IntCC::Equal, tag, *index as i64);
                let ok = self.b.create_block();
                self.brif(same, ok, fail);
                self.b.switch_to_block(ok);
                for (i, (sub, fty)) in fields.iter().enumerate() {
                    let raw = self.load(types::I64, val, 8 + 8 * i as i32);
                    let v = self.unkv(raw, fty);
                    self.pattern(sub, v, fty, fail);
                }
            }
        }
    }

    // ----- showing and comparing -----

    /// A `kstr *` showing the value, as `print` and interpolation do.
    fn display(&mut self, v: Value, ty: &Ty, span: Span) -> Value {
        let p = self.g.prims;
        match ty {
            _ if self.g.is_string(ty) => v,
            Ty::Adt(id, _) if *id == p.int => self.call_rt("k_int_str", &[v]),
            Ty::Adt(id, _) if *id == p.float => self.call_rt("k_float_str", &[v]),
            Ty::Adt(id, _) if *id == p.bool_ => self.call_rt("k_bool_str", &[v]),
            Ty::Adt(id, _) if *id == p.duration => self.call_rt("k_duration_str", &[v]),
            _ => {
                self.g.check(Op::Show, ty, span);
                let d = self.g.desc(ty);
                let addr = self.data_addr(d);
                let k = self.kv_of(v, ty);
                let q = self.b.ins().iconst(types::I8, 0);
                self.call_rt("k_desc_show", &[addr, k, q])
            }
        }
    }

    /// Whether two values are equal.
    fn eq_expr(&mut self, a: Value, at: &Ty, b: Value, bt: &Ty, span: Span) -> Value {
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
        if self.g.is_string(&at) {
            return self.call_rt("k_str_eq", &[a, b]);
        }
        match self.g.rep(&at) {
            Rep::Float => self.b.ins().fcmp(FloatCC::Equal, a, b),
            Rep::Int | Rep::Bool => {
                let t = self.b.func.dfg.value_type(a);
                let b = self.fit(b, t);
                self.b.ins().icmp(IntCC::Equal, a, b)
            }
            _ => {
                self.g.check(Op::Eq, &at, span);
                let d = self.g.desc(&at);
                let addr = self.data_addr(d);
                let (x, y) = (self.kv_of(a, &at), self.kv_of(b, &at));
                self.call_rt("k_desc_eq", &[addr, x, y])
            }
        }
    }

    // ----- standard library methods -----

    fn builtin_method(
        &mut self,
        r: Value,
        recv_ty: &Ty,
        name: &str,
        a: &[Value],
        args: &[Expr],
        span: Span,
    ) -> Value {
        let Ty::Adt(id, targs) = recv_ty else {
            return self.unsupported(span, format!("`.{name}`"));
        };
        let id = *id;
        let arg = |i: usize| a[i];
        let p = self.g.prims;
        if id == p.int {
            return match name {
                "to_string" => self.call_rt("k_int_str", &[r]),
                "to_float" => self.b.ins().fcvt_from_sint(types::F64, r),
                "abs" => self.call_rt("k_abs", &[r]),
                "min" => self.call_rt("k_min", &[r, arg(0)]),
                "max" => self.call_rt("k_max", &[r, arg(0)]),
                "pow" => self.call_rt("k_pow", &[r, arg(0)]),
                "clamp" => self.call_rt("k_clamp", &[r, arg(0), arg(1)]),
                _ => self.unsupported(span, format!("`Int.{name}`")),
            };
        }
        if id == p.duration {
            return match name {
                "to_millis" => self.b.ins().sdiv_imm_s(r, 1_000_000),
                "to_seconds" => {
                    let f = self.b.ins().fcvt_from_sint(types::F64, r);
                    let d = self.b.ins().f64const(1e9);
                    self.b.ins().fdiv(f, d)
                }
                _ => self.unsupported(span, format!("`Duration.{name}`")),
            };
        }
        if self.is_std_adt(id, "Instant") {
            let off = 8 * self.g.field_index(id, "unix_nanos") as i32;
            let ns = self.load(types::I64, r, off);
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
                    let other = self.load(types::I64, arg(0), off);
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
                "abs" => self.b.ins().fabs(r),
                "sqrt" => self.b.ins().sqrt(r),
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
                    let len = self.load(types::I64, r, 0);
                    return self.b.ins().icmp_imm_s(IntCC::Equal, len, 0);
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
                "len" => return self.load(types::I64, r, 0),
                "is_empty" => {
                    let len = self.load(types::I64, r, 0);
                    return self.b.ins().icmp_imm_s(IntCC::Equal, len, 0);
                }
                "contains" | "index_of" => {
                    let eq = self.g.eq_fn(&elem, span);
                    let fa = self.func_addr(eq);
                    let f = if name == "contains" {
                        "k_list_contains"
                    } else {
                        "k_list_index_of"
                    };
                    return self.call_rt(f, &[r, arg(0), fa]);
                }
                "sort" => {
                    let cmp = self.g.cmp_fn(&elem, span);
                    let fa = self.func_addr(cmp);
                    return self.call_rt("k_list_sort", &[r, fa]);
                }
                "min" | "max" => {
                    let cmp = self.g.cmp_fn(&elem, span);
                    let fa = self.func_addr(cmp);
                    let sign = self
                        .b
                        .ins()
                        .iconst(types::I32, if name == "min" { 1 } else { -1 });
                    return self.call_rt("k_list_extreme", &[r, fa, sign]);
                }
                "sort_by" => {
                    let key = match args.first().map(|a| &a.uncoerced().ty) {
                        Some(Ty::Fn(f)) => f.ret.clone(),
                        _ => Ty::Unknown,
                    };
                    let cmp = self.g.cmp_fn(&key, span);
                    let fa = self.func_addr(cmp);
                    return self.call_rt("k_list_sort_by", &[r, arg(0), fa]);
                }
                "join" => {
                    let show = self.g.show_fn(&elem, false, span);
                    let fa = self.func_addr(show);
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
                "len" => return self.load(types::I64, r, 0),
                "is_empty" => {
                    let len = self.load(types::I64, r, 0);
                    return self.b.ins().icmp_imm_s(IntCC::Equal, len, 0);
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

    fn unsupported(&mut self, span: Span, what: String) -> Value {
        self.g.unsupported(span, what);
        self.iconst(0)
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
    fn new_instant(&mut self, ns: Value, ty: &Ty) -> Value {
        let Ty::Adt(id, _) = ty else {
            return self.iconst(0);
        };
        let n = self.g.field_count(*id);
        let t = self.alloc(n);
        let off = 8 * self.g.field_index(*id, "unix_nanos") as i32;
        self.store(ns, t, off);
        t
    }

    /// A new `Date` of type `ty`: the UTC date of the instant `ns`.
    fn new_date(&mut self, ns: Value, ty: &Ty) -> Value {
        let Ty::Adt(id, _) = ty else {
            return self.iconst(0);
        };
        let n = self.g.field_count(*id);
        let t = self.alloc(n);
        for (part, f) in [
            ("year", "k_time_year"),
            ("month", "k_time_month"),
            ("day", "k_time_day"),
        ] {
            let v = self.call_rt(f, &[ns]);
            let off = 8 * self.g.field_index(*id, part) as i32;
            self.store(v, t, off);
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
        a: &[Value],
        tys: &[Ty],
        ty: &Ty,
        span: Span,
    ) -> Value {
        let (v, fallible) = match (module, name) {
            ("time", "now") => {
                let n = self.call_rt("k_time_now", &[]);
                (self.new_instant(n, ty), false)
            }
            ("time", "unix") => {
                let sec = self.iconst(1_000_000_000);
                let ns = self.call_rt("k_mul", &[a[0], sec]);
                (self.new_instant(ns, ty), false)
            }
            ("time", "since") => {
                let now = self.call_rt("k_time_now", &[]);
                let instant = self.g.p.adts.iter().position(|d| d.name == "Instant");
                let off = instant.map_or(0, |id| 8 * self.g.field_index(id, "unix_nanos") as i32);
                let then = self.load(types::I64, a[0], off);
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
                    "env" => "k_os_env",
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
                    "print_err" => "k_io_print_err",
                    _ => return self.unsupported(span, format!("`{module}.{name}`")),
                };
                (self.call_rt(f, a), false)
            }
            ("html", "escape") => (self.call_rt("k_rt_html_escape", a), false),
            ("html", "safe_url") => (self.call_rt("k_rt_html_safe_url", a), false),
            ("base64", "encode") => (self.call_rt("k_rt_base64_encode", a), false),
            ("base64", "decode") => (self.call_rt("k_base64_decode", a), true),
            ("json", "encode") => {
                let d = self.g.desc(&tys[0]);
                let addr = self.data_addr(d);
                (self.call_rt("k_json_encode", &[addr, a[0]]), false)
            }
            ("json", "decode") | ("web", "Request.json") => {
                let d = self.g.desc(ty);
                let addr = self.data_addr(d);
                let f = if module == "json" {
                    "k_json_decode"
                } else {
                    "k_web_req_json"
                };
                let raw = self.call_rt(f, &[addr, a[0]]);
                (self.unkv(raw, ty), true)
            }
            ("web", "json") => {
                let d = self.g.desc(&tys[0]);
                let addr = self.data_addr(d);
                (self.call_rt("k_web_json", &[addr, a[0], a[1]]), false)
            }
            ("web", "App.listen") => (self.call_rt("k_web_listen", a), true),
            ("web", _) => {
                let f: &'static str = match name {
                    "app" => "k_web_app",
                    "not_found" => "k_web_not_found",
                    "error" => "k_web_error",
                    "App.on_error" => "k_web_on_error",
                    "text" => "k_web_text",
                    "html" => "k_web_html",
                    "redirect" => "k_web_redirect",
                    "Request.query" => "k_web_query",
                    "Request.header" => "k_web_header",
                    "Request.cookie" => "k_web_cookie",
                    "App.get" | "App.post" | "App.put" | "App.patch" | "App.delete" => {
                        let index = ["get", "post", "put", "patch", "delete"]
                            .iter()
                            .position(|m| name.ends_with(&format!(".{m}")))
                            .unwrap_or(0);
                        let method = self.iconst(index as i64);
                        return self.call_rt("k_web_route", &[a[0], method, a[1], a[2]]);
                    }
                    _ => return self.unsupported(span, format!("`{module}.{name}`")),
                };
                (self.call_rt(f, a), false)
            }
            _ => return self.unsupported(span, format!("`{module}.{name}`")),
        };
        if fallible {
            let d = self.g.data_import("k_lib_error");
            let addr = self.data_addr(d);
            let message = self.load(types::I64, addr, 0);
            let (bad, ok) = (self.b.create_block(), self.b.create_block());
            self.brif(message, bad, ok);
            self.b.switch_to_block(bad);
            let w = self.new_error(message);
            let null = self.iconst(0);
            self.store(null, addr, 0);
            let show = self.g.error_message_fn();
            let fa = self.func_addr(show);
            self.call_rt("k_fail", &[w, fa]);
            self.jump(ok, &[]);
            self.b.switch_to_block(ok);
        }
        v
    }
}

fn sig(cc: CallConv, params: &[Type], ret: Option<Type>) -> Signature {
    let mut s = Signature::new(cc);
    s.params.extend(params.iter().map(|&t| AbiParam::new(t)));
    s.returns.extend(ret.map(AbiParam::new));
    s
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
