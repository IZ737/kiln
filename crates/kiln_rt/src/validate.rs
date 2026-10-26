//! `validate`: rules that say what is wrong with submitted input.
//!
//! A rule is an ordinary function that returns a `Problem?`: the problem
//! it found, or `none` when the value is fine. `validate.check` gathers
//! them, so a program's own functions are rules too, and a Glaze shows
//! `checked.error("title")` next to the field it belongs to.
//!
//! Decoding a form and checking it are different failures: a form the
//! server cannot read at all is the client's mistake (400), while input
//! that reads but breaks a rule is the person's, and the handler shows the
//! form again with these problems in it (422).

use crate::value::*;

/// The slots of a `Problem` and a `Checked`, in the field order of
/// `std/validate.kiln`.
const PROBLEM_FIELD: usize = 0;
const PROBLEM_MESSAGE: usize = 1;
const CHECKED_VALID: usize = 0;
const CHECKED_PROBLEMS: usize = 1;

/// A field's name as it reads in a message: `author_name` is "author name".
fn label(field: &str) -> String {
    field.replace('_', " ")
}

/// A `Problem`.
unsafe fn problem(field: Kv, message: String) -> Kv {
    let p = slots(2);
    *p.add(PROBLEM_FIELD) = field;
    *p.add(PROBLEM_MESSAGE) = string(message.as_bytes());
    kv(p)
}

/// A `Problem?`: the problem, or `none`.
unsafe fn found(field: Kv, message: Option<String>) -> Kv {
    match message {
        Some(m) => kv(k_box(problem(field, m))),
        None => 0,
    }
}

unsafe fn text(v: Kv) -> String {
    String::from_utf8_lossy(bytes(v)).into_owned()
}

/// How many characters a value has, as `String.len` counts them.
fn length_of(value: &str) -> i64 {
    value.chars().count() as i64
}

/// `validate.problem(field, message)`: a problem of a program's own.
///
/// # Safety
///
/// The arguments are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_problem(field: Kv, message: Kv) -> Kv {
    problem(field, text(message))
}

/// `validate.when(wrong, field, message)`: the problem when `wrong`.
///
/// # Safety
///
/// `field` and `message` are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_when(wrong: Kv, field: Kv, message: Kv) -> Kv {
    found(field, (wrong != 0).then(|| text(message)))
}

/// `validate.required(field, value)`: a value that is not only spaces.
///
/// # Safety
///
/// The arguments are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_required(field: Kv, value: Kv) -> Kv {
    let empty = text(value).trim().is_empty();
    found(
        field,
        empty.then(|| format!("{} is required", label(&text(field)))),
    )
}

/// `validate.length(field, value, min, max)`: a length between `min` and
/// `max` characters. A `max` of 0 is no upper bound.
///
/// # Safety
///
/// `field` and `value` are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_length(field: Kv, value: Kv, min: Kv, max: Kv) -> Kv {
    let (min, max) = (min as i64, max as i64);
    let n = length_of(&text(value));
    let name = label(&text(field));
    let message = if n < min {
        Some(format!("{name} needs at least {min} characters"))
    } else if max > 0 && n > max {
        Some(format!("{name} can have at most {max} characters"))
    } else {
        None
    };
    found(field, message)
}

/// `validate.range(field, value, min, max)`: a number between `min` and
/// `max`.
///
/// # Safety
///
/// `field` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_range(field: Kv, value: Kv, min: Kv, max: Kv) -> Kv {
    let (value, min, max) = (value as i64, min as i64, max as i64);
    let name = label(&text(field));
    let message = if value < min {
        Some(format!("{name} cannot be less than {min}"))
    } else if value > max {
        Some(format!("{name} cannot be more than {max}"))
    } else {
        None
    };
    found(field, message)
}

/// Whether `value` looks like one address: a local part, one `@`, and a
/// domain with a dot in it. Being sure needs sending mail to it.
fn is_email(value: &str) -> bool {
    let value = value.trim();
    if value.len() > 254 || value.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.len() < 3 || domain.contains('@') {
        return false;
    }
    match domain.split_once('.') {
        Some((host, rest)) => !host.is_empty() && !rest.is_empty() && !rest.ends_with('.'),
        None => false,
    }
}

/// `validate.email(field, value)`: an email address.
///
/// # Safety
///
/// The arguments are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_email(field: Kv, value: Kv) -> Kv {
    let value = text(value);
    let message = (!value.trim().is_empty() && !is_email(&value))
        .then(|| format!("{} is not an email address", label(&text(field))));
    found(field, message)
}

/// `validate.one_of(field, value, choices)`: one of a list of values.
///
/// # Safety
///
/// `field` and `value` are Kiln strings, `choices` a `List<String>`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_one_of(field: Kv, value: Kv, choices: Kv) -> Kv {
    let value = text(value);
    let list = &*ptr::<KList>(choices);
    let mut names = Vec::new();
    for i in 0..list.len {
        names.push(text(*list.items.add(i as usize)));
    }
    let message = (!names.contains(&value)).then(|| {
        format!(
            "{} should be {}",
            label(&text(field)),
            match names.split_last() {
                Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
                _ => names.join(""),
            }
        )
    });
    found(field, message)
}

/// `validate.same(field, value, other)`: two values that must match, such
/// as a password and its confirmation.
///
/// # Safety
///
/// The arguments are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_same(field: Kv, value: Kv, other: Kv) -> Kv {
    let message =
        (text(value) != text(other)).then(|| format!("{} does not match", label(&text(field))));
    found(field, message)
}

/// `validate.check(problems)`: the problems that are there, in order.
///
/// # Safety
///
/// `problems` is a `List<Problem?>`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_check(problems: Kv) -> Kv {
    let list = &*ptr::<KList>(problems);
    let out = k_list(0, std::ptr::null());
    for i in 0..list.len {
        let item = *list.items.add(i as usize);
        if item != 0 {
            k_list_push(out, (*ptr::<KBox>(item)).v);
        }
    }
    let checked = slots(2);
    *checked.add(CHECKED_VALID) = ((*out).len == 0) as Kv;
    *checked.add(CHECKED_PROBLEMS) = kv(out);
    kv(checked)
}

/// The problems of a `Checked`.
unsafe fn problems(checked: Kv) -> &'static KList {
    &*ptr::<KList>(*ptr::<Kv>(checked).add(CHECKED_PROBLEMS))
}

/// `checked.error(field)`: the first message for a field, or `none`.
///
/// # Safety
///
/// `checked` is a `Checked` and `field` a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_error(checked: Kv, field: Kv) -> Kv {
    let want = text(field);
    let list = problems(checked);
    for i in 0..list.len {
        let p = ptr::<Kv>(*list.items.add(i as usize));
        if text(*p.add(PROBLEM_FIELD)) == want {
            return kv(k_box(*p.add(PROBLEM_MESSAGE)));
        }
    }
    0
}

/// `checked.messages()`: one message per field with a problem, for a form
/// to show above itself.
///
/// # Safety
///
/// `checked` is a `Checked`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_validate_messages(checked: Kv) -> Kv {
    let list = problems(checked);
    let out = k_list(0, std::ptr::null());
    for i in 0..list.len {
        let p = ptr::<Kv>(*list.items.add(i as usize));
        k_list_push(out, *p.add(PROBLEM_MESSAGE));
    }
    kv(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_name_reads_as_words() {
        assert_eq!(label("author_name"), "author name");
        assert_eq!(label("title"), "title");
    }

    #[test]
    fn an_address_needs_a_local_part_and_a_domain_with_a_dot() {
        for good in ["a@b.co", "first.last@mail.example.com", "x+y@z.dev"] {
            assert!(is_email(good), "{good}");
        }
        for bad in ["", "a", "a@b", "a@.com", "a@b.", "a b@c.de", "a@b@c.de"] {
            assert!(!is_email(bad), "{bad}");
        }
    }
}
