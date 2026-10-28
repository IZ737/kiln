# Kiln — Complete logo and vocabulary icon pack

This consolidates the selected **A · Modern Grotesk** horizontal logo, the unchanged **v7 geometric kiln symbol**, and all five vocabulary icons into one portable package.

All artwork is editable SVG geometry; no raster images or installed fonts are needed to render the SVGs.

## Structure

- `logo/kiln-horizontal.svg` — selected primary logo (transparent)
- `logo/kiln-horizontal-dark.svg` — logo for dark UI (dark background included)
- `logo/kiln-horizontal-mono.svg` — monochrome wordmark and symbol
- `logo/kiln-symbol.svg` — original kiln mark only (transparent)
- `logo/kiln-symbol-mono.svg`, `logo/kiln-symbol-white.svg` — alternative mark colours
- `logo/png/` — 1200px lockups and 16, 32, 64, 128, 256, 512px symbol exports
- `icons/svg/` — seven vocabulary icons, each with standard, `-dark`, and `-mono` SVG
- `icons/png/{16,32,64,128}/` — PNG versions of five icons
- `vscode/file-icon-theme.json` — sample icon theme definition using relative paths
- `preview/brand-and-vocabulary.png` — single combined proof sheet

## Vocabulary mapping

| Icon | Kiln vocabulary | Meaning |
|---|---|---|
| `kiln-source.svg` | `.kiln` | Source file |
| `glaze-component.svg` | `.glaze` / Glaze | Template or component |
| `ware-package.svg` | Ware | Package |
| `ember-wasm.svg` | Ember | Interactive component compiled to WebAssembly |
| `fire-release.svg` | `kiln fire` | Release build |
| `kiln-manifest.svg` | `kiln.toml` | Project manifest |
| `kiln-lock.svg` | `kiln.lock` | Pinned Wares |

## Brand colours

- Fired terracotta: `#C65A34`
- Charcoal: `#1F232B`
- Warm off-white: `#F7F3EC`

## VS Code

You can move `vscode/file-icon-theme.json` into an extension or use its mappings as a reference. The paths in the example assume you preserve the folder layout shown above. To publish a VS Code icon theme you must register it in your extension's `package.json` with a `contributes.iconThemes` entry.

The primary wordmark is **option A** exactly as selected; it has not been redesigned for this bundle.
