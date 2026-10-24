# Standard library

The standard library's signatures are Kiln files in
[`crates/kiln_check/std/`](../crates/kiln_check/std). The prelude is visible
everywhere; every other module is imported by name (`import fs`). This page
lists the modules the compiler can build into native programs today. The
others (`web`, `sql`, `auth`, `json`) type-check, and building a program that
uses them reports E0401 until their phase arrives.

## Prelude

`print`, `env` and `panic`, and the methods of `Int`, `Float`, `String`,
`Duration`, `List`, `Map` and `Set`. `Duration` has `to_millis() -> Int`
and `to_seconds() -> Float`.

## `time`

Times are UTC. A `time.Instant` is a point in time, stored as `unix_nanos`,
nanoseconds since 1970-01-01.

| Function | |
| --- | --- |
| `time.now() -> Instant` | The current time |
| `time.unix(seconds: Int) -> Instant` | The instant `seconds` after 1970 |
| `time.since(start: Instant) -> Duration` | Time passed since `start` |
| `time.sleep(duration: Duration)` | Pauses the program |
| `time.today() -> Date` | Today's date |
| `time.parse(text: String) -> Instant !` | Reads RFC 3339 (`2026-10-24T21:30:00Z`, `...+02:00`) or a date alone |

`Instant` methods: `unix()` (whole seconds), `add(duration)`,
`since(earlier)`, `date()`, `iso()` and `format(pattern)`.

```kiln
import time

fn main() {
    let start = time.now()
    print(start.format("EEEE, MMMM d, yyyy 'at' HH:mm"))  // Saturday, October 24, 2026 at 21:30
    print(time.since(start).to_millis())
}
```

Format patterns: `yyyy` and `yy` years; `MMMM` (October), `MMM` (Oct),
`MM` and `M` months; `dd` and `d` days; `EEEE` (Saturday) and `EEE` (Sat);
`HH`, `H`, `hh` and `h` hours with `a` for AM or PM; `mm` minutes; `ss`
seconds; `SSS` fractions of a second. Text in single quotes is copied, and
`''` is a quote.

## `fs`

Paths are relative to the working directory. Functions that touch the disk
can fail with an `Error` whose message names the path and the reason, such
as `cannot read notes.txt: No such file or directory`.

| Function | |
| --- | --- |
| `fs.read(path) -> String !` | The whole file |
| `fs.read_lines(path) -> List<String> !` | Lines without `\n` or `\r\n` |
| `fs.write(path, text) !` | Replaces the file, creating it if needed |
| `fs.append(path, text) !` | Adds to the end, creating the file if needed |
| `fs.exists(path) -> Bool` | |
| `fs.is_dir(path) -> Bool` | |
| `fs.list(path) -> List<String> !` | Names in a directory, sorted |
| `fs.mkdir(path) !` | Creates the directory and its parents |
| `fs.remove(path) !` | Removes a file or an empty directory |
| `fs.rename(from, to) !` | |

## `os`

| Function | |
| --- | --- |
| `os.args() -> List<String>` | Command-line arguments, without the program's name |
| `os.env(name) -> String?` | An environment variable, or none when unset |
| `os.exit(code: Int) -> Never` | Ends the program |
| `os.cwd() -> String` | The working directory |
| `os.temp_dir() -> String` | `$TMPDIR`, or `/tmp` |
| `os.pid() -> Int` | The process id |
| `os.platform() -> String` | `"linux"`, `"macos"`, `"windows"` or `"unknown"` |

`fs` and `os` need the server, so embers cannot use them.

## Adding to the standard library

1. Declare the signature in `crates/kiln_check/std/<module>.kiln`, and list a
   new module in `STD_MODULES` in `crates/kiln_check/src/checker.rs`.
2. For the compiler, add the module to `LIB_MODULES` in
   `crates/kiln_ir/src/lower.rs`; its calls become `ExprKind::Lib`. Build
   each function in `lib_call` in `crates/kiln_codegen/src/emit.rs` and the
   runtime (`runtime/kiln.c`). A runtime function that fails sets
   `k_lib_error` to its message.
3. Add a conformance program.
