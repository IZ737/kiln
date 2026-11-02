# Formatting, live reload and agents

The tools around the compiler: `kiln fmt` for one layout, `kiln dev` for
a server that rebuilds as you type, `kiln fire` for release builds, and
`kiln mcp` for coding agents.
[Static sites](static-sites.md) covers `kiln fire --static`.
[Editor setup](editors.md) covers the language server, and
[Testing](../reference/testing.md) covers `kiln test`.

## `kiln fmt`

```sh
kiln fmt                   # format every .kiln and .glaze file under this folder
kiln fmt shared main.kiln  # these files and folders
kiln fmt --check           # change nothing; list unformatted files and fail if there are any
```

There is one layout and no options. It indents by four spaces following
brackets, continues a line that starts with `.` or follows an operator,
puts one space around operators and after commas, and keeps at most one
blank line. Your line breaks, comments and the order of everything are
kept: kiln fmt changes spacing, never tokens. It checks that, and refuses
to write a file whose tokens or comments would change. In glazes, only
the code inside one-line `{% ... %}` tags is formatted; markup and CSS are
left as they are.

Formatting is idempotent, so `kiln fmt --check` in CI keeps a project
formatted; the language server formats with the same code.

## `kiln dev`

```sh
kiln dev                   # the program in this folder, on http://127.0.0.1:8080
kiln dev examples/journal --port 3000
```

kiln builds the program and runs its server behind a small proxy. When a
file of the project changes it builds again and restarts the server, and
open pages reload. When a build fails, the errors are shown and **the last
good build keeps running**, so the page you are looking at keeps working
while you fix it. The program listens on a free port of its own
(`KILN_LISTEN`); you use the address kiln dev prints.

The reload script is added to HTML pages by the proxy only: a program
built by `kiln build` never has it. kiln dev listens on `127.0.0.1` unless
you give `--host`, and warns when another machine could reach it.

## `kiln fire`

```sh
kiln fire                        # a release build of the program in this folder
kiln fire examples/hello -o hello
kiln fire --backend native       # Cranelift, as kiln build
kiln build --backend llvm --emit-llvm out.ll examples/hello   # the same, and its LLVM IR
```

`kiln build` and `kiln run` make machine code with Cranelift inside
kiln: quick to build, with no tools to install. `kiln fire` makes the
release build: it writes the program as LLVM IR and runs **clang** on it
with `-O2`, so functions are inlined and loops are optimized, then links
the object with the runtime kiln carries, exactly as `kiln build` links
Cranelift's. The executable is as self-contained as any other, and
`--target windows` works from Linux the same way.

kiln looks for clang on the `PATH` (`clang`, or `clang-15` and up), or
runs the one `KILN_CLANG` names; clang 15 or later is needed. Without
one, `kiln fire` builds with Cranelift and says so in one line:

```
note: building without LLVM's optimizer: no clang was found on the PATH; install clang 15 or later (...)
```

`--backend llvm` asks for LLVM and fails instead of falling back, for a
build script that must not quietly ship the slower build. Both builds
behave the same: the conformance programs run through Cranelift, LLVM and
the C backend, and the same checks (integer overflow, division by zero,
bounds) are kept in release builds. How much faster a release is depends
on the program: loops and calls in Kiln code gain the most (1.3 to 3
times faster in the compiler's benchmarks), while a program that spends
its time in the runtime, such as string handling or the database, gains
little, since the runtime is prebuilt either way.

## Visual Studio Code

The extension adds **Kiln: Test Program**, **Start Dev Server** and
**Format Program** to the command palette beside Check, Run and Build,
and the same as tasks (`"type": "kiln", "command": "test"`); the test task
is in the Test group, and compile errors go to the Problems panel. Each
failing test names its `file:line:column` in the terminal. Format
Document, Find All References and Rename Symbol come from `kiln lsp`.

## `kiln mcp`

`kiln mcp` is a [Model Context Protocol](https://modelcontextprotocol.io)
server over standard input and output. It gives a coding agent the
compiler's own answers (the same code as `kiln check`, `kiln fmt` and
`kiln test`), as structured data:

| Tool | | Needs |
| --- | --- | --- |
| `version` | kiln's version, the schema version, the roots and what is allowed | |
| `project_graph` | Modules, their files, imports and `pub` exports with signatures, and Wares | |
| `check` | Diagnostics with codes, positions, help and exact fixes; paged by `offset` and `limit` | |
| `explain` | An error code's explanation, with a wrong and a right example | |
| `ware_tree` | The locked Wares, with sources, trust and capabilities | |
| `format_preview` | What `kiln fmt` would change, without writing | |
| `list_tests` | The project's tests, without running them | |
| `guide` | [The guide for coding agents](agents.md) | |
| `run_tests` | Run `kiln test`, with `filter` and `kinds`; cancellable | `--allow-exec` |
| `format_apply` | Format files in place | `--allow-write` |
| `build` | Build the program into `.kiln/mcp/` | `--allow-write` |

```sh
kiln mcp                                  # read and check the project in this folder
kiln mcp --root ~/code/shop --allow-exec  # and let the agent run its tests
```

Most clients take a command to start. For Claude Code:

```sh
claude mcp add kiln -- kiln mcp --allow-exec
```

and in a client's JSON configuration:

```json
{ "mcpServers": { "kiln": { "command": "kiln", "args": ["mcp", "--allow-exec"] } } }
```

**What it will not do.** Reading and checking never run the project's code,
never write `kiln.lock` and never reach the network. Running tests executes
the project's code (it builds and starts the app, uses a test database and
may drive a browser), so it is off unless the person starting the server
says `--allow-exec`. Writing files needs `--allow-write`. Adding Wares,
publishing, migrating databases and deploying are not offered at all: an
agent does those with the CLI, where the person sees them. Every path must
be inside a root (the current folder, or each `--root`), and long results
are cut. Results carry `"schema": 1`; a change that could break an agent's
use of a tool will come with a new schema version.
