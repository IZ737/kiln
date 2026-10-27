//! `kiln add npm:...` against a registry on this machine, and builds of
//! embers that call the packages it adds.

use base64::Engine;
use sha2::{Digest, Sha512};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MANIFEST: &str = r#"{
  "name": "kiln-demo",
  "version": "1.0.0",
  "type": "module",
  "exports": { ".": { "node": "./dist/node.js", "browser": "./dist/index.js", "default": "./dist/node.js" } },
  "types": "./dist/index.d.ts"
}"#;

const ENTRY: &str = r#"import { twice } from "./util.js";
export function shout(text) { return text.toUpperCase() + "!"; }
export function double(n) { return twice(n); }
export async function later(ms) {
  await new Promise((done) => setTimeout(done, ms ?? 10));
  return "done";
}
export const version = "1.0.0";
export function fail() { throw new Error("nope"); }
export function each(f) { f(); }
"#;

const DECLARATIONS: &str = r#"/** Shouts. */
export declare function shout(text: string): string;
export declare function double(n: number): number;
export declare function later(ms?: number): Promise<string>;
export declare const version: string;
export declare function fail(): void;
export declare function each(f: () => void): void;
"#;

/// A package tarball with the given files, inside `package/`.
fn tarball(files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    for (path, body) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, format!("package/{path}"), body.as_bytes())
            .unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

/// The demo package, and one more file when `extra` is given.
fn demo(extra: Option<(&str, &str)>) -> Vec<u8> {
    let mut files = vec![
        ("package.json", MANIFEST),
        ("dist/index.js", ENTRY),
        ("dist/util.js", "export const twice = (n) => n * 2;\n"),
        ("dist/node.js", "import fs from 'node:fs';\n"),
        ("dist/index.d.ts", DECLARATIONS),
        ("README.md", "# kiln-demo\n"),
        ("LICENSE", "MIT\n"),
    ];
    files.extend(extra);
    tarball(&files)
}

/// Serves npm's registry API for one package, `kiln-demo@1.0.0`, with the
/// given tarball and integrity, and returns the registry's URL.
fn serve(tarball: Vec<u8>, integrity: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let meta = serde_json::json!({
        "name": "kiln-demo",
        "dist-tags": { "latest": "1.0.0" },
        "versions": { "1.0.0": { "dist": {
            "integrity": integrity,
            "tarball": format!("{base}/kiln-demo/-/kiln-demo-1.0.0.tgz"),
        } } },
    })
    .to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut line).unwrap();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap() <= 2 {
                    break;
                }
            }
            let path = line.split_whitespace().nth(1).unwrap_or("");
            let (status, body): (&str, &[u8]) = match path {
                "/kiln-demo" => ("200 OK", meta.as_bytes()),
                "/kiln-demo/-/kiln-demo-1.0.0.tgz" => ("200 OK", &tarball),
                _ => ("404 Not Found", b""),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    base
}

fn integrity(tarball: &[u8]) -> String {
    let hash = base64::engine::general_purpose::STANDARD.encode(Sha512::digest(tarball));
    format!("sha512-{hash}")
}

/// An empty program folder.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-npm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn kiln(args: &[&str], registry: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(args)
        .env("KILN_NPM_REGISTRY", registry)
        .env("KILN_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap()
}

fn add(dir: &Path, registry: &str) -> Output {
    kiln(
        &["add", "npm:kiln-demo@1.0.0", dir.to_str().unwrap()],
        registry,
    )
}

fn files(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

const PROGRAM: &str = r#"import web

fn main() ! {
    let app = web.app()
    app.get("/", fn(req: web.Request) -> web.Response { web.render("index.glaze", { title: "npm" }) })
    try app.listen("127.0.0.1:0")
}
"#;

const INDEX: &str = r#"{% props title: String %}
<!doctype html>
<html><head><title>{title}</title></head>
<body><Demo /></body></html>
"#;

const DEMO: &str = r#"{% state said = "" %}
<div>
    <button on:click={
        let a = kiln_demo.shout("hi") catch err { err.message }
        let n = kiln_demo.double(2.0) catch err { 0.0 }
        let v = kiln_demo.version() catch err { "?" }
        said = kiln_demo.later(5.0) catch err { err.message }
        said = "${a} ${n} ${v} ${said}"
    }>Go</button>
    <span>{said}</span>
</div>
"#;

#[test]
fn add_keeps_the_browser_module_and_records_it() {
    let tarball = demo(None);
    let registry = serve(tarball.clone(), integrity(&tarball));
    let dir = folder("add");
    let out = add(&dir, &registry);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.starts_with("added kiln-demo 1.0.0 as `kiln_demo`: "),
        "{stdout}"
    );
    for name in ["shout", "double", "later", "version", "fail"] {
        assert!(stdout.contains(name), "{name} missing from: {stdout}");
    }
    assert!(stdout.contains("left out `each`"), "{stdout}");
    // Only what the browser entry imports, the declarations, the manifest
    // and the licence are kept: not the Node.js module or the README.
    assert_eq!(
        files(&dir),
        [
            "npm.lock",
            "npm/kiln-demo/LICENSE",
            "npm/kiln-demo/dist/index.d.ts",
            "npm/kiln-demo/dist/index.js",
            "npm/kiln-demo/dist/util.js",
            "npm/kiln-demo/package.json",
        ]
    );
    let lock: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("npm.lock")).unwrap()).unwrap();
    let entry = &lock["kiln_demo"];
    assert_eq!(entry["version"], "1.0.0");
    assert_eq!(entry["entry"], "dist/index.js");
    assert_eq!(entry["integrity"], integrity(&tarball));
    let hashes: BTreeMap<String, String> = serde_json::from_value(entry["files"].clone()).unwrap();
    assert!(hashes.values().all(|h| h.starts_with("sha256-")));

    // A program's embers call it, and the build serves its modules.
    std::fs::write(dir.join("main.kiln"), PROGRAM).unwrap();
    std::fs::write(dir.join("index.glaze"), INDEX).unwrap();
    std::fs::write(dir.join("demo.ember"), DEMO).unwrap();
    let exe = dir.join("app");
    let out = kiln(
        &["build", dir.to_str().unwrap(), "-o", exe.to_str().unwrap()],
        &registry,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Outside an ember the package is not there.
    std::fs::write(
        dir.join("main.kiln"),
        PROGRAM.replace(
            "fn main() ! {",
            "fn main() ! {\n    let x = kiln_demo.shout(\"hi\") catch err { \"\" }",
        ),
    )
    .unwrap();
    let out = kiln(&["check", dir.to_str().unwrap()], &registry);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("E0223") || stderr.contains("E0201"),
        "{stderr}"
    );

    // A changed file is refused.
    std::fs::write(dir.join("main.kiln"), PROGRAM).unwrap();
    std::fs::write(
        dir.join("npm/kiln-demo/dist/util.js"),
        "export const twice = 0;\n",
    )
    .unwrap();
    let out = kiln(
        &["build", dir.to_str().unwrap(), "-o", exe.to_str().unwrap()],
        &registry,
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("is not the file `kiln add` downloaded"),
        "{stderr}"
    );
}

#[test]
fn add_refuses_a_tarball_that_does_not_match() {
    let tarball = demo(None);
    let registry = serve(tarball, integrity(b"something else"));
    let dir = folder("integrity");
    let out = add(&dir, &registry);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(stderr.contains("does not match the integrity"), "{stderr}");
    assert!(files(&dir).is_empty(), "{:?}", files(&dir));
}

#[test]
fn add_refuses_packages_it_cannot_load() {
    // The browser module imports another package.
    let tarball = demo(Some((
        "dist/util.js",
        "import x from \"left-pad\";\nexport const twice = (n) => n * 2;\n",
    )));
    let registry = serve(tarball.clone(), integrity(&tarball));
    let dir = folder("bare");
    let out = add(&dir, &registry);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("imports the package `left-pad`"),
        "{stderr}"
    );

    let out = kiln(&["add", "nanoid", dir.to_str().unwrap()], &registry);
    assert!(String::from_utf8_lossy(&out.stderr).contains("`npm:<package>@<version>`"));
    let out = kiln(
        &["add", "npm:../evil@1.0.0", dir.to_str().unwrap()],
        &registry,
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("is not an npm package name"));
    let out = kiln(
        &["add", "npm:kiln-demo@2.0.0", dir.to_str().unwrap()],
        &registry,
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("has no version 2.0.0"));
}
