# Formatting, live reload and agents

The tools around the compiler: `kiln fmt` for one layout, `kiln dev` for
a server that rebuilds as you type, and `kiln mcp` for coding agents.
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
