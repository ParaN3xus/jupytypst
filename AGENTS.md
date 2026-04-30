# Repository Instructions

This workspace implements `jupytypst`, a Rust Jupyter kernel for Typst, plus a
reusable `typsess` library crate for stateful Typst code-mode execution.

## Workflow

- Keep commits small and use conventional commit prefixes such as `feat:`,
  `fix:`, `docs:`, `test:`, and `chore:`.
- Run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and
  `cargo test --workspace` before committing Rust code changes.
- Do not commit `local/`, `.venv/`, or `target/`.
- Prefer small modules over large files when adding kernel, Typst, REPL, or CLI
  logic.

## Workspace Layout

- `crates/typsess` owns Typst evaluation/rendering state and exposes the
  library API used by hosts.
- `crates/jupytypst` owns the Jupyter protocol server, kernelspec installer,
  notebook cell directives, and terminal REPL binary.
- Keep all third-party dependency versions in the root `[workspace.dependencies]`;
  member crates should use `.workspace = true`.

## References

- `local/typst` contains the Typst compiler/exporter reference source.
- `local/tinymist` contains the Tinymist LSP/DAP reference source.
- Keep reference-source use read-only unless the user explicitly asks otherwise.
- The kernel uses `tinymist-world` as its Typst `World` implementation for
  package imports, filesystem access, and font discovery. Avoid reintroducing a
  local ad hoc `World` unless the Tinymist API cannot support the needed case.

## Kernel Behavior

- Cell directives use Typst comments, for example
  `// jupytypst: mode=svg`.
- Supported modes are `svg` and `html`; `svg` is the default.
- Directive parsing belongs in `crates/jupytypst`, not in `typsess`.
- `typsess` returns structured `PagedDocument`/`HtmlDocument` outputs and Typst
  `SourceDiagnostic`s. Host-facing HTML wrapping and diagnostic formatting
  belong in `crates/jupytypst`.
- Cells run in Typst code mode, not markup mode. Use `let x = 1` instead of
  `#let x = 1`, and wrap literal content in content blocks such as `[Text]`.
- By default, the session initializes its persistent styles with
  `set page(width: auto, height: auto, margin: 16pt)` once at kernel startup.
  `--page-setup none` disables this, and `--page-setup <Typst code>` overrides
  it.
- Persist Typst execution state through the VM's top-level `Scope` and captured
  top-level `Styles`, not by concatenating prior cell source.
- Top-level `let`/imports update the persistent scope. Top-level selector
  `show` rules and `set` rules update persistent styles.
- Persist Typst introspection state by carrying forward invisible
  `state`/`counter` update content between cells. Do not carry forward visible
  cell content.
- Anonymous top-level `show: ...` rules persist between cells like other
  top-level show rules.
- Page setup must reset for each rendered cell. Do not persist transient page
  sizing fields: `page.paper`, `page.width`, and `page.height`. Other page
  fields, such as `fill`, may persist.

## REPL Behavior

- `jupytypst repl` starts a terminal REPL backed by `typsess`.
- The terminal REPL does not parse `// jupytypst:` directives. Its render mode
  is chosen once at startup with `--mode`, defaulting to `html`.
- The REPL has no dot-command namespace; leading `.` is valid Typst code.
- In a TTY, Ctrl-C clears the current input buffer, a second consecutive Ctrl-C
  exits, Ctrl-D exits, and Shift+Enter force-submits the current input when the
  terminal reports Shift+Enter distinctly.
