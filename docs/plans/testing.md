# Integrated testing — planned Kiln toolchain capability

**Status:** Planned. The public-facing development workflow is a single
`kiln test` command for unit, integration and **automated real-browser
frontend/end-to-end tests**. This is distinct from Kiln's existing compiler
conformance tests and CI checks: application developers should not need to
assemble a separate test framework and browser automation stack.

**Roadmap:** Phase 4 delivers Embers with compiler conformance and real-
browser CI smoke tests; Phase 5 adds SQL/auth and associated browser
integration regressions. **Phase 6** delivers the public `kiln test` runner
after Glazes (3), Embers (4) and database/auth (5); its initial browser
mode must support hydration, events, reactive state and typed RPC.
Do not change earlier phases' acceptance gates to depend on this public CLI.

## What one runner means

| Kind | Runs | Example responsibilities |
| --- | --- | --- |
| Unit | Kiln functions/modules in a controlled test process | Assertions, errors, fallible calls, deterministic inputs, table/parameterized cases |
| Integration | Real Kiln modules and bounded application resources | HTTP routes, JSON, database queries/migrations, auth/session flows, jobs, fixtures and setup/teardown |
| Browser / end-to-end | A **real browser** against a launched Kiln app | Navigation, forms, inputs, layout/state, keyboard focus, selectors, network interactions, accessible roles and responsive viewports |

One runner discovers test suites by a predictable convention, compiles
Kiln-authored tests with the normal type checker and reports results
consistently. The three modes may share assertions, fixtures, setup,
teardown and mock/test-data helpers. Assertions should show actual versus
expected values and link to original Kiln/Glaze source spans. Tests should
have reproducible results and clear failure causes.

## Target CLI (illustrative, not implemented)

```sh
kiln test                  # all configured suites
kiln test --unit           # fast pure/business-logic checks
kiln test --integration    # app/API/database tests
kiln test --browser        # headless real-browser end-to-end tests
kiln test --filter login   # focused iteration
kiln test --watch          # rerun relevant tests during development
kiln test --json           # machine-readable output for IDEs/CI/agents
```

Exact CLI flags and Kiln test-file syntax are not fixed until implemented.
Avoid inventing a parallel assertion DSL if ordinary Kiln functions and
types can express the needed behaviour.

## Crucial: browser testing is built into the Kiln experience

Browser testing must be a **first-class mode of `kiln test`**, rather
than an optional instruction to install Playwright, Node, Jest or a
different developer-facing JavaScript test project.

- Start/stop the built Kiln web server automatically where possible,
  select an unused port, make its URL available as a typed fixture, and
  clean up even when tests fail.
- Provide ergonomic Kiln APIs for browser navigation, querying the DOM,
  interacting with elements, typing, submitting forms, keyboard input,
  checking text/attributes and asserting URL/state changes.
- Prefer **accessible-role/label-based selectors** over brittle CSS paths.
  Assert focus order, error messages and semantic accessibility where
  relevant; automated tests assist but cannot prove full accessibility.
- Support headless local/CI runs by default and opt-in headed debugging.
  On failures provide reproducible logs plus **screenshots**, and where
  practical traces/video and relevant HTTP/console errors.
- Exercise screen sizes/media queries and CSS/themed Glaze pages. Add
  screenshot-based *visual regression* as an optional later mode with
  managed baselines and platform/rendering variance controls, not a
  mandatory brittle screenshot assertion in every test.
- From Phase 6's first release, the same test syntax covers Ember
  behaviour, hydration and browser/server RPC alongside SSR Glazes.
- Keep it possible to test an existing externally hosted site when
  explicitly configured, but never accidentally run destructive tests
  against production by default.

**Reuse a maintained automation protocol/engine** (e.g. an established
WebDriver or Chrome DevTools/Playwright-compatible backend), not a custom
browser implementation. The Kiln CLI/API owns discovery, fixtures,
test orchestration, assertions, diagnostics, environment isolation,
configuration and results. Browser binaries are external platform assets;
provide a predictable install/provisioning command or automatic setup
with pinned browser versions and cache support. "Built-in" refers to
the **developer workflow**, not embedding Chromium inside every compiled
Kiln application or claiming no browser download is ever required.

## Test infrastructure and reliability

- Isolated test data/config, fixtures, disposable databases, deterministic
  clocks and network mocks where needed; explicit opt-in for real external
  services.
- Parallel runs where tests are independent; resource locking and
  test-specific ports/databases where they are not.
- Retries are visible and opt-in; don't hide flaky or nondeterministic
  failures behind automatic success.
- Timeouts, process cleanup, failure summaries and CI exit codes should
  work on Linux and Windows.
- Compatible code coverage/report formats, machine-readable output,
  IDE links and easy CI integration; unit suites should remain fast.
- Document trust boundaries: browser tests can exercise live systems,
  so external URLs, credentials and potentially destructive operations
  should be explicit and constrained.

## Delivery checkpoints

**Phase 6 acceptance**

- [ ] A `kiln test` invocation discovers and executes Kiln-written
      unit tests, reports useful failures and gives a nonzero exit code.
- [ ] Integration tests can provision/teardown an isolated Kiln app
      with its HTTP/API and a test database.
- [ ] Browser tests can launch a real headless browser, visit a compiled
      Kiln/Glaze page, fill a form, submit it, navigate, assert content,
      keyboard/focus behaviour and CSS theme differences.
- [ ] The runner handles a failing browser test with screenshots/logs,
      source-linked diagnostics and nonzero CI exit status.
- [ ] Developers can run all three kinds via the same CLI with **no
      hand-maintained Node or JS testing project**; browser setup and
      caching are documented.
- [ ] The implementation reuses maintained browser engines/protocols
      and passes reproducible smoke suites on Linux and Windows.

**Ember coverage within Phase 6 acceptance**

- [ ] The browser suite verifies an Ember's hydration, reactive updates,
      events and calls to typed server functions using the same runner
      and assertions as other application and Glaze tests.

## Boundaries

This does **not** require a custom JS execution engine, a new browser,
a separate testing language, automatic screenshot testing of every Glaze,
or shipping browser binaries inside final server executables.

**Principle:** From a pure function to a database-backed HTTP route to
the page someone clicks, **one language and one test command** should
provide the developer's complete confidence loop.
