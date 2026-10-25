# Conformance programs

Each folder here is a complete program: its `.kiln` files share one
namespace, and one of them has `fn main()`. `cargo test` runs it with
`kiln run` and compares the result with the files in the folder:

- `main.out` is the expected standard output.
- `main.err` is the expected standard error, for a program that fails. A
  panic exits with code 101 and an error that leaves `main` with code 1.
  Without this file, the program must exit with 0 and write nothing to
  standard error.
- `main.args`, if present, holds the program's arguments, one per line.
- `main.in`, if present, is the program's standard input.
- `main.code`, if present, holds the expected exit code, for a program
  that ends itself with `os.exit` or writes to standard error and succeeds.
- `main.requests` makes the program an HTTP server under test. It gets a
  free port in the `PORT` environment variable, and once it accepts
  connections the test sends each request in the file, separated by lines
  of `###`, on a connection of its own. A request's lines up to the first
  empty line are its head and the rest is its body; the test adds
  `content-length` when there is a body. The last request should make the
  program exit (with `os.exit(0)`). `main.responses` holds the expected
  responses, each after a `### ` line with its request line and without
  the `date` header, and `main.err` what the server logged, with the port
  written as `PORT`.

Add a program here for every feature the compiler learns. The suite passing
on the compiled programs is the exit gate of phase 1.
