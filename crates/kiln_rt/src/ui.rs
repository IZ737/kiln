//! `ui.merge`: styles are strings of atomic classes (see the checker's
//! `style` module), and a later style's class replaces an earlier one that
//! sets the same thing.

use crate::value::{bytes, kv, ptr, string, KStr};

/// What a class sets: the six characters after its `k`, for the classes
/// styles make.
fn slot(class: &[u8]) -> Option<&[u8]> {
    (class.len() == 13 && class[0] == b'k').then(|| &class[1..7])
}

/// The classes of `a` that `b` does not replace, then those of `b`.
pub fn merge(a: &[u8], b: &[u8]) -> Vec<u8> {
    let later: Vec<&[u8]> = b.split(|&c| c == b' ').filter_map(slot).collect();
    let mut out: Vec<u8> = Vec::with_capacity(a.len() + b.len() + 1);
    let classes = a
        .split(|&c| c == b' ')
        .filter(|c| !c.is_empty() && slot(c).is_none_or(|s| !later.contains(&s)))
        .chain(b.split(|&c| c == b' ').filter(|c| !c.is_empty()));
    for class in classes {
        if !out.is_empty() {
            out.push(b' ');
        }
        out.extend_from_slice(class);
    }
    out
}

/// # Safety
///
/// `a` and `b` are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_ui_merge(a: *mut KStr, b: *mut KStr) -> *mut KStr {
    let (x, y) = (bytes(kv(a)), bytes(kv(b)));
    if y.is_empty() {
        return a;
    }
    if x.is_empty() {
        return b;
    }
    ptr(string(&merge(x, y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_classes_win() {
        let a = b"kaaaaaa111111 kbbbbbb222222 card";
        let b = b"kaaaaaa333333";
        assert_eq!(merge(a, b), b"kbbbbbb222222 card kaaaaaa333333".to_vec());
    }
}
