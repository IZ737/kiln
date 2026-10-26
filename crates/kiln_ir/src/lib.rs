//! The typed intermediate representation (IR) between the type checker and
//! the backends.
//!
//! [`lower`] turns a checked program into an IR [`Program`]. Everything the
//! source leaves implicit is explicit here, so a backend never looks at the
//! syntax tree or the checker's tables:
//!
//! - Names are resolved: locals are numbered per function, and calls name
//!   the function they call.
//! - Generic functions and methods are compiled once per set of type
//!   arguments, so the types inside a function are concrete. Generic
//!   structs and enums, and the built-in `List`, `Map` and `Set`, are not:
//!   their fields and items of a type-parameter type live in slots.
//! - Closures, field shorthands (`.title`) and nothing else become
//!   functions of their own, with the locals they capture listed. A `var`
//!   that a closure captures is marked as living in a cell.
//! - Call arguments are in parameter order, with defaults filled in.
//! - Every value has a type. Where a value moves to a different type, such
//!   as an `Int` passed for a generic `T` or a `T` stored in a `T?`, the IR
//!   has a [`ExprKind::Coerce`] node; the backend decides what it costs.
//! - String interpolation is [`ExprKind::Concat`] of [`ExprKind::Show`]
//!   parts, and the error-wrapping rule of bare `!` is a flag on `try` and
//!   `throw`.
//!
//! The IR is still a tree with blocks, loops and `if`s rather than a flat
//! control-flow graph: the C backend maps it almost line for line, and
//! later backends can flatten it.
//!
//! ```
//! let parsed = kiln_syntax::parse("fn main() {\n    print(\"hi\")\n}\n");
//! let checked = kiln_check::check_program(&parsed.module);
//! let ir = kiln_ir::lower(&parsed.module, &checked).unwrap();
//! assert!(ir.to_string().contains("fn main()"));
//! ```

pub mod admin;
pub mod glaze;
mod lower;
mod print;
pub mod rt;

pub use kiln_check::types::{AdtDef, AdtId, AdtKind, FnTy, Ty};
pub use kiln_check::ValueKind;
pub use lower::lower;

use kiln_diagnostics::Span;

pub type FuncId = usize;
pub type LocalId = usize;
pub type GlobalId = usize;

/// A whole program.
pub struct Program {
    /// Every struct and enum, the standard library's first.
    pub adts: Vec<AdtDef>,
    /// How many of `adts` come from the standard library.
    pub std_adts: usize,
    /// For each of `adts`, whether it is a built-in type with no Kiln
    /// fields, such as `Int`, `String` or `List`, that the runtime
    /// implements.
    pub builtin: Vec<bool>,
    pub prims: Prims,
    /// Top-level constants.
    pub globals: Vec<Global>,
    pub funcs: Vec<Func>,
    /// Runs before `main` and sets every global, in source order.
    pub init: FuncId,
    pub main: FuncId,
}

/// The ids of the built-in types the backends treat specially.
#[derive(Clone, Copy, Debug)]
pub struct Prims {
    pub int: AdtId,
    pub float: AdtId,
    pub bool_: AdtId,
    pub string: AdtId,
    pub duration: AdtId,
    pub list: AdtId,
    pub map: AdtId,
    pub set: AdtId,
    pub error: AdtId,
}

pub struct Global {
    pub name: String,
    pub ty: Ty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuncKind {
    /// Sets the globals before `main` runs.
    Init,
    /// A top-level function.
    Fn,
    /// A method or type function of a struct or enum.
    Method(AdtId),
    /// A closure or field shorthand. Its arguments and captures arrive as
    /// values of their declared types, and generic ones are erased.
    Closure,
}

pub struct Func {
    /// The source name; `<closure>` for closures.
    pub name: String,
    pub kind: FuncKind,
    /// For a copy of a generic function or method: the types its type
    /// parameters stand for, the `impl` target's first. Empty otherwise.
    pub type_args: Vec<Ty>,
    pub params: Vec<LocalId>,
    /// For closures: the locals that receive the captured values, in the
    /// order of [`ExprKind::Closure::captures`].
    pub captures: Vec<LocalId>,
    pub ret: Ty,
    /// The error type, when the function can fail.
    pub throws: Option<Ty>,
    pub locals: Vec<Local>,
    pub body: Block,
    /// A `server fn`: it runs only on the server, and browser code calls
    /// it through a request.
    pub server: bool,
}

#[derive(Clone)]
pub struct Local {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
    /// The local lives in a heap cell shared with the closures that
    /// capture it: a captured `var`.
    pub cell: bool,
}

/// Statements, then an optional value: the value of the block.
#[derive(Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub value: Option<Box<Expr>>,
}

pub enum Stmt {
    /// Initializes a local, allocating its cell if it has one.
    Let(LocalId, Expr),
    SetGlobal(GlobalId, Expr),
    /// `place = value`, or `place op= value`. The value has the type of
    /// the place.
    Assign {
        place: Place,
        op: Option<ArithOp>,
        value: Expr,
    },
    /// An expression whose value is not used.
    Expr(Expr),
    Return(Option<Expr>),
    Break,
    Continue,
    While {
        cond: Expr,
        body: Block,
    },
    /// `for var in lo..hi`, excluding `hi`.
    ForRange {
        var: LocalId,
        lo: Expr,
        hi: Expr,
        filter: Option<Expr>,
        body: Block,
    },
    /// `for var in iter` over a `List` or a `Set`. A set is looped over
    /// through a snapshot of its items.
    ForEach {
        var: LocalId,
        iter: Expr,
        filter: Option<Expr>,
        body: Block,
    },
}

pub enum Place {
    Local(LocalId),
    /// A struct field. `ty` is the field's type at this use, which differs
    /// from its declared type in generic structs.
    Field {
        base: Expr,
        field: usize,
        ty: Ty,
    },
    ListItem {
        list: Expr,
        index: Expr,
        ty: Ty,
    },
    /// `map[key] = value`. With an operator, such as `map[key] += 1`,
    /// the key must already be in the map, or the program panics.
    MapEntry {
        map: Expr,
        key: Expr,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
}

pub struct Expr {
    pub kind: ExprKind,
    pub ty: Ty,
    pub span: Span,
}

pub enum ExprKind {
    /// An `Int` or, in nanoseconds, a `Duration`.
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    None,
    Local(LocalId),
    Global(GlobalId),
    /// A top-level function used as a value.
    FuncRef(FuncId),
    /// Joins `String` values.
    Concat(Vec<Expr>),
    /// The value as `print` and interpolation show it, as a `String`.
    Show(Box<Expr>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    /// Arithmetic on `Int`, `Float` or `Duration`, or `+` on `String`.
    /// `Int` arithmetic panics on overflow and division by zero.
    Arith(ArithOp, Box<Expr>, Box<Expr>),
    Cmp(CmpOp, Box<Expr>, Box<Expr>),
    /// Deep equality of two values of the same type.
    Eq(Box<Expr>, Box<Expr>),
    IsNone(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    /// `opt ?? default`. The default has the type of the whole expression.
    Coalesce(Box<Expr>, Box<Expr>),
    /// `lo..hi` as a list.
    Range(Box<Expr>, Box<Expr>),
    /// Items have the list's item type.
    List(Vec<Expr>),
    /// An anonymous record, `{ name: "x", n: 2 }`: field values in the
    /// order of the record type, each moved to `Any` (a slot).
    Record(Vec<Expr>),
    /// An empty `Map` or `Set` of the expression's type.
    NewMap,
    /// Field values in declaration order, with the declared field types.
    Struct {
        adt: AdtId,
        fields: Vec<Expr>,
    },
    /// Field values with the declared field types.
    Variant {
        adt: AdtId,
        index: usize,
        fields: Vec<Expr>,
    },
    /// The expression's type is the field's declared type, or `Any` for
    /// a record's field, which lives in a slot.
    Field {
        base: Box<Expr>,
        field: usize,
    },
    ListItem {
        list: Box<Expr>,
        index: Box<Expr>,
    },
    /// `map[key]`, an optional.
    MapGet {
        map: Box<Expr>,
        key: Box<Expr>,
    },
    /// The expression's type is the function's declared return type;
    /// arguments have the declared parameter types, a method's receiver
    /// first.
    Call {
        func: FuncId,
        args: Vec<Expr>,
    },
    /// The expression's type is the declared return type of the callee's
    /// function type.
    CallClosure {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    /// A standard library method of a built-in type, such as
    /// `List.push`. Types are as for [`ExprKind::Call`].
    Method {
        name: String,
        recv: Box<Expr>,
        args: Vec<Expr>,
    },
    /// A function of a standard library module that the backend builds,
    /// such as `fs.read`, or a method of one of its types, such as
    /// `App.get`, with the receiver first. Types are as for
    /// [`ExprKind::Call`], except that a generic function's type is the
    /// one at this call (`json.decode<Post>` has type `Post`). An `Any`
    /// argument is coerced, and the backend reads its type from under the
    /// coercion. When the function can fail, it fails with the standard
    /// `Error`.
    Lib {
        module: String,
        name: String,
        args: Vec<Expr>,
    },
    Print(Box<Expr>),
    Env(Box<Expr>),
    Panic(Box<Expr>),
    /// `log.info(message, data...)`, each argument already shown.
    Log {
        level: String,
        args: Vec<Expr>,
    },
    Closure {
        func: FuncId,
        captures: Vec<LocalId>,
    },
    If {
        cond: Box<Expr>,
        then: Block,
        otherwise: Option<Block>,
    },
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<Arm>,
    },
    /// Whether the value matches; binds the pattern's names when it does.
    Is {
        value: Box<Expr>,
        pattern: Box<Pattern>,
    },
    Block(Block),
    /// A fallible call whose error goes to the caller. With `wrap`, the
    /// error is first wrapped in the standard `Error`.
    Try {
        expr: Box<Expr>,
        wrap: bool,
    },
    /// Runs the handler when the call fails, binding the error of type
    /// `err`.
    Catch {
        expr: Box<Expr>,
        err: Ty,
        binding: Option<LocalId>,
        handler: Block,
    },
    /// With `wrap`, the value is shown into the message of a standard
    /// `Error` first.
    Throw {
        value: Box<Expr>,
        wrap: bool,
    },
    /// The inner value, moved to the expression's type.
    Coerce(Box<Expr>),
    /// Stands for a part that could not be lowered; a diagnostic was
    /// reported.
    Error,
}

pub struct Arm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

pub enum Pattern {
    Wildcard,
    Bind(LocalId),
    /// Equal to the value, which has the scrutinee's type.
    Literal(Expr),
    None,
    /// `some(p)`, with the type of the optional's contents.
    Some(Box<Pattern>, Ty),
    /// An enum variant, with each field's type at this use.
    Variant {
        adt: AdtId,
        index: usize,
        fields: Vec<(Pattern, Ty)>,
    },
}

impl Expr {
    /// The expression under any coercions.
    pub fn uncoerced(&self) -> &Expr {
        match &self.kind {
            ExprKind::Coerce(inner) => inner.uncoerced(),
            _ => self,
        }
    }

    /// The type a runtime function is told this value has, for a value
    /// passed as `Any`. It is the type under the coercion to `Any`, which
    /// is not always the innermost one: a record's field is held as `Any`
    /// and read back with a coercion to its own type, so
    /// `((form.email as String) as Any)` describes a `String`.
    pub fn described(&self) -> &Ty {
        match &self.kind {
            ExprKind::Coerce(inner) if !matches!(inner.ty, Ty::Any | Ty::Unknown) => {
                inner.described()
            }
            _ => &self.ty,
        }
    }
}

impl Program {
    pub fn is_prim(&self, id: AdtId) -> bool {
        self.builtin[id]
    }
}
