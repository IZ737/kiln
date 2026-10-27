# Testing

`kiln test` runs a project's unit, integration and browser tests: one
command, tests written in Kiln, and a real Chromium for the browser, with
no JavaScript test framework to install.

```sh
kiln test                     # every test of the project in this folder
kiln test examples/journal --unit
kiln test --browser --filter comment
kiln test --json              # one JSON report, for editors, CI and agents
```

## Writing tests

A test is a function named `test_*` with no parameters, in a file named
`test_*.kiln`. Where the file sits says what kind of test it is:

| Kind | Where | What it can do |
| --- | --- | --- |
| Unit | Any folder module, beside the code | Call the module's functions, private ones too |
| Integration | `tests/integration/` | Start the app and send it requests; use a database |
| Browser | `tests/browser/` | Drive a real browser against the app |

A test file belongs to its folder's module, so a unit test in `shared/`
calls `post_problems(...)` without a prefix. Test files are only compiled
by `kiln test`: `kiln build` and `kiln run` leave them out. Tests use
the `test` module:

```kiln
import test

fn test_a_post_needs_a_title() {
    let checked = post_problems("", "body")
    test.is_false(checked.valid)
    test.equal(checked.messages(), ["title is required"])
}
```

A test passes when it returns. It fails at the first assertion that does
not hold, or when it throws: a failing `try` is reported with the
error's message and the line of the test it came from. Each test runs in a
process of its own, so tests cannot leak state into each other, and they
run in parallel (`--jobs`, by default one per CPU).

### Assertions

Each one fails the test with the expected and actual values and the line
that called it.

| Function | |
| --- | --- |
| `test.equal(actual, expected)` | Equal, for any type the two share |
| `test.not_equal(actual, other)` | Different |
| `test.is_true(value, message: "...")`, `test.is_false(...)` | A `Bool` |
| `test.contains(text, part)` | `text` contains `part` |
| `test.present(value)` | An optional holds a value; returns it |
| `test.fails(fn() { ... })` | The function throws; returns the error's message |
| `test.fail("message")` | Fail now |
| `test.temp_dir()` | A folder of the test's own, removed after it |

### Integration tests

An integration test reaches the app over HTTP. kiln builds the project's
program once, and starts it for each test that uses it on a free port of
`127.0.0.1`, with:

- a SQLite database of the test's own, with the project's migrations
  applied, as `DATABASE_URL` (`test.database_url()` gives it to the test
  too);
- the variables in `kiln.toml`'s `[test] env`;
- `KILN_LISTEN`, which makes `app.listen` use the port kiln chose,
  whatever address the program asks for.

```kiln
import test

fn test_signing_up_signs_in() {
    let form: Map<String, String> = Map()
    form["email"] = "ada@example.com"
    form["password"] = "correct horse battery"
    let res = test.post_form("/sign-up", form)
    test.equal(res.status, 302)
    test.contains(test.get("/").body, "Sign out")
}
```

| Function | |
| --- | --- |
| `test.get(path)` | A `GET` |
| `test.post_form(path, fields)` | A form, encoded as a browser does |
| `test.post_json(path, body)` | JSON |
| `test.request(method, path, body: "", headers: map)` | Anything else |
| `test.base_url()` | The app's address, such as `http://127.0.0.1:41234` |

A `test.Response` has `status`, `headers` and `body`, and
`res.header("location")` reads one header. Redirects are not followed, so
a test sees the `303` it expects. Cookies the app sets are kept for the
rest of the test, and a form or JSON request carries the app's own
`Origin`, as a browser's does; set `origin` in `headers` to act as
another site.

### Browser tests

`test.page()` opens a page in a real Chromium, in a fresh browser context,
so cookies and storage start empty. Elements are found the way a person
finds them: by role and accessible name, by label, by text, by
`data-testid`, and as a last resort by CSS selector. Finding an element
waits for it to be there, visible, enabled, hydrated (inside an ember) and
not covered, up to the test's timeout, so a test needs no sleeps.

```kiln
import test

fn test_commenting() {
    let page = test.page()
    page.goto("/posts/1")
    page.label("Comment").fill("Nice post")
    page.role("button", name: "Add comment").click()
    page.css("#comments li").expect_text("Nice post")
    test.equal(page.errors(), [])
}
```

| On a page | |
| --- | --- |
| `goto(path)`, `reload()` | Load a page of the app and wait for it |
| `path()`, `title()`, `text()` | Where it is, its title, its visible text |
| `wait_for_path(path)` | Wait until the page is at `path`, as after a form is sent |
| `press(key)`, `type_text(text)` | The keyboard, where the focus is: `Tab`, `Enter`, `Escape`, `a` |
| `focused()` | The focused element, as `textbox "Title"` |
| `errors()` | Uncaught exceptions, console errors and failed requests |
| `viewport(width, height)` | Resize; `test.page(width: 360, height: 640)` opens small |
| `screenshot(name)` | Save a screenshot in the test's results |
| `role(role, name: "...")`, `label(text)`, `by_text(text)`, `test_id(id)`, `css(selector)` | Find elements |

| On what was found | |
| --- | --- |
| `click()`, `fill(text)`, `select(option)`, `check()`, `uncheck()`, `focus()`, `press(key)` | Act like a person: real mouse and keyboard events |
| `text()`, `value()`, `attribute(name)`, `style(property)`, `count()` | Read |
| `is_visible()`, `is_focused()`, `is_checked()` | Ask |
| `wait()`, `expect_text(text)`, `expect_gone()` | Wait for something to be true |

When a browser test fails, kiln saves a screenshot of each open page,
its errors and the app's log in `.kiln/test-results/<test>/`, and the
report links them.

**The browser.** kiln looks for Chromium in this order: `KILN_TEST_BROWSER`,
`[test] browser` in `kiln.toml`, the one `kiln test --install-browser`
downloaded (a pinned Chrome for Testing, kept in kiln's cache), a
Playwright browser cache, then an installed Chrome or Chromium. Nothing
else is needed: kiln talks to the browser itself. Browser tests run
headless; `--headed` shows the window.

## Running tests

| Flag | |
| --- | --- |
| `--unit`, `--integration`, `--browser` | Only these kinds (they combine) |
| `--filter TEXT` | Only tests whose name contains `TEXT` |
| `--list` | List the tests without running them |
| `--json` | Print one JSON report (below) instead of text |
| `--junit FILE` | Also write JUnit XML, for CI |
| `--watch` | Run again whenever a file of the project changes |
| `--jobs N` | How many tests run at once |
| `--timeout SECONDS` | How long one test may take (default `[test] timeout`, or 60) |
| `--retries N` | Run a failing test again; one that passes on a retry is reported as flaky |
| `--headed` | Show the browser |
| `--install-browser` | Download the pinned browser, and stop |
| `--locked`, `--offline` | As for `kiln build`: see [Wares](wares.md) |

A test's name is its module and function: `shared.test_titles_have_a_limit`,
`tests.browser.test_commenting`.

**Exit codes:** `0` every test passed; `1` a test failed; `2` the tests
could not be built, or the command was wrong; `3` the runner itself failed
(a browser that would not start, for instance).

**Configuration** in `kiln.toml`:

```toml
[test]
timeout = 60                              # seconds per test
env = { APP_HTTP = "1" }                  # for the app the tests start
browser = "/usr/bin/chromium"             # a browser of your choosing
```

### The JSON report

`kiln test --json` prints one document, version 1:

```json
{
  "version": 1,
  "tests": [
    {
      "id": "shared.test_titles_have_a_limit",
      "kind": "unit",
      "module": "shared",
      "function": "test_titles_have_a_limit",
      "file": "shared/test_rules.kiln", "line": 9, "column": 1,
      "status": "failed",
      "duration_ms": 104,
      "retries": 0,
      "failure": {
        "message": "the values differ",
        "at": "shared/test_rules.kiln:10:5",
        "expected": "false",
        "actual": "true"
      },
      "output": "",
      "artifacts": []
    }
  ],
  "summary": { "total": 1, "passed": 0, "failed": 1, "errors": 0, "duration_ms": 412 }
}
```

`status` is `passed`, `failed` (an assertion) or `error` (the test
crashed, timed out or the runner could not do what it asked). `kiln test
--list --json` gives the same fields without results.
