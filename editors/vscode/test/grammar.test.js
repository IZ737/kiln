const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const textmate = require("vscode-textmate");
const { tokenize, root } = require("./textmate");

/// Returns the innermost scope of the first token on `line` whose text is
/// `text`, after skipping `skip` earlier matches.
function scopeOf(lines, line, text, skip = 0) {
  const tokens = lines[line].filter((t) => t.text === text);
  assert.ok(tokens.length > skip, `no token ${JSON.stringify(text)} on line ${line}`);
  return tokens[skip].scopes;
}

function assertScope(lines, line, text, scope, skip = 0) {
  const scopes = scopeOf(lines, line, text, skip);
  assert.ok(
    scopes.some((s) => s === scope),
    `${JSON.stringify(text)} on line ${line} has ${scopes.join(" ")}, not ${scope}`,
  );
}

test("Kiln declarations, keywords and literals", async () => {
  const { lines } = await tokenize(
    "source.kiln",
    [
      "import web, json",
      "pub struct Post { id: Int, title: String? }",
      "server fn like(id: Int) -> Int ! { return 0 }",
      'let msg = "Hi ${user.name}\\n" // greet',
      "var wait = 250ms + 1.5e3",
      "const PAGE_SIZE: Int = 20",
      "if a and not b { posts.map(.title) } else { none }",
      "let p = try json.decode<Post>(text) catch err { throw err }",
      "/* block */ x ?? 1..10",
    ].join("\n"),
  );
  assertScope(lines, 0, "import", "keyword.control.import.kiln");
  assertScope(lines, 1, "pub", "storage.modifier.kiln");
  assertScope(lines, 1, "struct", "storage.type.struct.kiln");
  assertScope(lines, 1, "Post", "entity.name.type.kiln");
  assertScope(lines, 1, "Int", "support.type.primitive.kiln");
  assertScope(lines, 1, "?", "keyword.operator.optional.kiln");
  assertScope(lines, 2, "server", "storage.modifier.kiln");
  assertScope(lines, 2, "like", "entity.name.function.kiln");
  assertScope(lines, 2, "->", "keyword.operator.arrow.kiln");
  assertScope(lines, 2, "!", "keyword.operator.error.kiln");
  assertScope(lines, 2, "return", "keyword.control.kiln");
  assertScope(lines, 3, "let", "storage.type.let.kiln");
  assertScope(lines, 3, "${", "punctuation.section.interpolation.begin.kiln");
  assertScope(lines, 3, "name", "variable.other.property.kiln");
  assertScope(lines, 3, "name", "meta.interpolation.kiln");
  assertScope(lines, 3, "\\n", "constant.character.escape.kiln");
  assertScope(lines, 3, " greet", "comment.line.double-slash.kiln");
  assertScope(lines, 4, "var", "storage.type.var.kiln");
  assertScope(lines, 4, "250ms", "constant.numeric.duration.kiln");
  assertScope(lines, 4, "1.5e3", "constant.numeric.float.kiln");
  assertScope(lines, 5, "PAGE_SIZE", "variable.other.constant.kiln");
  assertScope(lines, 5, "20", "constant.numeric.integer.kiln");
  assertScope(lines, 6, "and", "keyword.operator.word.kiln");
  assertScope(lines, 6, "not", "keyword.operator.word.kiln");
  assertScope(lines, 6, "map", "entity.name.function.kiln");
  assertScope(lines, 6, "title", "variable.other.property.kiln");
  assertScope(lines, 6, "none", "constant.language.none.kiln");
  assertScope(lines, 7, "decode", "entity.name.function.kiln");
  assertScope(lines, 7, "<", "punctuation.definition.generic.begin.kiln");
  assertScope(lines, 7, "catch", "keyword.control.kiln");
  assertScope(lines, 7, "throw", "keyword.control.kiln");
  assertScope(lines, 8, " block ", "comment.block.kiln");
  assertScope(lines, 8, "??", "keyword.operator.coalesce.kiln");
  assertScope(lines, 8, "..", "keyword.operator.range.kiln");
});

test("an unclosed Kiln string stops at the end of its line", async () => {
  const { lines } = await tokenize("source.kiln", 'let s = "open\nlet t = 1');
  assertScope(lines, 1, "let", "storage.type.let.kiln");
});

test("Glaze tags, output, components and attributes", async () => {
  const { lines } = await tokenize(
    "text.html.glaze",
    [
      '{% props post: Post, user: User? = none %}',
      "{# a comment #}",
      '<a href="/posts/{post.id}" class="x">{post.title}</a>',
      "{raw post.html}",
      '{% for tag in post.tags where tag.visible %}<Tag name={tag.name} />{% empty %}{% end %}',
      "<button on:click={ likes += 1 } client:visible>&hearts;</button>",
      '{% if user is some(u) %}{"{"}{% else if x %}{% end %}',
      "<style>.a { color: red; }</style>",
      "<p>{ { a: 1 }.a }</p>",
      '<script type="module">let a = {b: 1}</script>',
    ].join("\n"),
  );
  assertScope(lines, 0, "{%", "punctuation.definition.tag.begin.glaze");
  assertScope(lines, 0, "props", "keyword.control.glaze");
  assertScope(lines, 0, "Post", "entity.name.type.kiln");
  assertScope(lines, 0, "none", "constant.language.none.kiln");
  assertScope(lines, 0, "%}", "punctuation.definition.tag.end.glaze");
  assertScope(lines, 1, " a comment ", "comment.block.glaze");
  assertScope(lines, 2, "a", "entity.name.tag.html");
  assertScope(lines, 2, "href", "entity.other.attribute-name.html");
  assertScope(lines, 2, "id", "source.kiln");
  assertScope(lines, 2, "id", "string.quoted.double.html");
  assertScope(lines, 2, "title", "variable.other.property.kiln");
  assertScope(lines, 3, "raw", "keyword.other.raw.glaze");
  assertScope(lines, 3, "html", "variable.other.property.kiln");
  assertScope(lines, 4, "for", "keyword.control.glaze");
  assertScope(lines, 4, "in", "keyword.control.kiln");
  assertScope(lines, 4, "where", "keyword.control.kiln");
  assertScope(lines, 4, "Tag", "support.class.component.glaze");
  assertScope(lines, 4, "name", "entity.other.attribute-name.html");
  assertScope(lines, 4, "empty", "keyword.control.glaze");
  assertScope(lines, 4, "end", "keyword.control.glaze");
  assertScope(lines, 5, "on", "entity.other.attribute-name.event.glaze");
  assertScope(lines, 5, "click", "entity.other.attribute-name.event.glaze");
  assertScope(lines, 5, "+=", "keyword.operator.assignment.compound.kiln");
  assertScope(lines, 5, "client:visible", "entity.other.attribute-name.html");
  assertScope(lines, 5, "&hearts;", "constant.character.entity.html");
  assertScope(lines, 6, "is", "keyword.operator.word.kiln");
  assertScope(lines, 6, "some", "entity.name.function.kiln");
  assertScope(lines, 6, "{", "string.quoted.double.kiln", 1);
  assertScope(lines, 6, "else", "keyword.control.glaze");
  assertScope(lines, 6, "if", "keyword.control.kiln", 1);
  assertScope(lines, 7, ".a { color: red; }", "source.css");
  // A record's braces nest inside an output's.
  assertScope(lines, 8, "a", "variable.other.property.kiln");
  assertScope(lines, 8, "p", "entity.name.tag.html", 1);
  assertScope(lines, 9, "type", "entity.other.attribute-name.html");
  assertScope(lines, 9, "let a = {b: 1}", "source.js");
});

/// Every example program must tokenize back to where it started, so no rule
/// is left open at the end of the file.
test("every example tokenizes with no rule left open", async () => {
  const examples = path.join(root, "..", "..", "examples");
  const files = fs
    .readdirSync(examples, { recursive: true })
    .filter((f) => f.endsWith(".kiln") || f.endsWith(".glaze"));
  assert.ok(files.length > 0);
  for (const file of files) {
    const scope = file.endsWith(".kiln") ? "source.kiln" : "text.html.glaze";
    const text = fs.readFileSync(path.join(examples, file), "utf8");
    const { stack } = await tokenize(scope, text);
    assert.strictEqual(stack.depth, textmate.INITIAL.depth, `${file} leaves a rule open`);
  }
});
