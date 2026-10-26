//! PostgreSQL and MySQL, spoken over a plain socket.
//!
//! The Kiln runtime runs a program's tasks on one thread, so a query that
//! waits for the database must let the other tasks run meanwhile. These
//! clients do their own reading and writing on a non-blocking socket, and
//! when the socket is not ready they call the [`Waiter`] they were given,
//! which in a program parks the task (`k_task_wait`). `kiln db`, which has
//! no tasks, gives none, and the socket blocks.
//!
//! Values are bound, never written into a statement: PostgreSQL takes them
//! in its extended protocol and MySQL in prepared statements. A statement
//! without values goes as a simple query, which may hold several
//! statements, as a migration does.
//!
//! TLS is not here yet: a URL that asks for it (`sslmode=require`,
//! `ssl-mode=REQUIRED`) is refused rather than sent in the clear.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

pub mod mysql;
pub mod postgres;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Blob(Vec<u8>),
}

/// The rows a statement gave, with their columns' names.
#[derive(Default, Debug, PartialEq)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// What a statement changed: the number of rows, and the key the database
/// made for an insert, when it says.
#[derive(Default, Debug, PartialEq)]
pub struct Done {
    pub changed: i64,
    pub key: Option<i64>,
}

/// The kinds of failure `sql.error_kind` tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Connection,
    Syntax,
    Unique,
    ForeignKey,
    NotNull,
    Check,
    Constraint,
    Decode,
    Busy,
    Transaction,
    Unsupported,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub kind: Kind,
    pub message: String,
}

impl Error {
    pub fn new(kind: Kind, message: impl Into<String>) -> Error {
        Error {
            kind,
            message: message.into(),
        }
    }

    fn io(e: io::Error) -> Error {
        Error::new(Kind::Connection, format!("the connection failed: {e}"))
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The socket as the system knows it: a file descriptor, or a Windows
/// `SOCKET`.
#[cfg(unix)]
pub type RawSock = std::os::fd::RawFd;
#[cfg(windows)]
pub type RawSock = std::os::windows::io::RawSocket;

/// Waits until the socket can be read (or, with `write`, written), for at
/// most `limit` when there is one. False when it gave up, which fails the
/// call.
pub type Waiter = fn(sock: RawSock, write: bool, limit: Option<Duration>) -> bool;

/// How long a connection may take to open.
const CONNECT_LIMIT: Duration = Duration::from_secs(10);

/// A connection's socket, read and written in full, waiting through the
/// [`Waiter`] when there is one.
pub struct Stream {
    tcp: TcpStream,
    wait: Option<Waiter>,
}

impl Stream {
    /// Connects to `host:port`, trying each address the name has.
    pub fn connect(host: &str, port: u16, wait: Option<Waiter>) -> Result<Stream, Error> {
        let addrs: Vec<SocketAddr> = (host, port)
            .to_socket_addrs()
            .map_err(|e| Error::new(Kind::Connection, format!("cannot find {host}: {e}")))?
            .collect();
        let mut last = None;
        for addr in addrs {
            match connect_one(addr, wait) {
                Ok(tcp) => {
                    let _ = tcp.set_nodelay(true);
                    return Ok(Stream { tcp, wait });
                }
                Err(e) => last = Some(e),
            }
        }
        Err(Error::new(
            Kind::Connection,
            match last {
                Some(e) => format!("cannot connect to {host}:{port}: {e}"),
                None => format!("cannot find {host}"),
            },
        ))
    }

    fn raw(&self) -> RawSock {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            self.tcp.as_raw_fd()
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawSocket;
            self.tcp.as_raw_socket()
        }
    }

    fn ready(&self, write: bool) -> io::Result<()> {
        match self.wait {
            // A query may run as long as it runs.
            Some(wait) if wait(self.raw(), write, None) => Ok(()),
            Some(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the database did not answer in time",
            )),
            None => Ok(()),
        }
    }

    pub fn read_some(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        loop {
            match self.tcp.read(buf) {
                Ok(0) => {
                    return Err(Error::new(
                        Kind::Connection,
                        "the database closed the connection",
                    ))
                }
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.ready(false).map_err(Error::io)?
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(Error::io(e)),
            }
        }
    }

    pub fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<(), Error> {
        while !buf.is_empty() {
            let n = self.read_some(buf)?;
            buf = &mut buf[n..];
        }
        Ok(())
    }

    pub fn write_all(&mut self, mut buf: &[u8]) -> Result<(), Error> {
        while !buf.is_empty() {
            match self.tcp.write(buf) {
                Ok(0) => {
                    return Err(Error::new(
                        Kind::Connection,
                        "the database closed the connection",
                    ))
                }
                Ok(n) => buf = &buf[n..],
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.ready(true).map_err(Error::io)?
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(Error::io(e)),
            }
        }
        Ok(())
    }
}

/// Connects to one address. With a waiter the connect does not block: it
/// starts, and the waiter waits until the socket can be written.
fn connect_one(addr: SocketAddr, wait: Option<Waiter>) -> io::Result<TcpStream> {
    let Some(wait) = wait else {
        return TcpStream::connect_timeout(&addr, CONNECT_LIMIT);
    };
    let socket = socket2::Socket::new(
        socket2::Domain::for_address(addr),
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_nonblocking(true)?;
    match socket.connect(&addr.into()) {
        Ok(()) => {}
        Err(e) if in_progress(&e) => {
            let tcp: TcpStream = socket.into();
            #[cfg(unix)]
            let raw = {
                use std::os::fd::AsRawFd;
                tcp.as_raw_fd()
            };
            #[cfg(windows)]
            let raw = {
                use std::os::windows::io::AsRawSocket;
                tcp.as_raw_socket()
            };
            if !wait(raw, true, Some(CONNECT_LIMIT)) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the database did not answer in time",
                ));
            }
            if let Some(e) = tcp.take_error()? {
                return Err(e);
            }
            tcp.peer_addr()?;
            return Ok(tcp);
        }
        Err(e) => return Err(e),
    }
    Ok(socket.into())
}

fn in_progress(e: &io::Error) -> bool {
    // EINPROGRESS on Linux and macOS; Windows says it would block.
    e.kind() == io::ErrorKind::WouldBlock || matches!(e.raw_os_error(), Some(115 | 36))
}

/// The parts of a database URL:
/// `scheme://user:password@host:port/database?key=value`.
#[derive(Debug, PartialEq)]
pub struct Target {
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub options: Vec<(String, String)>,
}

impl Target {
    pub fn parse(url: &str, default_port: u16) -> Result<Target, Error> {
        let bad = |why: &str| {
            Error::new(
                Kind::Connection,
                format!("cannot read the database URL: {why}"),
            )
        };
        let rest = url
            .split_once("://")
            .map(|(_, rest)| rest)
            .ok_or_else(|| bad("it has no `://`"))?;
        let (rest, query) = match rest.split_once('?') {
            Some((r, q)) => (r, q),
            None => (rest, ""),
        };
        let (authority, database) = match rest.split_once('/') {
            Some((a, d)) => (a, d),
            None => (rest, ""),
        };
        let (credentials, hostport) = match authority.rsplit_once('@') {
            Some((c, h)) => (c, h),
            None => ("", authority),
        };
        let (user, password) = match credentials.split_once(':') {
            Some((u, p)) => (u, p),
            None => (credentials, ""),
        };
        let (host, port) = if let Some(inner) = hostport.strip_prefix('[') {
            // An IPv6 address: `[::1]:5432`.
            let (h, after) = inner
                .split_once(']')
                .ok_or_else(|| bad("an IPv6 address has no `]`"))?;
            (h, after.strip_prefix(':').unwrap_or(""))
        } else {
            match hostport.rsplit_once(':') {
                Some((h, p)) => (h, p),
                None => (hostport, ""),
            }
        };
        let port = if port.is_empty() {
            default_port
        } else {
            port.parse().map_err(|_| bad("the port is not a number"))?
        };
        let decode = |s: &str| {
            percent_encoding::percent_decode_str(s)
                .decode_utf8_lossy()
                .into_owned()
        };
        let options = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| match p.split_once('=') {
                Some((k, v)) => (decode(k), decode(v)),
                None => (decode(p), String::new()),
            })
            .collect();
        Ok(Target {
            user: decode(user),
            password: decode(password),
            host: if host.is_empty() {
                "localhost".into()
            } else {
                decode(host)
            },
            port,
            database: decode(database),
            options,
        })
    }

    pub fn option(&self, key: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// A connection to either database.
pub trait Client {
    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Rows, Error>;
    fn exec(&mut self, sql: &str, params: &[Value]) -> Result<Done, Error>;
}

/// Opens `url`, a `postgres://` or `mysql://` URL.
pub fn open(url: &str, wait: Option<Waiter>) -> Result<Box<dyn Client>, Error> {
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        return Ok(Box::new(postgres::Postgres::connect(url, wait)?));
    }
    if url.starts_with("mysql://") || url.starts_with("mariadb://") {
        return Ok(Box::new(mysql::Mysql::connect(url, wait)?));
    }
    Err(Error::new(
        Kind::Connection,
        "the URL should start with postgres:// or mysql://",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_read_into_their_parts() {
        let t = Target::parse(
            "postgres://kiln:s%40cret@db.local:6543/app?sslmode=disable",
            5432,
        )
        .unwrap();
        assert_eq!(t.user, "kiln");
        assert_eq!(t.password, "s@cret");
        assert_eq!(t.host, "db.local");
        assert_eq!(t.port, 6543);
        assert_eq!(t.database, "app");
        assert_eq!(t.option("SSLMODE"), Some("disable"));
        let t = Target::parse("mysql://root@[::1]/shop", 3306).unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.password.as_str()),
            ("::1", 3306, "")
        );
        assert!(Target::parse("mysql://h:port/x", 3306).is_err());
    }
}
