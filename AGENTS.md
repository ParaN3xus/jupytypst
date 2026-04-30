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
- By default, each rendered cell gets
  `#set page(width: auto, height: auto, margin: 16pt)` before user code.
  `--page-setup none` disables this, and `--page-setup <Typst code>` overrides
  it.
- Persist only top-level definition/configuration statements between cells:
  `let`, `set`, `show`, `import`, and `include`.
