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
- `main.code`, if present, holds the expected exit code, for a program
  that ends itself with `os.exit`.

Add a program here for every feature the compiler learns. The suite passing
on the compiled programs is the exit gate of phase 1.
