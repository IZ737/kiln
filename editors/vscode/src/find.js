// Finds the `kiln` executable. Kept free of the `vscode` module so tests
// can run it under plain Node.
const path = require("node:path");

/// Where to look for `kiln`, in order: the `kiln.path` setting, the PATH,
/// then `~/.kiln/bin`, where the installers put it. Returns the first
/// executable that exists, or `null`, and every place tried.
function findKiln({ configured, env, platform, home, exists }) {
  const paths = platform === "win32" ? path.win32 : path.posix;
  const names = platform === "win32" ? ["kiln.exe"] : ["kiln"];
  const tried = [];
  const check = (candidate) => {
    tried.push(candidate);
    return exists(candidate) ? candidate : null;
  };
  if (configured && configured.trim() !== "") {
    let file = configured.trim();
    if (file === "~" || file.startsWith("~/") || file.startsWith("~\\")) {
      file = paths.join(home, file.slice(1));
    }
    return { path: check(file), tried };
  }
  const dirs = (env.PATH || env.Path || "").split(paths.delimiter).filter((d) => d !== "");
  for (const dir of dirs) {
    for (const name of names) {
      const found = check(paths.join(dir, name));
      if (found) return { path: found, tried };
    }
  }
  for (const name of names) {
    const found = check(paths.join(home, ".kiln", "bin", name));
    if (found) return { path: found, tried };
  }
  return { path: null, tried };
}

module.exports = { findKiln };
