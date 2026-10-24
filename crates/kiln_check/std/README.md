# Standard library signatures

These files declare the types and function signatures of the standard
library so the type checker can check programs that use it. Function bodies
here are placeholders and are never checked; the implementations arrive
with the runtime in later phases.

Three type names exist only in these files:

- `Any` accepts a value of any type.
- `Rest` as the last parameter accepts any number of extra arguments.
- `Never` is the type of an expression that never produces a value.

`prelude.kiln` is visible in every file without an import. Every other file
is a module, imported by its file name (`import web`).
