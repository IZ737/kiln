//! The syntax tree produced by the parser.
//!
//! Every node carries the [`Span`] of the source it came from, so later
//! passes (name resolution, type checking) can report precise errors.

use kiln_diagnostics::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Module {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Import(Import),
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Impl(ImplDecl),
    Const(ConstDecl),
}

/// `import web, sql` or `import web.router`
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub paths: Vec<Vec<Ident>>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    pub is_pub: bool,
    /// `server fn`: callable from browser code as a typed RPC.
    pub is_server: bool,
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub throws: Throws,
    pub body: Block,
    pub span: Span,
}

/// What a function can fail with, written after `!`.
#[derive(Clone, Debug, PartialEq)]
pub enum Throws {
    /// No `!`: the function cannot fail.
    Never,
    /// A bare `!`: fails with the default error type.
    Default,
    /// `! DbError`
    Type(Box<TypeExpr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: Ident,
    /// Optional only for closure parameters and `self`.
    pub ty: Option<TypeExpr>,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub fields: Vec<Field>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: Ident,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub generics: Vec<Ident>,
    pub variants: Vec<Variant>,
    pub span: Span,
}

/// `Admin` or `Viewer(since: Date)`
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub name: Ident,
    pub fields: Vec<VariantField>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariantField {
    pub name: Option<Ident>,
    pub ty: TypeExpr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImplDecl {
    pub target: TypeExpr,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConstDecl {
    pub is_pub: bool,
    pub name: Ident,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    /// `Int`, `web.Error`, `List<Post>`
    Named {
        path: Vec<Ident>,
        args: Vec<TypeExpr>,
    },
    /// `User?`
    Optional(Box<TypeExpr>),
    /// `fn(Int, String) -> Bool`, with `!` when it can fail.
    Fn {
        params: Vec<TypeExpr>,
        ret: Option<Box<TypeExpr>>,
        throws: Throws,
    },
    /// `{ name: String, age: Int }`, an anonymous record type.
    Record(Vec<(Ident, TypeExpr)>),
    /// Produced after a syntax error so parsing can continue.
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Let {
        mutable: bool,
        name: Ident,
        ty: Option<TypeExpr>,
        value: Expr,
        span: Span,
    },
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    /// `for post in posts where post.published { ... }`
    For {
        binding: Ident,
        iter: Expr,
        filter: Option<Expr>,
        body: Block,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(String),
    Float(String),
    Duration(String),
    Str(Vec<StrPiece>),
    Bool(bool),
    None,
    Ident(String),
    List(Vec<Expr>),
    /// `{ posts, title: "Home" }`
    Record(Vec<RecordField>),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `db.query<Post>(sql, id)` or `web.json(post, status: 201)`
    Call {
        callee: Box<Expr>,
        type_args: Vec<TypeExpr>,
        args: Vec<Arg>,
    },
    Field {
        expr: Box<Expr>,
        name: Ident,
    },
    Index {
        expr: Box<Expr>,
        index: Box<Expr>,
    },
    /// `.title`, shorthand for `fn(x) { x.title }`
    FieldShorthand(Ident),
    Closure {
        params: Vec<Param>,
        ret: Option<TypeExpr>,
        body: Block,
    },
    If {
        cond: Box<Expr>,
        then: Block,
        otherwise: Option<Box<Expr>>,
    },
    Block(Block),
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    /// `try expr`: propagate the error to the caller.
    Try(Box<Expr>),
    /// `expr catch err { handler }`
    Catch {
        expr: Box<Expr>,
        binding: Option<Ident>,
        handler: Block,
    },
    Throw(Box<Expr>),
    /// `user is some(u)`
    Is {
        expr: Box<Expr>,
        pattern: Box<Pattern>,
    },
    /// Produced after a syntax error so parsing can continue.
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrPiece {
    Text(String),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordField {
    pub name: Ident,
    /// `None` for the shorthand `{ posts }`, meaning `{ posts: posts }`.
    pub value: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Expr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Coalesce,
    Range,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Wildcard,
    /// A lowercase name binds the matched value.
    Binding(String),
    Literal(Box<Expr>),
    None,
    /// `Admin`, `Role.Admin`, `some(x)`, `Viewer(since)`
    Variant {
        path: Vec<Ident>,
        args: Vec<Pattern>,
    },
    Error,
}

// ----- Glazes (`.glaze` templates and components) -----

/// The source suffix declares whether a component can run in the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ComponentKind {
    #[default]
    Glaze,
    Ember,
}

/// A parsed `.glaze` or `.ember` file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Glaze {
    pub kind: ComponentKind,
    /// `{% props post: Post, user: User? = none %}`
    pub props: Option<Props>,
    /// `{% state likes = post.likes %}` (valid only in an Ember).
    pub states: Vec<LocalDecl>,
    /// `{% layout "base.glaze" title: post.title %}`
    pub layout: Option<Layout>,
    /// `{% css "site.css" %}` and `{% css module styles from "card.module.css" %}`
    pub css: Vec<CssImport>,
    pub nodes: Vec<Node>,
    /// Where each `{% tag %}` and `{# comment #}` is. A line that holds
    /// only these renders as nothing, not as a blank line.
    pub tags: Vec<Span>,
}

impl Glaze {
    /// Whether the source file explicitly declares an Ember.
    pub fn is_ember(&self) -> bool {
        self.kind == ComponentKind::Ember
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Props {
    pub params: Vec<Param>,
    pub span: Span,
}

/// `{% state name = value %}` or `{% let name = value %}`.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalDecl {
    pub name: Ident,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

/// `{% css "site.css" %}` uses a stylesheet; `{% css module styles from
/// "card.module.css" %}` uses a CSS Module, whose class names are fields
/// of `styles`.
#[derive(Clone, Debug, PartialEq)]
pub struct CssImport {
    pub module: Option<Ident>,
    /// The file, a string literal relative to the glaze.
    pub path: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The layout glaze, a string literal such as `"base.glaze"`.
    pub path: Expr,
    pub args: Vec<Arg>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Text {
        text: String,
        span: Span,
    },
    /// `{value}`, escaped for where it appears, or `{raw value}`.
    Expr {
        expr: Expr,
        raw: bool,
        span: Span,
    },
    /// An HTML element, or a component when the name is capitalized.
    Element(Element),
    If {
        branches: Vec<Branch>,
        otherwise: Option<Vec<Node>>,
        span: Span,
    },
    For {
        binding: Ident,
        iter: Expr,
        filter: Option<Expr>,
        body: Vec<Node>,
        /// `{% empty %}`: shown when nothing was iterated.
        empty: Option<Vec<Node>>,
        span: Span,
    },
    Match {
        expr: Expr,
        cases: Vec<Case>,
        span: Span,
    },
    Let(LocalDecl),
    /// `{% slot %}` in a layout or component: where the caller's children go.
    Slot {
        name: Option<Ident>,
        span: Span,
    },
    /// `{% fill name %} ... {% end %}` among a component's children, or at
    /// the top level of a page with a layout: the content for the slot
    /// `{% slot name %}`.
    Fill {
        name: Ident,
        body: Vec<Node>,
        span: Span,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    pub cond: Expr,
    pub body: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Vec<Node>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub name: Ident,
    pub attrs: Vec<Attr>,
    /// For `<script>` and `<style>`, a single text node with the raw contents.
    pub children: Vec<Node>,
    pub span: Span,
}

impl Element {
    /// Capitalized tags such as `<LikeButton>` are components.
    pub fn is_component(&self) -> bool {
        self.name.name.starts_with(|c: char| c.is_ascii_uppercase())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub name: Ident,
    pub value: AttrValue,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    /// `disabled`, `client:visible`
    Flag,
    /// `href="/posts/{post.id}"`
    Text(Vec<AttrPart>),
    /// `disabled={busy}`
    Expr(Expr),
    /// `on:click={ busy = true }`: statements run in the browser.
    Handler(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrPart {
    Text(String),
    Expr(Expr),
}
