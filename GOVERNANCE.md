# Kiln governance

Kiln is an independent, **maintainer-led** open-source project. Its direction is guided by the goal of making full-stack web development cohesive without sacrificing performance, type safety, or control.

## Who decides?

The project maintainer, [@IZ737](https://github.com/IZ737), has final responsibility for language design, public APIs, compatibility, roadmap priorities, release timing, and what is merged. This keeps the language coherent while it is still rapidly changing before 1.0.

Contributors are welcome to challenge assumptions, share measurements, identify edge cases, and propose alternatives. Ideas are evaluated on technical merit, developer experience, long-term maintainability, security, scope, and alignment with the [vision](docs/vision.md). A proposal is not a promise of adoption, and majority preference alone does not determine language semantics.

## How decisions happen

- **Bug fixes, documentation, and focused internal improvements** can usually go straight to a PR.
- **Public syntax/API, runtime semantics, breaking changes, ecosystem design, or major dependencies** should start as an [issue](https://github.com/IZ737/kiln/issues/new/choose). Describe the problem, alternatives, compatibility and migration considerations, and a possible testable acceptance criterion.
- The maintainer may accept the direction, request revisions or an experiment, defer it to a later milestone, or decline it. Important accepted changes should be reflected in the relevant design/reference documents and tests.
- CI is a baseline for mergeability, not a replacement for architecture review.
- The [roadmap](README.md#roadmap) and [plans](docs/README.md#implementation-plans) provide working priorities. Experimental ideas in [the vision](docs/vision.md) are not automatically commitments.

## Releases and stability

Kiln is pre-1.0. Breaking changes may occur, particularly in unfinished features; communicate user-facing changes through documentation, the [changelog](CHANGELOG.md), and release notes where appropriate. The maintainer decides when a feature is sufficiently tested and stable to ship.

This model can evolve as the project and contributor community grow. Governance changes should be proposed and discussed openly rather than assumed.
