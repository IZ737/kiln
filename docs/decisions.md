# Language decisions

Kiln's design decisions, newest first, with why each was made and what it
means for existing code. A decision here is settled until new evidence
reopens it; the [design overview](design.md) gives the goals and the
[reference](README.md#language-reference) the rules as they stand.

Phase 6.5 reviews the whole language against the [vision](vision.md); its
decisions are added here as they are made.

| Date | Decision | Why | Compatibility |
| --- | --- | --- | --- |
| 2026-10-27 | A Glaze page never serves a route by itself; every route is registered in code, such as `app.get("/", fn(req) { web.render("home.glaze") })` | The vision keeps routing explicit in application code: Glaze files are views and components, not URLs | None: this was never implemented |
| 2026-10-26 | WasmGC is the only browser target through 1.0; pages keep their server-rendered HTML where it is missing | Phase 4 confirmed the browser support the plan assumed; a JavaScript backend stays a later option | None |
