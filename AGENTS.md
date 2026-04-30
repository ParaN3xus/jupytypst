# Repository Instructions

This repository implements `jupytypst`, a Rust Jupyter kernel for Typst.

## Workflow

- Keep commits small and use conventional commit prefixes such as `feat:`,
  `fix:`, `docs:`, `test:`, and `chore:`.
- Run `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test`
  before committing Rust code changes.
- Do not commit `local/`, `.venv/`, or `target/`.
- Prefer small modules over large files when adding kernel, Typst, or CLI logic.

## References

- `local/typst` contains the Typst compiler/exporter reference source.
- `local/tinymist` contains the Tinymist LSP/DAP reference source.
- Keep reference-source use read-only unless the user explicitly asks otherwise.

## Kernel Behavior

- Cell directives use Typst comments, for example
  `// jupytypst: mode=svg`.
- Supported modes are `svg` and `html`; `svg` is the default.
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
- Anonymous top-level `show: ...` rules are cell-local because they cannot be
  replayed safely as persistent style recipes; emit a user-visible warning.
- Page setup must reset for each rendered cell. Do not persist transient page
  sizing fields: `page.paper`, `page.width`, and `page.height`. Other page
  fields, such as `fill`, may persist.
