# Project Context

`jupytypst` is a Typst Jupyter kernel written in Rust. It should support
interactive Typst evaluation with notebook-friendly display outputs.

## Goals

- Provide a `clap` CLI with `start` and `install` commands.
- Install a Jupyter kernelspec that launches this binary.
- Maintain useful Typst context across cells without re-rendering previous
  visible content.
- Support two execution modes:
  - `svg`: render Typst markup as notebook HTML containing per-page SVG.
  - `html`: render Typst markup as `text/html`.
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

## Environment

- Python/Jupyter is managed by `uv`.
- Use `UV_CACHE_DIR=/tmp/jupytypst-uv-cache` if `uv` needs a writable cache.
- Reference projects live under `local/` and are ignored by git.
