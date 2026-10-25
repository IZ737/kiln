//! The `web` server's connections, on hyper. Each connection has a Kiln
//! task of its own (the accept loop in `kiln.c` starts it), and on it
//! `k_rt_web_serve` drives hyper's HTTP/1 connection with a `block_on` that
//! waits through the Kiln scheduler: when hyper cannot go on, the socket
//! says what it waits for and `k_task_wait` lets the other tasks run until
//! then. Hyper reads requests and writes responses; the C runtime routes
//! each request to its handler, which runs on this task's stack and may
//! wait too.
//!
//! The timeouts are the socket's, as the C server had them: a request's
//! head must arrive within the app's `timeout`, a kept-alive connection
//! waits `keep_alive` for the next one, and any other read or write that
//! waits longer than `timeout` ends the connection.

use std::cell::Cell;
use std::convert::Infallible;
use std::ffi::{c_char, c_void};
use std::future::Future;
use std::io;
use std::pin::{pin, Pin};
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::{Body, Incoming};
use hyper::ext::ReasonPhrase;
use hyper::header::{HeaderName, HeaderValue, CONNECTION, CONTENT_LENGTH};
use hyper::http::request::Parts;
use hyper::{Method, Request, Response, StatusCode, Version};

use crate::value::{bytes, k_alloc, k_map_set, kv, ptr, string, KMap, Kv};
use crate::web::reason;

/// A socket, as `kiln.c`'s `ksock`.
#[cfg(windows)]
pub type Sock = usize;
#[cfg(not(windows))]
pub type Sock = std::ffi::c_int;

/// The slots of a `Request` and a `Response`, in the field order of
/// `std/web.kiln`, as `kiln.c` numbers them.
const REQ_METHOD: usize = 0;
const REQ_PATH: usize = 1;
const REQ_QUERY: usize = 2;
const REQ_HEADERS: usize = 3;
const REQ_BODY: usize = 4;
const REQ_FIELDS: usize = 5;
const RES_STATUS: usize = 0;
const RES_HEADERS: usize = 1;
const RES_BODY: usize = 2;

extern "C" {
    fn k_task_wait(fd: Sock, write: bool, deadline: i64) -> bool;
    fn k_task_yield();
    fn k_mono_now() -> i64;
    fn k_sock_recv(fd: Sock, buf: *mut u8, n: i64) -> i64;
    fn k_sock_send(fd: Sock, buf: *const u8, n: i64) -> i64;
    fn k_web_headers() -> *mut KMap;
    fn k_web_handle(app: *mut c_void, req: *mut Kv, status: i64) -> *mut Kv;
    fn k_web_access_log(app: *mut c_void, req: *mut Kv, status: i64, start: i64);
    fn k_web_log_line(line: *const c_char, n: i64);
}

/// Serves the connection `fd` until it ends, for the app `app`, whose
/// settings come with it. The caller closes the socket.
///
/// # Safety
///
/// `app` is a `kapp` that the listening task keeps alive, and this runs on
/// the connection's own task.
#[no_mangle]
pub unsafe extern "C" fn k_rt_web_serve(
    app: *mut c_void,
    fd: Sock,
    timeout: i64,
    keep_alive: i64,
    max_header: i64,
    max_body: i64,
) {
    let conn = Rc::new(Conn {
        fd,
        timeout,
        keep_alive,
        phase: Cell::new(Phase::Idle {
            since: now(),
            first: true,
        }),
        head_start: Cell::new(now()),
        read_deadline: Cell::new(None),
        write_deadline: Cell::new(None),
        wait: Cell::new(None),
        timed_out: Cell::new(false),
    });
    let max_header = max_header.clamp(1, u32::MAX as i64) as usize;
    let mut http = hyper::server::conn::http1::Builder::new();
    http.keep_alive(keep_alive > 0)
        // The socket keeps the time instead, so hyper needs no timer.
        .header_read_timeout(None)
        .max_header_size(max_header)
        .max_buf_size(max_header.max(8192))
        .half_close(false);
    let service = {
        let conn = conn.clone();
        hyper::service::service_fn(move |req| serve(conn.clone(), app, max_body, req))
    };
    let served = block_on(&conn, http.serve_connection(Socket(conn.clone()), service));
    if let Err(e) = served {
        // Hyper answered a request it could not read itself (400, 414 or
        // 431, without a body), except for one that did not arrive in
        // time, which gets 408 here.
        if conn.timed_out.get() {
            let response = timed_out_response();
            send_all(&conn, &response);
            unsafe { k_web_access_log(app, std::ptr::null_mut(), 408, conn.head_start.get()) };
        } else if let Some(status) = refused(&e) {
            unsafe { k_web_access_log(app, std::ptr::null_mut(), status, conn.head_start.get()) };
        }
    }
}

/// What a connection is doing, which decides how long a read may wait.
#[derive(Clone, Copy)]
enum Phase {
    /// Waiting for the first byte of a request: from `since`, for the
    /// app's timeout if it is the connection's first, otherwise for its
    /// keep-alive time.
    Idle { since: i64, first: bool },
    /// Reading a request's head, which must arrive by `deadline`.
    Head { deadline: i64 },
    /// Answering a request: a read or write may wait for the app's timeout.
    Request,
}

/// A connection's state, which the socket and the service share.
struct Conn {
    fd: Sock,
    timeout: i64,
    keep_alive: i64,
    phase: Cell<Phase>,
    /// When the head of the latest request started to arrive.
    head_start: Cell<i64>,
    /// When the read or write that is waiting gives up.
    read_deadline: Cell<Option<i64>>,
    write_deadline: Cell<Option<i64>>,
    /// What the connection waits for when hyper is pending: a write or a
    /// read, until a deadline.
    wait: Cell<Option<(bool, i64)>>,
    /// Whether a read of a request's head gave up, so that a request that
    /// did not arrive in time gets 408.
    timed_out: Cell<bool>,
}

impl Conn {
    fn enter(&self, phase: Phase) {
        self.phase.set(phase);
        self.read_deadline.set(None);
    }

    /// When a read that waits now gives up.
    fn read_deadline(&self) -> i64 {
        match self.phase.get() {
            Phase::Idle { since, first } => {
                since.saturating_add(if first { self.timeout } else { self.keep_alive })
            }
            Phase::Head { deadline } => deadline,
            Phase::Request => self
                .read_deadline
                .get_or_insert_with(|| later(self.timeout)),
        }
    }

    /// Some bytes arrived.
    fn received(&self) {
        self.read_deadline.set(None);
        if let Phase::Idle { since, first } = self.phase.get() {
            self.head_start.set(now());
            let deadline = if first {
                since.saturating_add(self.timeout)
            } else {
                later(self.timeout)
            };
            self.phase.set(Phase::Head { deadline });
        }
    }
}

trait CellExt<T> {
    fn get_or_insert_with(&self, f: impl FnOnce() -> T) -> T;
}

impl<T: Copy> CellExt<T> for Cell<Option<T>> {
    fn get_or_insert_with(&self, f: impl FnOnce() -> T) -> T {
        match self.get() {
            Some(v) => v,
            None => {
                let v = f();
                self.set(Some(v));
                v
            }
        }
    }
}

fn now() -> i64 {
    unsafe { k_mono_now() }
}

/// The monotonic time `ns` from now.
fn later(ns: i64) -> i64 {
    now().saturating_add(ns)
}

/// Runs `fut` to the end on this task. The waker does nothing: a
/// connection can only go on when its socket is ready or a deadline has
/// passed, and `k_task_wait` waits for exactly that.
fn block_on<F: Future>(conn: &Conn, fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        conn.wait.set(None);
        if let Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
            return out;
        }
        match conn.wait.take() {
            // Whether it was ready or the deadline came, the next poll
            // finds out.
            Some((write, deadline)) => {
                unsafe { k_task_wait(conn.fd, write, deadline) };
            }
            // Pending for some other reason, such as hyper yielding: let
            // the other tasks run, then poll again.
            None => unsafe { k_task_yield() },
        }
    }
}

/// The socket, as hyper reads and writes it.
struct Socket(Rc<Conn>);

impl hyper::rt::Read for Socket {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        let conn = &self.0;
        let n = unsafe {
            let free = buf.as_mut();
            k_sock_recv(conn.fd, free.as_mut_ptr().cast(), free.len() as i64)
        };
        if n >= 0 {
            unsafe { buf.advance(n as usize) };
            if n > 0 {
                conn.received();
            }
            return Poll::Ready(Ok(()));
        }
        if n == -2 {
            return Poll::Ready(Err(io::ErrorKind::ConnectionReset.into()));
        }
        let deadline = conn.read_deadline();
        if now() >= deadline {
            if !matches!(conn.phase.get(), Phase::Request) {
                conn.timed_out.set(true);
            }
            return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
        }
        conn.wait.set(Some((false, deadline)));
        Poll::Pending
    }
}

impl hyper::rt::Write for Socket {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let conn = &self.0;
        let n = unsafe { k_sock_send(conn.fd, buf.as_ptr(), buf.len() as i64) };
        if n >= 0 {
            conn.write_deadline.set(None);
            return Poll::Ready(Ok(n as usize));
        }
        if n == -2 {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        let deadline = conn
            .write_deadline
            .get_or_insert_with(|| later(conn.timeout));
        if now() >= deadline {
            return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
        }
        conn.wait.set(Some((true, deadline)));
        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    /// `kiln.c` closes the connection, after hyper is done with it.
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Sends `bytes` outside hyper, giving up if the client stops reading.
fn send_all(conn: &Conn, mut bytes: &[u8]) {
    let deadline = later(conn.timeout);
    while !bytes.is_empty() {
        let n = unsafe { k_sock_send(conn.fd, bytes.as_ptr(), bytes.len() as i64) };
        if n > 0 {
            bytes = &bytes[n as usize..];
        } else if n == -2 || !unsafe { k_task_wait(conn.fd, true, deadline) } {
            return;
        }
    }
}

/// The status of the response hyper sent for a request it could not
/// read, if it sent one.
fn refused(e: &hyper::Error) -> Option<i64> {
    if !e.is_parse() || e.is_parse_version_h2() {
        None
    } else if e.is_parse_too_large() {
        Some(if e.to_string().contains("URI") {
            414
        } else {
            431
        })
    } else {
        Some(400)
    }
}

/// The answer to a request whose head did not arrive in time.
fn timed_out_response() -> Vec<u8> {
    let date = httpdate::fmt_http_date(std::time::SystemTime::now());
    format!(
        "HTTP/1.1 408 Request Timeout\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: 15\r\ndate: {date}\r\nconnection: close\r\n\r\nRequest Timeout"
    )
    .into_bytes()
}

/// Answers one request. Only this function awaits, and it holds no Kiln
/// values: hyper keeps this future in memory the collector does not scan.
/// `answer` makes and uses them without waiting in between (a handler may
/// still switch tasks, but its values are on this task's stack).
async fn serve(
    conn: Rc<Conn>,
    app: *mut c_void,
    max_body: i64,
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    conn.enter(Phase::Request);
    let start = conn.head_start.get();
    let (head, body) = req.into_parts();
    let max_body = max_body.max(0) as u64;
    let (body, status) = match body.size_hint().exact() {
        Some(n) if n > max_body => (Bytes::new(), 413),
        _ => match Limited::new(body, max_body as usize).collect().await {
            Ok(body) => (body.to_bytes(), 0),
            Err(e) if e.is::<LengthLimitError>() => (Bytes::new(), 413),
            Err(_) => (Bytes::new(), 400),
        },
    };
    let res = answer(app, &head, &body, status, conn.keep_alive > 0, start);
    conn.enter(Phase::Idle {
        since: now(),
        first: false,
    });
    Ok(res)
}

/// The response to a request, from its handler, or the error response
/// for `refused` if it is not 0. `keep_alive` is whether the app keeps
/// connections open.
fn answer(
    app: *mut c_void,
    head: &Parts,
    body: &[u8],
    refused: i64,
    keep_alive: bool,
    start: i64,
) -> Response<Full<Bytes>> {
    let req = request(head, body);
    let res = unsafe { k_web_handle(app, req, refused) };
    let res = unsafe { std::slice::from_raw_parts(res, 3) };
    let status = res[RES_STATUS] as i64;
    unsafe { k_web_access_log(app, req, status, start) };
    // HTTP/1.1 connections stay open unless the client asks to close
    // them, and HTTP/1.0 ones close unless it asks to keep them; one whose
    // request the server refused closes.
    let asked = |what: &str| {
        head.headers
            .get_all(CONNECTION)
            .iter()
            .any(|v| contains(v.as_bytes(), what))
    };
    let http10 = head.version == Version::HTTP_10;
    let keep = keep_alive && refused == 0 && !asked("close") && (!http10 || asked("keep-alive"));
    response(res, head.method == Method::HEAD, keep, http10)
}

/// Whether the header value `v` contains `what`, ignoring case.
fn contains(v: &[u8], what: &str) -> bool {
    v.windows(what.len())
        .any(|w| w.eq_ignore_ascii_case(what.as_bytes()))
}

/// The `Request` a handler gets: names of headers in lower case, and a
/// repeated header's values joined with ", ".
fn request(head: &Parts, body: &[u8]) -> *mut Kv {
    let req: *mut Kv = unsafe { k_alloc(std::mem::size_of::<Kv>() * REQ_FIELDS).cast() };
    let slot = |i: usize, v: Kv| unsafe { *req.add(i) = v };
    slot(REQ_METHOD, string(head.method.as_str().as_bytes()));
    slot(REQ_PATH, string(head.uri.path().as_bytes()));
    slot(REQ_QUERY, string(head.uri.query().unwrap_or("").as_bytes()));
    let headers = unsafe { k_web_headers() };
    slot(REQ_HEADERS, kv(headers));
    for name in head.headers.keys() {
        let mut value = Vec::new();
        for (i, v) in head.headers.get_all(name).iter().enumerate() {
            if i > 0 {
                value.extend_from_slice(b", ");
            }
            value.extend_from_slice(v.as_bytes());
        }
        let name = string(name.as_str().as_bytes());
        unsafe { k_map_set(headers, name, string(&value)) };
    }
    slot(REQ_BODY, string(body));
    req
}

/// The response hyper sends for a handler's `Response`. Header names and
/// values must be ones HTTP allows: line breaks are left out of them, and a
/// header that is still not allowed is left out, with an error logged.
fn response(res: &[Kv], head: bool, mut keep: bool, http10: bool) -> Response<Full<Bytes>> {
    let status = res[RES_STATUS] as u16;
    let body = bytes(res[RES_BODY]);
    let map = unsafe { &*ptr::<KMap>(res[RES_HEADERS]) };
    let no_body = status == 204 || status == 304;
    // Hyper sends no body for HEAD, and checks the body against a
    // `content-length` the handler set, which may be the length of the body
    // a GET would get.
    let mut out = Response::new(Full::new(if no_body || head {
        Bytes::new()
    } else {
        Bytes::copy_from_slice(body)
    }));
    *out.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    out.extensions_mut()
        .insert(ReasonPhrase::from_static(reason(status as i64).to_bytes()));
    let headers = out.headers_mut();
    let mut has_length = false;
    for i in 0..map.used as usize {
        if !unsafe { *map.live.add(i) } {
            continue;
        }
        let name: Vec<u8> = clean(bytes(unsafe { *map.keys.add(i) }));
        let value: Vec<u8> = clean(bytes(unsafe { *map.vals.add(i) }));
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(&name),
            HeaderValue::from_bytes(&value),
        ) else {
            let line = format!(
                "error a handler set the header {:?}, which HTTP does not allow; it was left out",
                String::from_utf8_lossy(&name)
            );
            unsafe { k_web_log_line(line.as_ptr().cast(), line.len() as i64) };
            continue;
        };
        if name == CONNECTION {
            if contains(value.as_bytes(), "close") {
                keep = false;
            }
            continue;
        }
        if name == CONTENT_LENGTH {
            // A response to HEAD may say how long the body would be;
            // otherwise the length is the body's.
            has_length = true;
            if !head {
                headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len()));
                continue;
            }
        }
        headers.append(name, value);
    }
    if !has_length && !no_body {
        headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len()));
    }
    if !keep {
        headers.insert(CONNECTION, HeaderValue::from_static("close"));
    } else if http10 {
        headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
    }
    out
}

/// `s` without line breaks, so that a value cannot start a header of its
/// own.
fn clean(s: &[u8]) -> Vec<u8> {
    s.iter()
        .copied()
        .filter(|&c| c != b'\r' && c != b'\n')
        .collect()
}
