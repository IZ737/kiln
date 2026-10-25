# Vendored code

`bdwgc/` is the Boehm-Demers-Weiser garbage collector, version 8.2.8, from
<https://github.com/ivmai/bdwgc/releases/tag/v8.2.8>. Only the files needed
to build it as one translation unit (`extra/gc.c`) are kept: the `.c` files
at the top level, `include/`, and `extra/gc.c`. Its license is at the end of
`bdwgc/README.QUICK` (an MIT-style license). `build.rs` compiles it for the
runtime that native executables link with.

To update it, replace these files with the same ones from a newer release
and run the conformance tests.
