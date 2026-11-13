> **AI generated**: Pretty much this entire project was made using Claude Code.
> It's in that first stage of slopvelopment: buggy as shit, basically doesn't work,
> has about the usual couple hundred warnings and needs a LOT of testing and extra
> finagling to get up to a similar vein as emuera-tty.
>
> Also only tested on Linux and VSCode, no idea about Windows or macOS.

# EraBasic Language Server

A (not) production-ready Language Server Protocol (LSP) implementation for **EraBasic** (ERB/ERH files), supporting **Emuera-EM**, **Emuera-Anchor**, and **Emuera-EE** dialects.

Built on the battle-tested parser from [emuera-tty](https://github.com/astartes-gaming/emuera-tty), extended with full LSP capabilities via `tower-lsp`.

---

## Features

> Dev note: It says they work, but really, they don't. They absolutely do not.
> Trek at your own risk.

| Feature | Status | Notes |
|---------|--------|-------|
| **Diagnostics** | ✅ Done | String comparison warnings, malformed names, undefined functions, duplicate definitions |
| **Completion** | ✅ Done | Keywords, built-ins (200+), user functions/labels/variables/macros |
| **Hover** | ✅ Done | Signatures, types, EM-Reference documentation |
| **Signature Help** | ✅ Done | Parameter info for CALL/PRINT/built-ins |
| **Go to Definition** | ✅ Done | Functions, labels, variables, macros (cross-file) |
| **Find References** | ✅ Done | Workspace-wide symbol usage |
| **Document Symbols** | ✅ Done | Outline: functions, labels, globals, macros |
| **Semantic Tokens** | ✅ Done | Syntax highlighting (functions, variables, keywords, etc.) |
| **Code Actions** | ✅ Done | Quick fixes for string comparisons, missing ENDIF, etc. |
| **Folding Ranges** | ✅ Done | IF/ENDIF, FOR/NEXT, SELECTCASE, functions |
| **Inlay Hints** | ✅ Done | Parameter names, inferred types |
| **Workspace Symbols** | ✅ Done | Cross-file symbol search |
| **Multi-root Workspace** | ✅ Done | Multiple Era game folders |
| **File Watching** | ✅ Done | Auto-reparse on file changes |
| **Dialect Detection** | ✅ Done | Auto/EM/Anchor/EE per-file |
| **Encoding Support** | ✅ Done | UTF-8 (with/without BOM), Shift_JIS |

### In Progress / Missing

| Feature | Status | Notes |
|---------|--------|-------|
| `#INCLUDE` / `#INCLUDES` parsing | 🚧 Planned | Critical for full Era games; need dependency graph |
| CSV integration (character data) | 🚧 Planned | Parse CSV for hover/completion of character refs |
| Load-order emulation | 🚧 Planned | NTFS/ICU/NLS order affects duplicate resolution |
| Large project optimization | 🔄 Ongoing | Lazy loading, background indexing for 1000+ files |
| Rename refactoring | ❌ Not started | Requires full reference resolution |
| Document formatting | ❌ Not started | EraBasic has no standard formatter |

---

## Quick Start

### Prerequisites

- **Rust** 1.75+ (`rustup install stable`)
- **Node.js** 18+ (for VS Code extension)
- **VS Code** 1.80+ (or Neovim/Helix with LSP client)

---

## Building the LSP Server

```bash
cd lsp
cargo build --release
```

The binary will be at `lsp/target/release/erabasic-lsp` (~5.4 MB).

### Running Manually (for testing)

```bash
# Test initialization
printf 'Content-Length: 75\r\n\r\n{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":1,"rootUri":null,"capabilities":{}}}' | ./target/release/erabasic-lsp
```

### Configuration (via LSP client settings)

```json
{
  "erabasic-lsp.dialect": "auto",        // "auto" | "emuera-em" | "emuera-anchor" | "emuera-ee"
  "erabasic-lsp.lint.stringComparison": true,
  "erabasic-lsp.lint.malformedNames": true,
  "erabasic-lsp.lint.unclosedFormHoles": true
}
```

---

## Building the VS Code Extension

```bash
cd extension
npm install
npm run compile
```

### Package with Bundled Server (for distribution)

```bash
# Build LSP first
cd ../lsp && cargo build --release

# Bundle server into extension
cd ../extension
mkdir -p server
cp ../lsp/target/release/erabasic-lsp server/

# Package .vsix
npm run package
```

Output: `extension/erabasic-lsp-0.1.0.vsix` (~7 MB)

### Install Locally

```bash
code --install-extension erabasic-lsp-0.1.0.vsix
```

### Development Mode (without bundling)

The extension auto-detects the server from these locations (in order):
1. Configured `erabasic-lsp.serverPath` setting
2. `extension/server/erabasic-lsp` (bundled)
3. `./target/release/erabasic-lsp` (workspace root)
4. `./erabasic-rust-lsp/target/release/erabasic-lsp`
5. `~/.cargo/bin/erabasic-lsp` (cargo install)
6. `erabasic-lsp` in PATH

For development, just run `cargo build --release` in `lsp/` and the extension will pick it up.

---

## Architecture

```
erabasic-full-lsp-git/
├── lsp/                          # Rust LSP server (tower-lsp 0.20)
│   ├── src/
│   │   ├── main.rs               # Entry point, stdio transport
│   │   ├── server.rs             # Capabilities, request handlers
│   │   ├── document.rs           # Rope-based editing, incremental parse
│   │   ├── workspace.rs          # Multi-root, file watching, discovery
│   │   ├── lsp_ast.rs            # Owned AST with Span (byte offsets)
│   │   ├── lsp_parser.rs         # Eager parser with span tracking
│   │   ├── parser/               # Adapted from emuera-tty
│   │   │   ├── ast.rs, expr.rs, parser.rs, loader.rs, lint.rs, builtins.rs
│   │   ├── analysis/             # Semantic layer
│   │   │   ├── symbols.rs, types.rs, references.rs, variant.rs, index.rs
│   │   ├── providers/            # LSP feature implementations
│   │   │   ├── completion.rs, hover.rs, diagnostics.rs, ...
│   │   ├── docs/                 # EM-Reference markdown parser
│   │   └── util/                 # Position conversion, encoding
│   └── Cargo.toml
└── extension/                    # VS Code extension
    ├── src/extension.ts          # vscode-languageclient wrapper
    ├── syntaxes/erbasic.tmLanguage.json
    ├── snippets/erbasic.json
    └── package.json
```

### Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Owned AST (`lsp_ast.rs`)** | LSP requires `'static` lifetime; parser AST borrows source text |
| **Eager parser (`lsp_parser.rs`)** | LSP needs full AST immediately; original parser is lazy |
| **Ropey for text** | Efficient incremental editing for large files |
| **Per-file dialect detection** | Games mix EM/Anchor files; config overrides available |
| **Workspace index** | Cross-file symbol resolution built on file load/change |

---

## Testing

```bash
# Run all tests
cd lsp && cargo test

# Specific test suites
cargo test parser::           # Parser unit tests
cargo test providers::        # LSP provider tests
cargo test document::         # Document management tests
```

---

## For Era Game Developers

### Opening a Game Project

1. Open the game root folder in VS Code (File → Open Folder)
2. The extension activates on `.erb`/`.erh` files
3. All ERB/ERH files in the workspace are indexed automatically

### Dialect Configuration

For pure Emuera-EM games, set in workspace settings (`.vscode/settings.json`):

```json
{
  "erabasic-lsp.dialect": "emuera-em",
  "erabasic-lsp.subdirs": true
}
```

For Emuera-Anchor games:

```json
{
  "erabasic-lsp.dialect": "emuera-anchor"
}
```

`auto` (default) detects per-file via heuristics (`#FUNCTIONS`, `PARAM`, `TR_`, etc.)

### Known Limitations with Full Games

| Issue | Workaround |
|-------|------------|
| `#INCLUDE` not resolved | Functions/vars in included files won't appear in completion |
| Duplicate functions | Last-loaded wins (matches Emuera NTFS order by default) |
| CSV references | Character names from CSV not in hover/completion yet |

---

## Contributing

1. Fork the repository
2. Create a feature branch
3. Run `cargo fmt && cargo clippy` before committing
4. Add tests for new features
5. Submit PR

### Code Style

- `cargo fmt` for formatting
- `cargo clippy -D warnings` for lints
- Tests in `#[cfg(test)]` modules alongside code

---

## License

MIT License - see [LICENSE](extension/LICENSE.txt)

---

## Source References

This LSP was built by adapting and extending code from the following projects:

> Emuera-Anchor and emuEra are the same thing.

| Project | Purpose |
|---------|---------|
| **emuera-tty** | Core parser (expr.rs, parser.rs, ast.rs, loader.rs, lint.rs, builtins.rs) — battle-tested Rust port of Emuera |
| **emuera.em** | Original Emuera-EM (C#) — reference for dialect behavior, built-in functions, control flow |
| **Emuera-Anchor** | Anchor variant (C#) — reference for `#FUNCTIONS`, `PARAM`, `TR_` functions, `#SINGLE`/`#ONLY`/`#KOJO` |
| **emuEra** | Another C# implementation — reference for EE dialect (`VARI`/`VARS`), edge cases |
| **EM-Reference** | Markdown documentation for 200+ built-in functions/commands as found in AnonTW — parsed for hover docs |

---

## Acknowledgments

- **emuera-tty** - The Rust parser this LSP is built on
- **tower-lsp** - Excellent LSP framework for Rust
- **EM-Reference** - Documentation source for built-in functions
- **Emuera-EM / Emuera-Anchor / Emuera-EE** - The interpreters this supports
