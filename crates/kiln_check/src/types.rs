//! Types and the definitions the checker builds from declarations.

use std::collections::HashMap;
use std::rc::Rc;

pub type AdtId = usize;
pub type ModId = usize;

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    /// A struct or enum, including built-ins such as `Int` and `List<T>`.
    Adt(AdtId, Vec<Ty>),
    /// `T?`
    Opt(Box<Ty>),
    Fn(Rc<FnTy>),
    /// An anonymous record such as `{ posts, title: "Home" }`.
    Record(Vec<(String, Ty)>),
    /// A generic type parameter inside the declaration that introduces it.
    Param(String),
    /// An inference variable, solved by unification.
    Var(u32),
    /// The type of statements and of functions without a return type.
    Unit,
    /// The type of `return`, `throw` and other expressions that never finish.
    Never,
    /// Accepts any value. Only used in standard library signatures.
    Any,
    /// Any number of extra arguments. Only used in standard library signatures.
    Rest,
    /// Produced after an error, and compatible with everything so one
    /// mistake is reported once.
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnTy {
    pub params: Vec<Ty>,
    pub ret: Ty,
    /// The error type, when the function can fail.
    pub throws: Option<Ty>,
}

#[derive(Clone, Debug)]
pub struct AdtDef {
    pub name: String,
    pub params: Vec<String>,
    pub kind: AdtKind,
    pub methods: Vec<FnSig>,
}

#[derive(Clone, Debug)]
pub enum AdtKind {
    Struct(Vec<FieldDef>),
    Enum(Vec<VariantDef>),
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub has_default: bool,
}

#[derive(Clone, Debug)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<(Option<String>, Ty)>,
}

#[derive(Clone, Debug)]
pub struct FnSig {
    pub name: String,
    /// The function's own type parameters.
    pub generics: Vec<String>,
    /// For methods, the type parameters of the `impl` target, matched to the
    /// receiver's type arguments by position.
    pub impl_params: Vec<String>,
    pub has_self: bool,
    pub params: Vec<ParamSig>,
    pub ret: Ty,
    pub throws: Option<Ty>,
}

#[derive(Clone, Debug)]
pub struct ParamSig {
    pub name: Option<String>,
    pub ty: Ty,
    pub has_default: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ModuleDef {
    pub name: String,
    pub fns: HashMap<String, FnSig>,
    pub types: HashMap<String, AdtId>,
}

/// Replaces type parameters by name.
pub fn substitute(ty: &Ty, map: &HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::Param(name) => map.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Adt(id, args) => Ty::Adt(*id, args.iter().map(|a| substitute(a, map)).collect()),
        Ty::Opt(inner) => Ty::Opt(Box::new(substitute(inner, map))),
        Ty::Fn(f) => Ty::Fn(Rc::new(FnTy {
            params: f.params.iter().map(|p| substitute(p, map)).collect(),
            ret: substitute(&f.ret, map),
            throws: f.throws.as_ref().map(|t| substitute(t, map)),
        })),
        Ty::Record(fields) => Ty::Record(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), substitute(t, map)))
                .collect(),
        ),
        _ => ty.clone(),
    }
}

/// The closest candidate to a misspelled name, if any is close enough.
pub fn similar<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let limit = (name.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .filter(|c| *c != name)
        .map(|c| {
            let d = if c.eq_ignore_ascii_case(name) {
                0
            } else {
                edit_distance(name, c)
            };
            (d, c)
        })
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, c)| (*d, c.to_string()))
        .map(|(_, c)| c.to_string())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            row.push((prev[j] + cost).min(prev[j + 1] + 1).min(row[j] + 1));
        }
        prev = row;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_close_names() {
        assert_eq!(similar("tittle", ["title", "body"]), Some("title".into()));
        assert_eq!(similar("string", ["String", "Int"]), Some("String".into()));
        assert_eq!(similar("xyz", ["title", "body"]), None);
    }
}
