// Loads the extension's grammars with the same TextMate engine VS Code uses.
const fs = require("node:fs");
const path = require("node:path");
const oniguruma = require("vscode-oniguruma");
const textmate = require("vscode-textmate");

const root = path.join(__dirname, "..");
const grammars = {
  "source.kiln": "syntaxes/kiln.tmLanguage.json",
  "text.html.glaze": "syntaxes/glaze.tmLanguage.json",
  "text.html.ember": "syntaxes/ember.tmLanguage.json",
  "source.toml.kiln": "syntaxes/toml.tmLanguage.json",
};

const wasm = fs.readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm"));
const registry = new textmate.Registry({
  onigLib: oniguruma.loadWASM(wasm.buffer).then(() => ({
    createOnigScanner: (patterns) => new oniguruma.OnigScanner(patterns),
    createOnigString: (s) => new oniguruma.OnigString(s),
  })),
  loadGrammar: async (scope) => {
    // CSS and JavaScript come with VS Code; here they are empty stand-ins.
    if (scope === "source.css" || scope === "source.js") {
      return textmate.parseRawGrammar(JSON.stringify({ scopeName: scope, patterns: [] }), "x.json");
    }
    const file = grammars[scope];
    if (!file) return null;
    const text = fs.readFileSync(path.join(root, file), "utf8");
    return textmate.parseRawGrammar(text, path.join(root, file));
  },
});

/// Tokenizes `text`, returning each token's text and scopes, line by line,
/// and the rule stack left after the last line.
async function tokenize(scope, text) {
  const grammar = await registry.loadGrammar(scope);
  let stack = textmate.INITIAL;
  const lines = [];
  for (const line of text.split("\n")) {
    const result = grammar.tokenizeLine(line, stack);
    lines.push(
      result.tokens.map((t) => ({
        text: line.slice(t.startIndex, t.endIndex),
        scopes: t.scopes,
      })),
    );
    stack = result.ruleStack;
  }
  return { lines, stack };
}

module.exports = { tokenize, root };
