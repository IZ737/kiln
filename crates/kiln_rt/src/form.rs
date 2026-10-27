//! `req.form<T>()`: a submitted form read as a struct.
//!
//! The browser sends a form as `application/x-www-form-urlencoded`, or as
//! `multipart/form-data` when it has a file in it. Both arrive here as the
//! request's body, and the compiler says which struct to make of it, so
//! this reads the fields by name and type.
//!
//! What a form carries is text, which the fields' types turn into values:
//! a `String` as it was sent, an `Int` or a `Float` as a number, a `Bool`
//! as a checkbox (a box the person did not tick is not sent at all, so an
//! absent `Bool` is `false`), an optional as `none` when it is absent or
//! empty, a `List` as every value with that name, and an enum as the name
//! of one of its variants. A field of any other type, a number that is not
//! one, or a missing field is the client's mistake: the handler gets an
//! `Error`, and a handler that passes it on answers 400.

use crate::value::*;

extern "C" {
    /// Stores the message of a failing library call, and marks it as the
    /// client's mistake, so that the server answers 400.
    fn k_rt_client_fail(message: *mut KStr);
    fn k_rt_header(request: *mut Kv, name: *mut KStr) -> *mut KStr;
}

/// How much of a form the runtime reads: enough for ordinary forms, and
/// not so much that a request can spend the server's memory. The server's
/// `max_body` has already bounded the body itself.
const MAX_FIELDS: usize = 1000;
const MAX_NAME: usize = 256;
const MAX_VALUE: usize = 1 << 20;

fn fail(message: &str) {
    unsafe { k_rt_client_fail(ptr(string(message.as_bytes()))) }
}

/// One submitted field.
#[cfg_attr(test, derive(Debug))]
struct Field {
    name: String,
    value: Vec<u8>,
    /// The file name a multipart part carried, if it was a file.
    filename: Option<String>,
}

/// `req.form<T>()`.
///
/// # Safety
///
/// `d` describes the struct to make, and `request` is a `Request`'s slots.
#[no_mangle]
pub unsafe extern "C" fn k_rt_web_form(d: *const KDesc, request: Kv) -> Kv {
    let req = ptr::<Kv>(request);
    let kind = header(req, "content-type").unwrap_or_default();
    let body = bytes(*req.add(4));
    let fields = match form(&kind, body) {
        Ok(fields) => fields,
        Err(e) => {
            fail(&e);
            return 0;
        }
    };
    match build(&*d, &fields) {
        Ok(v) => v,
        Err(e) => {
            fail(&e);
            0
        }
    }
}

/// A request header, in lower case as the server stores it.
unsafe fn header(req: *mut Kv, name: &str) -> Option<String> {
    let found = k_rt_header(req, ptr(string(name.as_bytes())));
    if found.is_null() {
        return None;
    }
    Some(String::from_utf8_lossy((*found).bytes()).into_owned())
}

/// The fields of a body, by its content type.
fn form(kind: &str, body: &[u8]) -> Result<Vec<Field>, String> {
    let media = kind.split(';').next().unwrap_or("").trim().to_lowercase();
    match media.as_str() {
        "application/x-www-form-urlencoded" => urlencoded(body),
        "multipart/form-data" => match boundary(kind) {
            Some(b) => multipart(body, b.as_bytes()),
            None => Err("the form has no boundary in its content type".into()),
        },
        "" => Err("the form has no content type".into()),
        other => Err(format!(
            "cannot read a form sent as {other}: send it as \
             application/x-www-form-urlencoded or multipart/form-data"
        )),
    }
}

fn urlencoded(body: &[u8]) -> Result<Vec<Field>, String> {
    let mut fields = Vec::new();
    for (name, value) in form_urlencoded::parse(body) {
        if fields.len() == MAX_FIELDS {
            return Err(format!("the form has more than {MAX_FIELDS} fields"));
        }
        if name.len() > MAX_NAME {
            return Err("the form has a field name that is too long".into());
        }
        if value.len() > MAX_VALUE {
            return Err(format!("the form field `{name}` is too long"));
        }
        fields.push(Field {
            name: name.into_owned(),
            value: value.into_owned().into_bytes(),
            filename: None,
        });
    }
    Ok(fields)
}

/// The `boundary=` of a `multipart/form-data` content type.
fn boundary(kind: &str) -> Option<String> {
    for part in kind.split(';').skip(1) {
        let (name, value) = part.split_once('=')?;
        if name.trim().eq_ignore_ascii_case("boundary") {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            if value.is_empty() || value.len() > 70 {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

/// Where `needle` is in `hay`, from `from`.
fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// The parts of a `multipart/form-data` body (RFC 7578): each starts after
/// `--<boundary>` and its own headers, and the last is followed by `--`.
fn multipart(body: &[u8], boundary: &[u8]) -> Result<Vec<Field>, String> {
    let mut mark = Vec::with_capacity(boundary.len() + 4);
    mark.extend_from_slice(b"--");
    mark.extend_from_slice(boundary);
    let bad = || "the form is not a valid multipart body".to_string();
    let mut at = find(body, &mark, 0).ok_or_else(bad)?;
    let mut fields = Vec::new();
    loop {
        at += mark.len();
        if body[at..].starts_with(b"--") {
            return Ok(fields);
        }
        // The CRLF that ends the boundary line, and then the part's
        // headers up to an empty line.
        let rest = body[at..]
            .strip_prefix(b"\r\n")
            .or_else(|| body[at..].strip_prefix(b"\n"))
            .ok_or_else(bad)?;
        let head = at + (body.len() - at - rest.len());
        // A line ending is `\r\n`, as the standard says, but a client that
        // sends bare newlines is read as it meant to be.
        let (blank, skip) = match (find(body, b"\r\n\r\n", head), find(body, b"\n\n", head)) {
            (Some(crlf), Some(lf)) if lf < crlf => (lf, 2),
            (Some(crlf), _) => (crlf, 4),
            (None, Some(lf)) => (lf, 2),
            (None, None) => return Err(bad()),
        };
        let headers = String::from_utf8_lossy(&body[head..blank]).into_owned();
        let start = blank + skip;
        let end = find(body, &mark, start).ok_or_else(bad)?;
        // The line ending before the next boundary belongs to the boundary.
        let mut value = &body[start..end];
        for ending in [b"\r\n".as_slice(), b"\n".as_slice()] {
            if let Some(v) = value.strip_suffix(ending) {
                value = v;
                break;
            }
        }
        let (name, filename) = disposition(&headers).ok_or_else(|| {
            "a part of the form has no name in its content-disposition".to_string()
        })?;
        if fields.len() == MAX_FIELDS {
            return Err(format!("the form has more than {MAX_FIELDS} fields"));
        }
        if name.len() > MAX_NAME {
            return Err("the form has a field name that is too long".into());
        }
        if value.len() > MAX_VALUE {
            return Err(format!("the form field `{name}` is too long"));
        }
        fields.push(Field {
            name,
            value: value.to_vec(),
            filename,
        });
        at = end;
    }
}

/// The `name` and `filename` of a part's `content-disposition`.
fn disposition(headers: &str) -> Option<(String, Option<String>)> {
    let line = headers
        .lines()
        .find(|l| l.to_lowercase().starts_with("content-disposition:"))?;
    let mut name = None;
    let mut filename = None;
    for part in line.split(';').skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim().to_lowercase().as_str() {
            "name" => name = Some(value),
            "filename" => filename = Some(value),
            _ => {}
        }
    }
    Some((name?, filename))
}

/// The struct a form's fields make.
unsafe fn build(d: &KDesc, fields: &[Field]) -> Result<Kv, String> {
    if d.kind != D_STRUCT && d.kind != D_RECORD {
        return Err("a form can only be read as a struct".into());
    }
    let out = slots(d.n);
    for i in 0..d.n {
        let name = d.label(i).unwrap_or("");
        let part = d.part(i);
        let found: Vec<&Field> = fields.iter().filter(|f| f.name == name).collect();
        let v = field(name, part, &found)?;
        unsafe { *out.add(i as usize) = v };
    }
    Ok(kv(out))
}

/// One field's value.
fn field(name: &str, d: &KDesc, found: &[&Field]) -> Result<Kv, String> {
    match d.kind {
        D_LIST => {
            let item = d.part(0);
            let list = unsafe { k_list(0, std::ptr::null()) };
            for f in found {
                let v = scalar(name, item, f)?;
                unsafe { k_list_push(list, v) };
            }
            Ok(kv(list))
        }
        D_BOOL => match found.last() {
            None => Ok(0),
            Some(f) => scalar(name, d, f),
        },
        D_OPT => match found.last() {
            None => Ok(0),
            Some(f) if f.value.is_empty() && f.filename.as_deref().unwrap_or("").is_empty() => {
                Ok(0)
            }
            Some(f) => {
                let v = scalar(name, d.part(0), f)?;
                Ok(kv(unsafe { k_box(v) }))
            }
        },
        _ => match found.last() {
            Some(f) => scalar(name, d, f),
            None => Err(format!("the form has no field `{name}`")),
        },
    }
}

/// One text value as the type it stands for.
fn scalar(name: &str, d: &KDesc, f: &Field) -> Result<Kv, String> {
    let text = String::from_utf8_lossy(&f.value);
    match d.kind {
        D_STR if std::str::from_utf8(&f.value).is_err() => Err(format!(
            "the form field `{name}` is not text; read a file as `Bytes`"
        )),
        D_STR => Ok(string(&f.value)),
        D_BYTES => Ok(string(&f.value)),
        D_INT => text
            .trim()
            .parse::<i64>()
            .map(|n| n as Kv)
            .map_err(|_| format!("the form field `{name}` is not a whole number")),
        D_FLOAT => text
            .trim()
            .parse::<f64>()
            .map(|n| n.to_bits())
            .map_err(|_| format!("the form field `{name}` is not a number")),
        D_BOOL => match text.trim().to_lowercase().as_str() {
            "on" | "true" | "1" | "yes" => Ok(1),
            "off" | "false" | "0" | "no" | "" => Ok(0),
            _ => Err(format!("the form field `{name}` is not true or false")),
        },
        D_ENUM => {
            let want = text.trim();
            match (0..d.n).find(|&i| d.part(i).name() == want && d.part(i).n == 0) {
                Some(i) => Ok(kv(unsafe { k_enum(i, 0, std::ptr::null()) })),
                None => {
                    let names: Vec<&str> = (0..d.n)
                        .filter(|&i| d.part(i).n == 0)
                        .map(|i| d.part(i).name())
                        .collect();
                    Err(format!(
                        "the form field `{name}` should be {}, not {want:?}",
                        names.join(" or ")
                    ))
                }
            }
        }
        _ => Err(format!(
            "a form cannot carry the type of the field `{name}`"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(fields: &[Field]) -> Vec<(String, String)> {
        fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    String::from_utf8_lossy(&f.value).into_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn a_urlencoded_form_keeps_every_value_of_a_name() {
        let fields = form(
            "application/x-www-form-urlencoded",
            b"title=Hello+there&tag=a&tag=b&body=%26%3D",
        )
        .unwrap();
        assert_eq!(
            names(&fields),
            [
                ("title".into(), "Hello there".into()),
                ("tag".into(), "a".into()),
                ("tag".into(), "b".into()),
                ("body".into(), "&=".into()),
            ]
        );
    }

    #[test]
    fn a_multipart_form_reads_its_parts_and_file_names() {
        let body = b"--xy\r\n\
            content-disposition: form-data; name=\"title\"\r\n\r\n\
            Hello\r\n\
            --xy\r\n\
            Content-Disposition: form-data; name=\"photo\"; filename=\"a.png\"\r\n\
            content-type: image/png\r\n\r\n\
            \x89PNG\r\n\
            --xy--\r\n";
        let fields = form("multipart/form-data; boundary=xy", body).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(names(&fields)[0], ("title".into(), "Hello".into()));
        assert_eq!(fields[1].name, "photo");
        assert_eq!(fields[1].value, b"\x89PNG");
        assert_eq!(fields[1].filename.as_deref(), Some("a.png"));
    }

    #[test]
    fn a_multipart_form_read_with_bare_newlines_is_the_same() {
        let body = b"--xy\ncontent-disposition: form-data; name=\"a\"\n\n1\n--xy--\n";
        let fields = form("multipart/form-data; boundary=xy", body).unwrap();
        assert_eq!(names(&fields), [("a".into(), "1".into())]);
    }

    #[test]
    fn a_body_that_is_not_a_form_says_so() {
        assert!(form("application/json", b"{}")
            .unwrap_err()
            .contains("cannot read a form sent as application/json"));
        assert!(form("", b"").unwrap_err().contains("no content type"));
        assert!(form("multipart/form-data", b"")
            .unwrap_err()
            .contains("no boundary"));
        assert!(form("multipart/form-data; boundary=xy", b"nope")
            .unwrap_err()
            .contains("not a valid multipart body"));
    }
}
