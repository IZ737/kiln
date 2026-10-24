//! A readable text form of the IR, printed by `kiln build --emit-ir`.
//!
//! Locals print as `name#id`, constants as `@NAME`, and coercions as
//! `(value as Type)`.

use crate::*;
use std::fmt;

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut p = Printer {
            prog: self,
            func: None,
            out: String::new(),
            depth: 0,
        };
        for g in &self.globals {
            p.out
                .push_str(&format!("const {}: {}\n", g.name, p.ty(&g.ty)));
        }
        if !self.globals.is_empty() {
            p.out.push('\n');
        }
        for (id, func) in self.funcs.iter().enumerate() {
            p.func(id, func);
        }
        f.write_str(p.out.trim_end())?;
        f.write_str("\n")
    }
}

struct Printer<'a> {
    prog: &'a Program,
    func: Option<&'a Func>,
    out: String,
    depth: usize,
}

impl<'a> Printer<'a> {
    fn ty(&self, ty: &Ty) -> String {
        match ty {
            Ty::Adt(id, args) => {
                let name = &self.prog.adts[*id].name;
                if args.is_empty() {
                    name.clone()
                } else {
                    let args: Vec<String> = args.iter().map(|a| self.ty(a)).collect();
                    format!("{name}<{}>", args.join(", "))
                }
            }
            Ty::Opt(inner) => format!("{}?", self.ty(inner)),
            Ty::Fn(f) => {
                let params: Vec<String> = f.params.iter().map(|p| self.ty(p)).collect();
                let mut s = format!("fn({}) -> {}", params.join(", "), self.ty(&f.ret));
                if let Some(t) = &f.throws {
                    s.push_str(&format!(" ! {}", self.ty(t)));
                }
                s
            }
            Ty::Record(fields) => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|(n, t)| format!("{n}: {}", self.ty(t)))
                    .collect();
                format!("{{ {} }}", fields.join(", "))
            }
            Ty::Param(name) => name.clone(),
            Ty::Var(n) => format!("?{n}"),
            Ty::Unit => "Unit".into(),
            Ty::Never => "Never".into(),
            Ty::Any => "Any".into(),
            Ty::Rest => "...".into(),
            Ty::Unknown => "?".into(),
        }
    }

    fn func_name(&self, id: FuncId) -> String {
        let func = &self.prog.funcs[id];
        let mut name = match func.kind {
            FuncKind::Method(adt) => format!("{}.{}", self.prog.adts[adt].name, func.name),
            FuncKind::Closure => format!("closure#{id}"),
            FuncKind::Init | FuncKind::Fn => func.name.clone(),
        };
        if !func.type_args.is_empty() {
            let args: Vec<String> = func.type_args.iter().map(|t| self.ty(t)).collect();
            name.push_str(&format!("<{}>", args.join(", ")));
        }
        name
    }

    fn local(&self, id: LocalId) -> String {
        match self.func {
            Some(f) => format!("{}#{id}", f.locals[id].name),
            None => format!("#{id}"),
        }
    }

    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn func(&mut self, id: FuncId, func: &'a Func) {
        self.func = Some(func);
        let params: Vec<String> = func
            .params
            .iter()
            .map(|&p| format!("{}: {}", self.local(p), self.ty(&func.locals[p].ty)))
            .collect();
        let mut head = format!("fn {}({})", self.func_name(id), params.join(", "));
        if !matches!(func.ret, Ty::Unit) {
            head.push_str(&format!(" -> {}", self.ty(&func.ret)));
        }
        if let Some(t) = &func.throws {
            head.push_str(&format!(" ! {}", self.ty(t)));
        }
        if !func.captures.is_empty() {
            let caps: Vec<String> = func.captures.iter().map(|&c| self.local(c)).collect();
            head.push_str(&format!(" captures [{}]", caps.join(", ")));
        }
        let cells: Vec<String> = (0..func.locals.len())
            .filter(|&i| func.locals[i].cell)
            .map(|i| self.local(i))
            .collect();
        if !cells.is_empty() {
            head.push_str(&format!(" cells [{}]", cells.join(", ")));
        }
        let body = self.block(&func.body);
        self.line(&format!("{head} {body}"));
        self.out.push('\n');
    }

    /// A block, as text that starts with `{` and ends with `}`; lines inside
    /// are indented one level deeper than the current line.
    fn block(&mut self, b: &Block) -> String {
        let saved = std::mem::take(&mut self.out);
        self.depth += 1;
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(v) = &b.value {
            let v = self.expr(v);
            self.line(&v);
        }
        self.depth -= 1;
        let inner = std::mem::replace(&mut self.out, saved);
        if inner.is_empty() {
            return "{}".into();
        }
        let mut s = String::from("{\n");
        s.push_str(&inner);
        for _ in 0..self.depth {
            s.push_str("    ");
        }
        s.push('}');
        s
    }

    fn stmt(&mut self, s: &Stmt) {
        let text = match s {
            Stmt::Let(l, v) => {
                let f = self.func.expect("inside a function");
                let kw = if f.locals[*l].mutable { "var" } else { "let" };
                let ty = self.ty(&f.locals[*l].ty);
                format!("{kw} {}: {ty} = {}", self.local(*l), self.expr(v))
            }
            Stmt::SetGlobal(g, v) => format!("@{} = {}", self.prog.globals[*g].name, self.expr(v)),
            Stmt::Assign { place, op, value } => {
                let place = match place {
                    Place::Local(l) => self.local(*l),
                    Place::Field { base, field, .. } => self.field(base, *field),
                    Place::ListItem { list, index, .. } => {
                        format!("{}[{}]", self.expr(list), self.expr(index))
                    }
                    Place::MapEntry { map, key } => {
                        format!("{}[{}]", self.expr(map), self.expr(key))
                    }
                };
                let op = op.map(arith).unwrap_or("");
                format!("{place} {op}= {}", self.expr(value))
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::Return(None) => "return".into(),
            Stmt::Return(Some(v)) => format!("return {}", self.expr(v)),
            Stmt::Break => "break".into(),
            Stmt::Continue => "continue".into(),
            Stmt::While { cond, body } => {
                let c = self.expr(cond);
                format!("while {c} {}", self.block(body))
            }
            Stmt::ForRange {
                var,
                lo,
                hi,
                filter,
                body,
            } => {
                let head = format!(
                    "for {} in {}..{}",
                    self.local(*var),
                    self.expr(lo),
                    self.expr(hi)
                );
                let filter = filter
                    .as_ref()
                    .map(|f| format!(" where {}", self.expr(f)))
                    .unwrap_or_default();
                format!("{head}{filter} {}", self.block(body))
            }
            Stmt::ForEach {
                var,
                iter,
                filter,
                body,
            } => {
                let head = format!("for {} in {}", self.local(*var), self.expr(iter));
                let filter = filter
                    .as_ref()
                    .map(|f| format!(" where {}", self.expr(f)))
                    .unwrap_or_default();
                format!("{head}{filter} {}", self.block(body))
            }
        };
        self.line(&text);
    }

    fn field(&mut self, base: &Expr, field: usize) -> String {
        let name = match &base.ty {
            Ty::Adt(id, _) => match &self.prog.adts[*id].kind {
                AdtKind::Struct(fields) => fields[field].name.clone(),
                AdtKind::Enum(_) => field.to_string(),
            },
            _ => field.to_string(),
        };
        format!("{}.{name}", self.expr(base))
    }

    fn list(&mut self, items: &[Expr]) -> String {
        let items: Vec<String> = items.iter().map(|e| self.expr(e)).collect();
        items.join(", ")
    }

    fn expr(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(n) => n.to_string(),
            ExprKind::Float(x) => format!("{x:?}"),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(s) => format!("{s:?}"),
            ExprKind::None => "none".into(),
            ExprKind::Local(l) => self.local(*l),
            ExprKind::Global(g) => format!("@{}", self.prog.globals[*g].name),
            ExprKind::FuncRef(f) => format!("&{}", self.func_name(*f)),
            ExprKind::Concat(parts) => format!("concat({})", self.list(parts)),
            ExprKind::Show(v) => format!("show({})", self.expr(v)),
            ExprKind::Not(v) => format!("!{}", self.expr(v)),
            ExprKind::Neg(v) => format!("-{}", self.expr(v)),
            ExprKind::Arith(op, a, b) => {
                format!("({} {} {})", self.expr(a), arith(*op), self.expr(b))
            }
            ExprKind::Cmp(op, a, b) => {
                let op = match op {
                    CmpOp::Lt => "<",
                    CmpOp::Le => "<=",
                    CmpOp::Gt => ">",
                    CmpOp::Ge => ">=",
                };
                format!("({} {op} {})", self.expr(a), self.expr(b))
            }
            ExprKind::Eq(a, b) => format!("({} == {})", self.expr(a), self.expr(b)),
            ExprKind::IsNone(v) => format!("is_none({})", self.expr(v)),
            ExprKind::And(a, b) => format!("({} && {})", self.expr(a), self.expr(b)),
            ExprKind::Or(a, b) => format!("({} || {})", self.expr(a), self.expr(b)),
            ExprKind::Coalesce(a, b) => format!("({} ?? {})", self.expr(a), self.expr(b)),
            ExprKind::Range(a, b) => format!("({}..{})", self.expr(a), self.expr(b)),
            ExprKind::List(items) => format!("[{}]", self.list(items)),
            ExprKind::NewMap => format!("{}()", self.ty(&e.ty)),
            ExprKind::Struct { adt, fields } => {
                let def = &self.prog.adts[*adt];
                let names: Vec<String> = match &def.kind {
                    AdtKind::Struct(fs) => fs.iter().map(|f| f.name.clone()).collect(),
                    AdtKind::Enum(_) => Vec::new(),
                };
                let parts: Vec<String> = fields
                    .iter()
                    .zip(names)
                    .map(|(v, n)| format!("{n}: {}", self.expr(v)))
                    .collect();
                format!("{}({})", def.name, parts.join(", "))
            }
            ExprKind::Variant { adt, index, fields } => {
                let def = &self.prog.adts[*adt];
                let name = match &def.kind {
                    AdtKind::Enum(vs) => vs[*index].name.clone(),
                    AdtKind::Struct(_) => index.to_string(),
                };
                if fields.is_empty() {
                    format!("{}.{name}", def.name)
                } else {
                    format!("{}.{name}({})", def.name, self.list(fields))
                }
            }
            ExprKind::Field { base, field } => self.field(base, *field),
            ExprKind::ListItem { list, index } => {
                format!("{}[{}]", self.expr(list), self.expr(index))
            }
            ExprKind::MapGet { map, key } => format!("{}[{}]", self.expr(map), self.expr(key)),
            ExprKind::Call { func, args } => {
                format!("{}({})", self.func_name(*func), self.list(args))
            }
            ExprKind::CallClosure { callee, args } => {
                format!("call({})({})", self.expr(callee), self.list(args))
            }
            ExprKind::Method { name, recv, args } => {
                format!("{}.{name}({})", self.expr(recv), self.list(args))
            }
            ExprKind::Print(v) => format!("print({})", self.expr(v)),
            ExprKind::Lib { module, name, args } => {
                format!("{module}.{name}({})", self.list(args))
            }
            ExprKind::Env(v) => format!("env({})", self.expr(v)),
            ExprKind::Panic(v) => format!("panic({})", self.expr(v)),
            ExprKind::Log { level, args } => format!("log.{level}({})", self.list(args)),
            ExprKind::Closure { func, captures } => {
                let caps: Vec<String> = captures.iter().map(|&c| self.local(c)).collect();
                if caps.is_empty() {
                    format!("closure#{func}")
                } else {
                    format!("closure#{func}[{}]", caps.join(", "))
                }
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                let mut s = format!("if {} {}", self.expr(cond), self.block(then));
                if let Some(o) = otherwise {
                    s.push_str(&format!(" else {}", self.block(o)));
                }
                s
            }
            ExprKind::Match { scrutinee, arms } => {
                let head = format!("match {} {{", self.expr(scrutinee));
                let saved = std::mem::take(&mut self.out);
                self.depth += 1;
                for arm in arms {
                    let mut p = self.pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        p.push_str(&format!(" if {}", self.expr(g)));
                    }
                    let body = self.expr(&arm.body);
                    self.line(&format!("{p} => {body}"));
                }
                self.depth -= 1;
                let inner = std::mem::replace(&mut self.out, saved);
                let mut s = format!("{head}\n{inner}");
                for _ in 0..self.depth {
                    s.push_str("    ");
                }
                s.push('}');
                s
            }
            ExprKind::Is { value, pattern } => {
                format!("({} is {})", self.expr(value), self.pattern(pattern))
            }
            ExprKind::Block(b) => self.block(b),
            ExprKind::Try { expr, wrap } => {
                let kw = if *wrap { "try wrap" } else { "try" };
                format!("{kw} {}", self.expr(expr))
            }
            ExprKind::Catch {
                expr,
                binding,
                handler,
                ..
            } => {
                let b = binding
                    .map(|l| format!(" {}", self.local(l)))
                    .unwrap_or_default();
                format!("{} catch{b} {}", self.expr(expr), self.block(handler))
            }
            ExprKind::Throw { value, wrap } => {
                let kw = if *wrap { "throw wrap" } else { "throw" };
                format!("{kw} {}", self.expr(value))
            }
            ExprKind::Coerce(v) => format!("({} as {})", self.expr(v), self.ty(&e.ty)),
            ExprKind::Error => "<error>".into(),
        }
    }

    fn pattern(&mut self, p: &Pattern) -> String {
        match p {
            Pattern::Wildcard => "_".into(),
            Pattern::Bind(l) => self.local(*l),
            Pattern::Literal(e) => self.expr(e),
            Pattern::None => "none".into(),
            Pattern::Some(inner, _) => format!("some({})", self.pattern(inner)),
            Pattern::Variant { adt, index, fields } => {
                let name = match &self.prog.adts[*adt].kind {
                    AdtKind::Enum(vs) => vs[*index].name.clone(),
                    AdtKind::Struct(_) => index.to_string(),
                };
                if fields.is_empty() {
                    name
                } else {
                    let parts: Vec<String> = fields.iter().map(|(p, _)| self.pattern(p)).collect();
                    format!("{name}({})", parts.join(", "))
                }
            }
        }
    }
}

fn arith(op: ArithOp) -> &'static str {
    match op {
        ArithOp::Add => "+",
        ArithOp::Sub => "-",
        ArithOp::Mul => "*",
        ArithOp::Div => "/",
        ArithOp::Rem => "%",
    }
}
