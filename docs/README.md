# Kiln documentation

This directory separates **current language and tooling reference** from
**implementation plans**. The root [README](../README.md) contains the
project overview, installation instructions and phase roadmap.

## Language reference

These pages document the syntax, types and APIs **currently supported or
recognized** by Kiln's compiler and checker; some parsed/checkable features
are not yet executable. Check their status notes before relying on a feature.

- [Kiln syntax](reference/syntax.md) — source files, declarations and expressions
- [Projects and modules](reference/projects.md) — local imports, visibility, roots and manifests
- [Glazes](reference/glazes.md) — template/component syntax and type checking
- [Databases, models and migrations](reference/data.md) — `sql`, `orm` and `kiln db`
- [Type rules](reference/types.md) — inference, records, optionals and more
- [Semantics](reference/semantics.md) — values and references, evaluation, numbers and failure
- [Standard library](reference/stdlib.md) — available modules and APIs
- [Wares](reference/wares.md) — packages, `kiln.lock`, the cache and registries
- [Testing](reference/testing.md) — `kiln test`: unit, integration and browser tests

## Guides

- [Your first Kiln site](guides/first-site.md) — from install to a tested multi-folder site
- [Editor setup](guides/editors.md) — LSP, VS Code, Neovim and Helix
- [Formatting, live reload and agents](guides/tooling.md) — `kiln fmt`, `kiln dev` and `kiln mcp`
- [Kiln for coding agents](guides/agents.md) — the guide `kiln docs --llm` prints

## Project direction

- [Design overview](design.md) — architecture and technical decisions
- [Long-term vision](vision.md) — principles, ecosystem and future scope
- [Language decisions](decisions.md) — what was decided, why, and what it changed
- [Roadmap](../README.md#roadmap) — milestones and completion status

## Implementation plans

Plans describe work being implemented, upcoming milestones and intended
APIs. **Proposed syntax in a plan is not automatically implemented syntax.**

- [Phase 2.5 — VS Code developer experience](plans/phase-2.5.md) (completed)
- [Phase 3 — Glazes, native styling, Finishes and themes](plans/phase-3.md)
- [Phase 4 — Embers, WasmGC, reactive UI and typed server functions](plans/phase-4.md)
- [Phase 4.1 — Explicit `.ember` files and server/client component boundaries](plans/phase-4.1.md) (complete; included in v0.9.0)
- [Phase 4.2 — Multi-folder projects and local modules](plans/phase-4.2.md) (complete; included in v0.9.0)
- [Phase 5 — SQL, ORM, migrations, forms, authentication and admin](plans/phase-5.md)
- [Phase 6 — Wares, testing, tooling and agent integration](plans/phase-6.md) (complete; included in v0.11.0)
- [Phase 6.5 — Language review and vision alignment](plans/phase-6.5.md) (complete; included in v0.12.0)
- [Moving the runtime to Rust](plans/runtime-rust.md)
- [Integrated testing — unit, integration and browser](plans/testing.md) (implemented as `kiln test`)

For changes to the language, update the relevant reference page as the
feature lands. For scope/sequence changes, update the appropriate plan and
the [roadmap](../README.md#roadmap).
