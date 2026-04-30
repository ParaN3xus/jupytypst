# Project Context

`jupytypst` is a Typst Jupyter kernel written in Rust. The workspace also
contains `typsess`, a reusable library crate for stateful Typst code-mode
evaluation and rendering.

## Goals

- Provide a `clap` CLI with `start`, `install`, and `repl` commands.
- Install a Jupyter kernelspec that launches this binary.
- Provide a pure terminal REPL that uses the same Typst session engine without
  serving as a Jupyter kernel.
- Maintain useful Typst context across cells without re-rendering previous
  visible content.
- Support two execution modes:
  - `svg`: render Typst markup as notebook HTML containing per-page SVG.
  - `html`: render Typst markup as `text/html`.
- Notebook cells may switch render mode with `// jupytypst: mode=svg|html`.
  This parsing is a `jupytypst` host concern; `typsess` receives plain Typst
  code and a render mode.
- Execute cells as Typst code mode through `typst_eval::Vm`. Users write
  `let`, `set`, and function calls directly without a leading `#`.
- Default page setup is `set page(width: auto, height: auto, margin: 16pt)`.
  CLI users can pass `--page-setup none` or custom Typst page setup code.
- Automatic page setup is evaluated once at session initialization and stored
  as the initial persistent styles. User `set page(...)` rules persist only for
  non-sizing fields; `paper`, `width`, and `height` are filtered so the next
  rendered cell returns to the configured page sizing.

## Design Notes

- Use `jupyter-protocol` for Jupyter message structures and MIME bundles.
- Use `zeromq` for the kernel sockets.
- Keep workspace dependency versions in the root `Cargo.toml`; member crates use
  `.workspace = true` for shared dependencies.
- `crates/typsess` owns the persistent Typst session, page setup, input
  completeness classification, and SVG/HTML rendering.
- `crates/jupytypst` owns Jupyter protocol handling, kernelspec installation,
  notebook directives, and the user-facing terminal REPL.
- `typsess` returns structured Typst outputs (`PagedDocument` or
  `HtmlDocument`) plus `SourceDiagnostic`s. `jupytypst` converts those outputs
  into notebook/terminal HTML and user-facing error strings.
- Use `tinymist-world` for Typst's `World` implementation so package imports,
  filesystem resolution, font discovery, and package cache behavior match
  Tinymist's system environment.
- Use lower-level Typst APIs for evaluation and rendering:
  - Keep a persistent top-level `Scope` for definitions/imports.
  - Capture top-level `Styles` from `set` and selector `show` rules.
  - Carry forward invisible `state`/`counter` update content so Typst's
    per-layout `Introspector` can see prior cell updates.
  - Render the current cell's evaluated `Content`, not accumulated source.
- Tinymist DAP REPL is useful as a reference, but it does not persist console
  definitions, so this kernel owns its own session context.

## CLI REPL

- Run with `cargo run -p jupytypst -- repl`.
- `--mode html|svg` selects the render mode for the whole terminal session;
  default is `html` because SVG is awkward to view directly in a terminal.
- `--page-setup` accepts the same values as the kernel: omitted/default, `none`,
  or custom Typst setup code.
- The REPL has no dot-command namespace, so method chains starting with `.` are
  passed through as Typst code.
- In a TTY, Ctrl-C clears the current input buffer, a second consecutive Ctrl-C
  exits, Ctrl-D exits, and Shift+Enter force-submits the current input when the
  terminal reports Shift+Enter distinctly.

## Environment

- Python/Jupyter is managed by `uv`.
- Use `UV_CACHE_DIR=/tmp/jupytypst-uv-cache` if `uv` needs a writable cache.
- Reference projects live under `local/` and are ignored by git.
