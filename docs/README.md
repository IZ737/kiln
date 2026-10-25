# Kiln documentation

This directory separates **current language and tooling reference** from
**implementation plans**. The root [README](../README.md) contains the
project overview, installation instructions and phase roadmap.

## Language reference

These pages document the syntax, types and APIs **currently supported or
recognized** by Kiln's compiler and checker; some parsed/checkable features
are not yet executable. Check their status notes before relying on a feature.

- [Kiln syntax](reference/syntax.md) — source files, declarations and expressions
- [Glazes](reference/glazes.md) — template/component syntax and type checking
- [Type rules](reference/types.md) — inference, records, optionals and more
- [Standard library](reference/stdlib.md) — available modules and APIs

## Guides

- [Editor setup](guides/editors.md) — LSP, VS Code, Neovim and Helix

## Project direction

- [Design overview](design.md) — architecture and technical decisions
- [Long-term vision](vision.md) — principles, ecosystem and future scope
- [Roadmap](../README.md#roadmap) — milestones and completion status

## Implementation plans

Plans describe work being implemented, upcoming milestones and intended
APIs. **Proposed syntax in a plan is not automatically implemented syntax.**

- [Phase 2.5 — VS Code developer experience](plans/phase-2.5.md) (completed)
- [Phase 3 — Glazes, native styling, Finishes and themes](plans/phase-3.md)
- [Phase 4 — SQL, ORM, migrations, forms, authentication and admin](plans/phase-4.md)
- [Moving the runtime to Rust](plans/runtime-rust.md)
- [Integrated testing — unit, integration and browser](plans/testing.md)

For changes to the language, update the relevant reference page as the
feature lands. For scope/sequence changes, update the appropriate plan and
the [roadmap](../README.md#roadmap).
