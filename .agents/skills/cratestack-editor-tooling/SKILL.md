---
name: cratestack-editor-tooling
description: Editor support for .cstack files — the cratestack-lsp language server and its exact capabilities, the CrateStack Schema VS Code extension, and the tree-sitter-cstack grammar for Neovim, Helix and Zed. Load when setting up .cstack syntax highlighting, diagnostics or go-to-definition in any editor, or when asking why VS Code does not use the tree-sitter grammar.
---

# Editor tooling

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

Two independent pieces, for different editors, with a deliberate split.

## `cratestack-lsp`

A standalone binary speaking **LSP over stdio**, no arguments and no config file.
Install with `cargo install cratestack-lsp`.

Advertised capabilities — this list is the authority:

| | |
| --- | --- |
| Text sync | **Full** (not incremental) |
| Hover | yes |
| Completion | yes |
| Go to definition | yes |
| **Find references** | yes |
| **Document highlight** | yes |
| Document symbols | yes |
| **Rename** | yes, with prepare-rename |
| Semantic tokens | yes, full document, **no token modifiers** |
| Code actions, formatting, signature help, folding, inlay hints, workspace symbols | **no** |

`cratestack-lsp`'s own README says find-references, rename and code actions are
all unimplemented. **That is wrong for two of the three** — only code actions are
genuinely absent. The README also pins an install version of `0.7`, several
releases behind.

Diagnostics are published on `didOpen` and `didChange`.

**Last-known-good behaviour:** while a file has a syntax error the server keeps
serving the last version that parsed, so navigation, symbols and colouring stay
put. The live error is still reported, and hover says the schema is stale. A file
that has never parsed stays quiet.

Semantic token legend, nine types: `type`, `struct`, `enum`, `interface`,
`enumMember`, `property`, `function`, `parameter`, `decorator`. Models map to
`struct`, mixins to `interface`, builtin scalars to `type`, fields and
`@relation` columns to `property`, procedures to `function`, procedure arguments
to `parameter`. Only an attribute's `@name` head is a `decorator`.

Go-to-definition resolves field types to their `model` / `type` / `enum` /
`mixin`, procedure argument and return types **including inside `Page<T>`**,
`@use(Mixin)`, and both halves of `@relation(fields: […], references: […])`.
Find-references works from either end of a relation and qualifies fields by
owning declaration, so `User.id` and `Post.id` are distinct symbols.

Rename refuses rather than guesses: builtins are unrenameable, the new name must
be a non-keyword non-builtin identifier, collisions are rejected, and it refuses
entirely while the file has a syntax error.

### Pointing an editor at it

**Neovim:**

```lua
require('lspconfig.configs').cratestack = {
  default_config = {
    cmd = { 'cratestack-lsp' },
    filetypes = { 'cstack' },
    root_dir = require('lspconfig.util').root_pattern('.git', '*.cstack'),
  },
}
require('lspconfig').cratestack.setup{}
```

**Emacs (lsp-mode):** `(lsp-stdio-connection '("cratestack-lsp"))` with
`:major-modes '(cstack-mode)`, `:server-id 'cratestack`, plus an
`lsp-language-id-configuration` entry mapping `cstack-mode → "cstack"`.

**Anything else:** spawn `cratestack-lsp` on stdio with language id `cstack` for
`*.cstack`.

## VS Code

Extension id **`cratestack.cratestack-vscode-plugin`**, display name
**CrateStack Schema**.

```bash
code --install-extension cratestack.cratestack-vscode-plugin
codium --install-extension cratestack.cratestack-vscode-plugin   # Open VSX
```

It registers the `cstack` language and a TextMate grammar
(`scopeName: source.cstack`), and **launches the standalone `cratestack-lsp`
binary** — it contains no language logic of its own.

Server resolution order: an explicitly configured `cratestack.lsp.path` that
differs from the default wins; otherwise a staged binary at
`<extensionPath>/server/<platform>/cratestack-lsp`; otherwise `cratestack-lsp`
on `PATH`. Settings: `cratestack.lsp.path` (default `"cratestack-lsp"`) and
`cratestack.lsp.args` (default `[]`).

Published to both the VS Code Marketplace and Open VSX, so VSCodium, Cursor and
Windsurf work. An air-gapped `.vsix` from a GitHub Release also works but **does
not auto-update**.

## `tree-sitter-cstack`

A separate repo, `cratestack/tree-sitter-cstack`, published to crates.io. It
exists for **Neovim, Helix and Zed** — editors that consume tree-sitter grammars
by repository URL.

**VS Code deliberately does not use it.** VS Code has no tree-sitter API for
third-party languages (microsoft/vscode#50140, open since 2018), and the
workaround — compile to wasm and drive the Semantic Token API from the extension
— buys nothing, because `cratestack-lsp` already has a real parse in-process and
emits semantic tokens directly.

**It is not the authoritative parser and is deliberately more permissive.**
`cratestack-parser` decides what a `.cstack` file *means*; the grammar describes
only its shape. Unknown attributes, unknown mixins, invalid provider values and
duplicate names all parse fine — those are semantic errors, and an editor that
stops highlighting because a relation target does not exist yet is an editor that
stops highlighting constantly.

Its anti-drift guard is `test/conformance.sh`, which parses **every `.cstack`
file the framework repo ships** and fails on any unexpected `ERROR` node. It runs
against `cratestack@main` on every push **and weekly**, so upstream syntax
changes surface even when nothing lands in the grammar repo. Exactly one expected
failure is listed by explicit path — a fixture with an unbalanced paren.
`semantic_error_*` fixtures must **parse**; rejecting them would mean the grammar
had started doing semantic analysis.

Neovim setup: register the parser with
`nvim-treesitter.parsers.get_parser_configs()`, add
`vim.filetype.add({ extension = { cstack = "cstack" } })`, `:TSInstall cstack`,
and copy `queries/highlights.scm` into `queries/cstack/`.

Rust consumers get `tree_sitter_cstack::LANGUAGE`, `HIGHLIGHTS_QUERY` and
`TAGS_QUERY`. Generated `src/parser.c` is committed so consumers do not need the
CLI.

**`.cstack` has no call sites, permanently.** It is declarative — a `procedure`
is declared in the schema and implemented in Rust — so `tags.scm` emits no
`@reference.call` and never should. A tool reporting 0% call resolution for these
files is misreporting "there was nothing to try".
