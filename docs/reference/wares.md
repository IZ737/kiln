# Wares

A Ware is a Kiln package: source folders that other projects import. A
project names its Wares in `kiln.toml`, kiln pins the exact releases in
`kiln.lock`, and every project on the machine shares one copy of each
release in a per-user cache. Nothing is copied into the project but the
lockfile, and a program without a manifest needs none of this.

```toml
[package]
name = "guestbook"
entry = "main.kiln"

[dependencies]
ui = "kiln/ui@0.1"
```

```kiln
import web, ui            // the Ware's root folder, by the name kiln.toml gives it
import ui.forms           // a folder inside it, if it has one
```

A Ware is compiled with the program as source, exactly like the
project's own folders: its `pub` declarations are its API, its private
names stay private, its Glazes and Embers are components (`<ui.Button>`),
and its CSS and static files are served with the app. Types from a Ware
are distinct from look-alike types elsewhere.

## Names and versions

A Ware's name is `publisher/name`. Both parts are lower-case: letters,
digits and `_`, starting with a letter, at most 64 characters. Nothing is
normalized, so `Kiln/UI` is an error rather than another spelling of
`kiln/ui`. The publishers `std`, `core`, `builtin` and `local_ware` are
reserved.

Releases have [SemVer](https://semver.org) versions, and requirements use
Cargo's syntax: `0.1` and `^0.1` allow `>=0.1.0, <0.2.0`; `~1.2`,
`>=1.1, <2` and `=1.2.3` mean what they say; `*` allows anything.

Resolution is deterministic. Each Ware gets the highest release that every
requirement on it allows, or the locked release while it still fits. A
program holds **one** release of each Ware: when two requirements cannot
both be met, the error lists each requirement and who made it. Yanked
releases are never chosen for a new lock, but a lockfile that already
holds one keeps working.

## `kiln.toml`

The manifest is optional. Every key is checked, and an error points at the
key it is about.

```toml
[package]
name = "guestbook"          # an app's name is free text; a Ware's is publisher/name
kind = "app"                # "app" (the default) or "ware"
entry = "main.kiln"         # an app's entry file; a Ware has none
version = "0.1.0"           # required for a Ware
description = "..."
license = "MIT"
capabilities = []           # a Ware's: what its code may reach (below)

[dependencies]
ui = "kiln/ui@0.1"                                       # from the registry
forms = { ware = "acme/forms", version = ">=1.1, <2" }
local = { ware = "acme/local", path = "../local" }     # a folder, used live
theme = { ware = "acme/theme", git = "https://example.com/theme.git", tag = "v1.0.0" }

[registry]
url = "https://wares.example.com"    # where registry Wares come from

[policy]                    # limits on every Ware in the graph, indirect ones too
publishers = ["kiln", "acme"]
sources = ["registry", "path"]       # registry, git, path
capabilities = ["network"]
trust = "verified-publisher"         # the weakest trust tier accepted

[test]                      # see testing.md
timeout = 60
env = { APP_HTTP = "1" }
browser = "/usr/bin/chromium"
```

The key in `[dependencies]` is the name the project imports the Ware by.
A dependency has one source:

| Source | Written | Locked as |
| --- | --- | --- |
| Registry | `"publisher/name@req"` or `{ ware, version, registry }` | The release and the SHA-256 of its archive |
| Path | `{ ware, path }` | The relative path; its source is read live, so editing it changes the build |
| Git | `{ ware, git, rev \| branch \| tag }` | The full commit; a branch or tag is resolved once |

A `rev` is a full 40-character commit hash. Git Wares are read from a bare
mirror in the cache without checking anything out, and Git runs no hooks.
A Ware from a registry or Git cannot depend on a local path.

The registry is `[registry] url`, or `KILN_REGISTRY`. A registry must use
HTTPS, except on this machine (`127.0.0.1`, `localhost`, `::1`). Kiln does
not run a public registry yet; [the registry in this repository](../../registry/main.kiln)
is a complete one, written in Kiln, that you can run yourself.

## `kiln.lock`

kiln writes the lockfile whenever it resolves; commit it. It holds each
Ware's version, source, checksum, capabilities and dependency edges, plus
the trust tier the registry stated when it was locked. Keys are sorted and
lines end in `\n`, so one resolution makes the same bytes on every system.

```toml
# The exact Wares this project builds with, written by kiln. Commit it.
version = 1

[root]
ui = "kiln/ui"

[[ware]]
name = "kiln/ui"
version = "0.1.0"
source = "registry+https://wares.example.com"
checksum = "sha256:3b1f..."
capabilities = []
dependencies = {}
```

`kiln build`, `run`, `check` and `test` resolve when `kiln.toml` changed
and say what they changed in the lock. Two flags make builds reproducible:

- `--locked`: use `kiln.lock` exactly as it is, and fail if `kiln.toml`
  asks for something it does not hold.
- `--offline` (or `KILN_OFFLINE=1`): make no network requests; every Ware
  must already be in the cache (`kiln fetch` fills it).

`kiln build --locked --offline` on a CI machine with a filled cache builds
the same program on every run. Editors and `kiln mcp` never write
`kiln.lock` and never reach the network.

## The cache

Each release's source is kept once per user, in a folder named by its
content hash, and shared by every project. A download is unpacked into a
staging folder, checked against its SHA-256 and moved into place in one
rename, so a crash or a second kiln never leaves half a Ware. A cached
tree is checked again before it is used, so an edited cache is an error,
not a different program.

| | |
| --- | --- |
| Where | `KILN_CACHE_DIR`; otherwise `$XDG_CACHE_HOME/kiln` or `~/.cache/kiln` on Linux, `%LOCALAPPDATA%\kiln\cache` on Windows, `~/Library/Caches/kiln` on macOS |
| `KILN_CACHE_READONLY=1` | Read the cache, never write it, as on a CI machine whose cache was filled earlier |
| `kiln cache dir` / `stats` | Where it is and what it holds |
| `kiln cache verify` | Check every cached Ware against its checksum |
| `kiln cache prune [--older-than DAYS] [--dry-run]` | Remove Wares not used for a while, and leftovers of interrupted downloads |

## Capabilities and trust

A Ware declares what its code may reach, and kiln checks the declaration
against the standard modules its source imports. Using one without
declaring its capability is an error when the Ware is locked.

| Capability | Granted by |
| --- | --- |
| `filesystem` | `fs` |
| `network` | `web` |
| `database` | `sql`, `orm`, `auth`, `admin` |
| `environment` | `os`, and the prelude's `env(...)` |
| `foreign` | `js` and `npm.*` |

This reads imports; it is **not a sandbox**. A Ware's code runs with the
program's full rights, so read what you add. When `kiln add` or
`kiln update` would bring in a Ware that needs a capability, or a new
release that needs more than the locked one, kiln lists them and asks;
`--allow-capabilities` accepts them without asking (for scripts).

A registry states a trust tier for each release, weakest first:
`unverified`, `verified-publisher` (an administrator checked who the
publisher is) and `peer-reviewed` (someone reviewed the release). A
project's `[policy] trust` refuses anything weaker; `publishers`,
`sources` and `capabilities` limit the rest of the graph the same way.
`kiln ware verify <name>` checks a locked Ware's checksum and declared
capabilities again.

## Commands

| Command | |
| --- | --- |
| `kiln add kiln/ui[@req]` | Add a registry Ware to `kiln.toml` and lock it. `--version`, `--path DIR`, `--git URL` with `--rev`, `--branch` or `--tag`, `--registry URL`, `--as NAME` |
| `kiln remove NAME` | Remove it from `kiln.toml` and `kiln.lock` |
| `kiln update [NAME...]` | Choose the newest releases `kiln.toml` allows |
| `kiln tree [--why NAME]` | The locked graph, with sources, trust and capabilities; `--why` shows the chains that bring a Ware in |
| `kiln fetch` | Download every locked Ware into the cache, for offline builds |
| `kiln publish [DIR] [--dry-run]` | Publish the Ware in a folder |
| `kiln login --registry URL` | Save a publish token, read from standard input |
| `kiln yank publisher/name@1.2.3 [--undo]` | Stop new locks from choosing a release |
| `kiln ware verify NAME` | Check a locked Ware against `kiln.lock` |
| `kiln ui add NAME...` / `kiln ui list` | Copy kiln/ui components into the project, to own |
| `kiln new DIR` / `kiln new .` | A new project, in a new folder or the current one (see [Starting a project](projects.md#starting-a-project)) |

## Publishing

A Ware is a folder with a manifest whose `kind` is `ware`:

```toml
[package]
name = "acme/forms"
kind = "ware"
version = "1.2.0"
description = "Form helpers"
license = "MIT"
capabilities = []
```

`kiln publish --dry-run` lists the files that would be sent, with the
archive's size and checksum, and checks the declared capabilities; it
sends nothing. A published Ware depends only on registry Wares. A Ware's archive is its UTF-8 text files in a canonical
form, at most 16 MiB and 4096 files; hidden files, `kiln.lock`, `target`,
`node_modules`, databases and key files (`.db`, `.sqlite`, `.pem`,
`.key`, ...) are never published. The SHA-256 of the archive is the
release's checksum, and a published version can never change: publishing
it again with other content is refused. Publishing needs a token, from
`KILN_REGISTRY_TOKEN` or the one `kiln login` saved in
`credentials.toml` in kiln's configuration folder (`KILN_CONFIG_DIR`).

## Registry protocol, version 1

A registry serves JSON over HTTP. kiln treats everything it sends as
untrusted: sizes are limited, every field is checked, and an archive must
match its checksum.

| Request | |
| --- | --- |
| `GET /v1/wares/{publisher}/{name}` | Every release: `version`, `checksum`, `dependencies`, `capabilities`, `yanked`, `trust` |
| `GET /v1/wares/{publisher}/{name}/{version}/archive` | The release's archive |
| `PUT /v1/wares/{publisher}/{name}/{version}` | Publish, with `Authorization: Bearer <token>` and the release as JSON: `name`, `version`, `checksum`, `description`, `license`, `dependencies`, `capabilities` and `archive`. `201` when published, `200` when the same release was already there, `409` for another checksum |
| `POST /v1/wares/{publisher}/{name}/{version}/yank` | Yank or restore, with `{ "yanked": true }` |

Errors are a status and `{ "error": "..." }`. [`registry/`](../../registry/main.kiln)
implements it: publishers sign up, make tokens and see their Wares in
pages built with kiln/ui, and administrators verify publishers and record
reviews in the generated admin.

```sh
kiln db migrate --url sqlite:registry.db --path registry
REGISTRY_HTTP=1 REGISTRY_ADMIN=you@example.com kiln run registry   # on 127.0.0.1:8081
```

## kiln/ui

[`wares/ui`](../../wares/ui/README.md) is the first official Ware:
accessible, server-rendered Glaze components (buttons, fields, select,
checkbox, radio, form errors, alerts, badges, cards and empty states)
styled by the app's design tokens, so a theme restyles them. Use it as a
dependency, or copy components into the project with `kiln ui add`, which
writes them with a note of where they came from and never overwrites a
file you changed. [`examples/guestbook`](../../examples/guestbook/main.kiln)
does both.
