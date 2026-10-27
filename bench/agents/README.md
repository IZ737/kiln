# Agent benchmark

Realistic tasks for measuring how well coding agents write Kiln, and for
catching regressions in what helps them: the compiler's errors and fixes,
`kiln explain`, the [guide for agents](../../docs/guides/agents.md),
`kiln test` and `kiln mcp`.

| Task | What the agent does |
| --- | --- |
| `cli_word_count` | A command-line tool: read a file, count words, a folder module with a typed API, an error exit |
| `http_api` | A JSON API with status codes and validation |
| `ssr_page` | A server-rendered page from a glaze: formatting, a query filter, escaping |
| `db_query` | A folder module that queries a database through `orm`, and a route that uses it |
| `auth_form` | A form only the signed-in person may send, with a CSRF token, validation and per-account updates |
| `use_ware` | Add the kiln/ui Ware and rebuild a form with its components |
| `repair` | Fix code that does not compile and a failing test, without touching the tests |

Each task is a folder:

- `task.md`: the prompt the agent gets. `{{KILN_UI}}` becomes the path of
  [`wares/ui`](../../wares/ui).
- `start/`: the project the agent starts from.
- `check/`: hidden checks, copied over the agent's work when it is done:
  `test_bench_*` tests (unit tests in the module's folder, integration
  tests in `tests/integration/`), and in `run/`, runs of the program with
  their arguments (`NAME.args`), output (`NAME.out`) and exit code
  (`NAME.code`). A file in `check/` that is also in `start/` is restored
  from `check/`, and changing it counts against the agent.
- `solution/`: a reference solution, laid over `start/`.

`cargo test -p kiln_cli --test bench` keeps the tasks fair: every starting
project fails its checks, and every solution passes them, with the
compiler in the repository.

## Running an agent

```sh
cargo build -p kiln_cli
python3 bench/agents/run.py --agent 'claude -p "$(cat "$KILN_BENCH_PROMPT")" --allowedTools Bash,Edit,Write,Read' \
    --kiln target/debug/kiln --out results.jsonl
python3 bench/agents/run.py --agent oracle      # the reference solutions, to check the harness
```

The agent command runs with a shell in a fresh copy of the starting
project. It gets the prompt on standard input and as the file
`$KILN_BENCH_PROMPT`, and a `kiln` on its `PATH` that records each call.
`--tasks a,b` picks tasks, `--timeout` limits each one (30 minutes by
default) and `--keep` keeps the folders to read what the agent did.

## What is measured

One JSON line per task:

| Field | |
| --- | --- |
| `solved` | `kiln check` reports nothing, every test passes (the agent's own and the hidden ones) and every expected run matches |
| `compiles`, `check_errors` | Whether the finished project type-checks, and how many errors are left |
| `hidden_tests`, `hidden_passed` | The hidden checks, and how many passed |
| `runs`, `runs_passed` | The expected runs of the program |
| `kiln_calls`, `failed_kiln_calls` | How often the agent ran `kiln`, and how many of those runs failed: the corrective iterations |
| `changed_files`, `protected_changed` | What the agent changed, and which files it was told not to change |
| `failures` | Why a check failed, in the compiler's or the test's words: the quality of what the agent was told |
| `seconds`, `agent_exit` | How long the agent took, and how it ended |

Compare runs of the same agent before and after a change to Kiln: a task
that stops being solved, or takes more failed `kiln` calls, is a
regression in what Kiln tells agents. The benchmark makes no claim about
tokens or cost, and a baseline is only meaningful for one agent and model
at a time. The first task suite already paid for itself: writing
`db_query` found that `json.encode(xs.map(f))` encoded every item as
`null`, which is fixed.
