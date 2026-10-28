//! `kiln new`: a starter project in a new folder or the current one. It
//! writes files only; nothing is run or fetched. Every file is checked
//! before any is written, so a folder that already has one of them is
//! left as it was.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

/// The kinds of project `kiln new` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Template {
    /// A web app with its routes in `main.kiln`, a layout, a page and CSS.
    Web,
    /// A web app whose pages are files in `pages/`, served by
    /// `app.file_routes` and exportable with `kiln fire --static`.
    Pages,
    /// A command-line program, with no web code.
    Cli,
}

impl Template {
    const ALL: [Template; 3] = [Template::Web, Template::Pages, Template::Cli];

    fn describe(self) -> &'static str {
        match self {
            Template::Web => "web app (routes in main.kiln)",
            Template::Pages => "web app with pages as files (pages/, static export)",
            Template::Cli => "command-line program",
        }
    }

    fn web(self) -> bool {
        self != Template::Cli
    }
}

pub struct Options {
    pub dir: PathBuf,
    pub template: Option<Template>,
    pub name: Option<String>,
    pub ember: bool,
    pub yes: bool,
}

/// Asks the person at the terminal; `None` when there is none.
trait Ask {
    fn ask(&mut self, question: &str) -> Option<String>;
}

struct Terminal;

impl Ask for Terminal {
    fn ask(&mut self, question: &str) -> Option<String> {
        if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            return None;
        }
        eprint!("{question}");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer).ok()?;
        Some(answer.trim().to_string())
    }
}

pub fn run(options: Options) -> Result<String, String> {
    new_project(options, &mut Terminal)
}

fn new_project(options: Options, ask: &mut dyn Ask) -> Result<String, String> {
    let dir = &options.dir;
    let name = match &options.name {
        Some(name) if valid_name(name) => name.clone(),
        Some(name) => {
            return Err(format!(
                "`{name}` is not a project name: use letters, digits, `-` and `_`"
            ))
        }
        None => inferred_name(dir).ok_or_else(|| {
            format!(
                "cannot make a project name from `{}`; give one with `--name`",
                dir.display()
            )
        })?,
    };
    if dir.is_file() {
        return Err(format!("{} is a file, not a folder", dir.display()));
    }
    let existing: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();

    let interactive = !options.yes;
    let template = match options.template {
        Some(t) => t,
        None if interactive => choose_template(ask)?,
        None => Template::Web,
    };
    let mut ember = options.ember;
    if ember && !template.web() {
        return Err("`--ember` adds an interactive component to a web app; a command-line program has no page for it".into());
    }
    if !ember && template.web() && interactive && options.template.is_none() {
        if let Some(answer) =
            ask.ask("Add an interactive Ember (a counter) to the home page? [y/N] ")
        {
            ember = yes(&answer);
        }
    }

    let files = files(template, &name, ember);
    // A .gitignore of the user's own is kept; every other file must be new.
    let gitignore_kept = dir.join(".gitignore").exists();
    let conflicts: Vec<&str> = files
        .iter()
        .map(|(file, _)| *file)
        .filter(|file| *file != ".gitignore" && dir.join(file).exists())
        .collect();
    if !conflicts.is_empty() {
        return Err(format!(
            "{} already has {}; kiln new never overwrites files, so nothing was written. Move them away, or make the project in another folder",
            shown(dir),
            conflicts
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !existing.is_empty() && !options.yes {
        let question = format!(
            "{} is not empty; add the {} files of a new project beside what is there? [y/N] ",
            shown(dir),
            files.len() - usize::from(gitignore_kept)
        );
        match ask.ask(&question) {
            Some(answer) if yes(&answer) => {}
            Some(_) => return Err("nothing was written".into()),
            None => {
                return Err(format!(
                    "{} is not empty; `kiln new --yes` adds the project's files beside what is there (it never overwrites any)",
                    shown(dir)
                ))
            }
        }
    }

    for (file, text) in &files {
        let path = dir.join(file);
        if *file == ".gitignore" && gitignore_kept {
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }

    let here = dir.as_os_str().is_empty() || dir == Path::new(".");
    let place = if dir.file_name().is_some_and(|f| f.to_string_lossy() == name) {
        String::new()
    } else {
        format!(" in {}", shown(dir))
    };
    let mut out = format!(
        "made {name}{place}: a {}{}",
        template.describe(),
        if ember { ", with an Ember" } else { "" }
    );
    if gitignore_kept {
        out.push_str("\nkept your .gitignore; add `.kiln/` and `dist/` to it");
    }
    out.push_str("\n\nnext:\n");
    if !here {
        out.push_str(&format!("  cd {}\n", dir.display()));
    }
    match template {
        Template::Web | Template::Pages => {
            out.push_str(
                "  kiln dev        serve it on http://127.0.0.1:8080, reloading on every save\n",
            );
            out.push_str("  kiln test       run its unit and integration tests\n");
            if template == Template::Pages {
                out.push_str(
                    "  kiln fire --static .   write the pages to dist/ for a static host\n",
                );
            } else {
                out.push_str("  kiln fire .     make a release build\n");
            }
        }
        Template::Cli => {
            out.push_str("  kiln run . Ada  run it with an argument\n");
            out.push_str("  kiln test       run its tests\n");
            out.push_str("  kiln fire .     make a release build\n");
        }
    }
    Ok(out.trim_end().to_string())
}

fn choose_template(ask: &mut dyn Ask) -> Result<Template, String> {
    let mut question = String::from("What kind of project?\n");
    for (i, t) in Template::ALL.iter().enumerate() {
        question.push_str(&format!("  {}) {}\n", i + 1, t.describe()));
    }
    question.push_str("Choose 1-3 [1]: ");
    let Some(answer) = ask.ask(&question) else {
        return Ok(Template::Web);
    };
    let answer = answer.to_ascii_lowercase();
    if answer.is_empty() {
        return Ok(Template::Web);
    }
    if let Ok(n) = answer.parse::<usize>() {
        if (1..=Template::ALL.len()).contains(&n) {
            return Ok(Template::ALL[n - 1]);
        }
    }
    match answer.as_str() {
        "web" => Ok(Template::Web),
        "pages" => Ok(Template::Pages),
        "cli" => Ok(Template::Cli),
        _ => Err(format!(
            "`{answer}` is not one of the choices; pass `--template web`, `pages` or `cli`"
        )),
    }
}

fn yes(answer: &str) -> bool {
    matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes")
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The folder's own name, as a project name: `My App` is `my-app`. For
/// `.` it is the current folder's.
fn inferred_name(dir: &Path) -> Option<String> {
    let base = match dir.file_name() {
        Some(name) => PathBuf::from(name),
        None => std::path::absolute(dir).ok()?.canonicalize().ok()?,
    };
    let raw = base.file_name()?.to_string_lossy().to_string();
    let mut name = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            name.push(c.to_ascii_lowercase());
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    let name = name.trim_matches('-').to_string();
    valid_name(&name).then_some(name)
}

fn shown(dir: &Path) -> String {
    if dir.as_os_str().is_empty() || dir == Path::new(".") {
        "this folder".into()
    } else {
        dir.display().to_string()
    }
}

/// The files of a template, with the paths they go at.
fn files(template: Template, name: &str, ember: bool) -> Vec<(&'static str, String)> {
    let mut files: Vec<(&str, String)> = vec![(
        "kiln.toml",
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nkind = \"app\"\nentry = \"main.kiln\"\n"),
    )];
    let counter = if ember { EMBER_USE } else { "" };
    match template {
        Template::Web => files.extend([
            ("main.kiln", WEB_MAIN.to_string()),
            ("layout.glaze", LAYOUT.replace("{nav}", "")),
            ("site.css", CSS.to_string()),
            ("home.glaze", HOME.replace("{ember}", counter)),
            ("notes/notes.kiln", NOTES.to_string()),
            ("notes/test_notes.kiln", NOTES_TEST.to_string()),
            (
                "tests/integration/test_pages.kiln",
                WEB_INTEGRATION.to_string(),
            ),
        ]),
        Template::Pages => files.extend([
            ("main.kiln", PAGES_MAIN.to_string()),
            (
                "pages/_components/layout.glaze",
                LAYOUT.replace("{nav}", NAV),
            ),
            ("pages/_components/site.css", CSS.to_string()),
            ("pages/route.kiln", PAGES_ROUTE.to_string()),
            (
                "pages/index.glaze",
                HOME.replace(
                    "{% layout \"layout.glaze\"",
                    "{% layout \"pages/_components/layout.glaze\"",
                )
                .replace("{ember}", counter),
            ),
            ("pages/about.glaze", PAGES_ABOUT.to_string()),
            ("notes/notes.kiln", NOTES.to_string()),
            ("notes/test_notes.kiln", NOTES_TEST.to_string()),
            (
                "tests/integration/test_pages.kiln",
                PAGES_INTEGRATION.to_string(),
            ),
        ]),
        Template::Cli => files.extend([
            ("main.kiln", CLI_MAIN.to_string()),
            ("greet/greet.kiln", CLI_GREET.to_string()),
            ("greet/test_greet.kiln", CLI_TEST.to_string()),
        ]),
    }
    if ember {
        files.push(("components/counter.ember", EMBER.to_string()));
    }
    let ignore = if template.web() {
        ".kiln/\n*.db\ndist/\n"
    } else {
        ".kiln/\n"
    };
    files.push((".gitignore", ignore.to_string()));
    files
}

const WEB_MAIN: &str = r#"import web, notes

fn main() ! {
    let app = web.app()
    app.get("/", fn(req) {
        return web.render("home.glaze", { items: notes.all() })
    })
    app.get("/health", fn(req) {
        return web.text("ok")
    })
    try app.listen(":8080")
}
"#;

const NOTES: &str = r#"// The notes the home page shows.

pub struct Note {
    title: String
    done: Bool
}

pub fn all() -> List<Note> {
    return [Note(title: "Read the Kiln docs", done: true), Note(title: "Write a page", done: false)]
}

// How many notes are left to do.
pub fn left(notes: List<Note>) -> Int {
    return notes.filter(fn(n) { not n.done }).len()
}
"#;

const LAYOUT: &str = r#"{% props title: String %}
{% css "site.css" %}
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
</head>
<body>
{nav}<main>
{% slot %}
</main>
</body>
</html>
"#;

const NAV: &str = "<nav><a href=\"/\">Notes</a> <a href=\"/about\">About</a></nav>\n";

const CSS: &str = r#"/* The whole site's styles. */
:root {
  --accent: #c65a34;
  --ink: #1f232b;
  --paper: #f7f3ec;
}
body {
  margin: 0;
  font: 16px/1.5 system-ui, sans-serif;
  color: var(--ink);
  background: var(--paper);
}
nav, main {
  max-width: 40rem;
  margin: 0 auto;
  padding: 1rem;
}
a {
  color: var(--accent);
}
"#;

const HOME: &str = r#"{% props items: List<notes.Note> %}
{% layout "layout.glaze" title: "Notes" %}
<h1>Notes</h1>
<p>{notes.left(items)} left to do.</p>
<ul>
  {% for note in items %}
  <li>{note.title}{% if note.done %} (done){% end %}</li>
  {% end %}
</ul>
{ember}"#;

const EMBER_USE: &str = "<components.Counter start={0} />\n";

const EMBER: &str = r#"{# An Ember runs in the browser too: the page shows it at once, and its
   button works once its WebAssembly loads. #}
{% props start: Int %}
{% state count = start %}
<button type="button" on:click={count += 1}>Clicked {count} times</button>
"#;

const NOTES_TEST: &str = r#"import test

fn test_left_counts_notes_not_done() {
    let notes = [Note(title: "a", done: true), Note(title: "b", done: false)]
    test.equal(left(notes), 1)
}
"#;

const WEB_INTEGRATION: &str = r#"import test

fn test_home_lists_notes() {
    let res = test.get("/")
    test.equal(res.status, 200)
    test.contains(res.body, "Write a page")
}
"#;

const PAGES_MAIN: &str = r#"import web

// Each file in pages/ is a page: pages/index.glaze answers `GET /` and
// pages/about.glaze `GET /about`. `kiln run` serves them, and `kiln fire
// --static` writes them to dist/ for a static host.
fn main() ! {
    let app = web.app()
    app.file_routes("pages")
    // A health check only a running server answers.
    app.get("/health", fn(req) {
        return web.text("ok")
    }).dynamic()
    try app.listen(":8080")
}
"#;

const PAGES_ROUTE: &str = r#"// The data of the page beside it: `load` returns pages/index.glaze's props.
import notes

pub fn load() -> { items: List<notes.Note> } {
    return { items: notes.all() }
}
"#;

const PAGES_ABOUT: &str = r#"{% layout "pages/_components/layout.glaze" title: "About" %}
<h1>About</h1>
<p>A page is a file: add pages/contact.glaze and it answers /contact.</p>
"#;

const PAGES_INTEGRATION: &str = r#"import test

fn test_home_lists_notes() {
    let res = test.get("/")
    test.equal(res.status, 200)
    test.contains(res.body, "Write a page")
}

fn test_about_is_a_page() {
    test.contains(test.get("/about").body, "A page is a file")
}
"#;

const CLI_MAIN: &str = r#"import os, greet

// `kiln run . Ada` prints "Hello, Ada!".
fn main() {
    let args = os.args()
    let name = if args.len() > 0 { args[0] } else { "world" }
    print(greet.message(name))
}
"#;

const CLI_GREET: &str = r#"// What the program says.

pub fn message(name: String) -> String {
    return "Hello, ${name}!"
}
"#;

const CLI_TEST: &str = r#"import test

fn test_message_greets_by_name() {
    test.equal(message("Ada"), "Hello, Ada!")
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers questions from a list, and records them.
    struct Script(Vec<&'static str>, Vec<String>);

    impl Ask for Script {
        fn ask(&mut self, question: &str) -> Option<String> {
            self.1.push(question.to_string());
            (!self.0.is_empty()).then(|| self.0.remove(0).to_string())
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kiln-new-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn options(dir: &Path) -> Options {
        Options {
            dir: dir.to_path_buf(),
            template: None,
            name: None,
            ember: false,
            yes: false,
        }
    }

    #[test]
    fn names_come_from_the_folder() {
        assert_eq!(
            inferred_name(Path::new("My App")).as_deref(),
            Some("my-app")
        );
        assert_eq!(
            inferred_name(Path::new("notes_app")).as_deref(),
            Some("notes_app")
        );
        assert_eq!(
            inferred_name(Path::new("a/b/site.v2")).as_deref(),
            Some("site-v2")
        );
        assert_eq!(inferred_name(Path::new("日本")), None);
        assert!(inferred_name(Path::new(".")).is_some());
    }

    #[test]
    fn asks_for_the_kind_and_an_ember() {
        let dir = scratch("ask");
        let mut script = Script(vec!["2", "y"], Vec::new());
        let out = new_project(options(&dir), &mut script).unwrap();
        assert!(
            script.1[0].contains("What kind of project?"),
            "{:?}",
            script.1
        );
        assert!(
            out.contains("pages as files") && out.contains("with an Ember"),
            "{out}"
        );
        assert!(dir.join("pages/index.glaze").is_file());
        assert!(dir.join("components/counter.ember").is_file());
        let _ = std::fs::remove_dir_all(&dir);

        // With no terminal, the defaults.
        let mut silent = Script(Vec::new(), Vec::new());
        new_project(options(&dir), &mut silent).unwrap();
        assert!(dir.join("home.glaze").is_file() && !dir.join("components").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn never_overwrites_and_writes_nothing_on_a_conflict() {
        let dir = scratch("conflict");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("README.md"), "mine").unwrap();
        std::fs::write(dir.join("main.kiln"), "mine").unwrap();
        let mut o = options(&dir);
        o.template = Some(Template::Cli);
        o.yes = true;
        let err = new_project(o, &mut Script(Vec::new(), Vec::new())).unwrap_err();
        assert!(
            err.contains("`main.kiln`") && err.contains("nothing was written"),
            "{err}"
        );
        assert!(!dir.join("kiln.toml").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("main.kiln")).unwrap(),
            "mine"
        );

        // Without a conflict, a folder with files needs a yes.
        std::fs::remove_file(dir.join("main.kiln")).unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        let mut o = options(&dir);
        o.template = Some(Template::Cli);
        let err = new_project(o, &mut Script(Vec::new(), Vec::new())).unwrap_err();
        assert!(err.contains("--yes"), "{err}");
        let mut o = options(&dir);
        o.template = Some(Template::Cli);
        let out = new_project(o, &mut Script(vec!["y"], Vec::new())).unwrap();
        assert!(out.contains("kept your .gitignore"), "{out}");
        assert_eq!(
            std::fs::read_to_string(dir.join(".gitignore")).unwrap(),
            "target/\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("README.md")).unwrap(),
            "mine"
        );
        assert!(dir.join("greet/greet.kiln").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_programs_have_no_web_code() {
        let names: Vec<_> = files(Template::Cli, "x", false)
            .into_iter()
            .map(|(f, text)| {
                assert!(!text.contains("import web"), "{f}");
                f
            })
            .collect();
        assert!(!names
            .iter()
            .any(|f| f.ends_with(".glaze") || f.ends_with(".css")));
    }
}
