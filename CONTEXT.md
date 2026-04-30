# Project Context

`jupytypst` is a Typst Jupyter kernel written in Rust. It should support
interactive Typst evaluation with notebook-friendly display outputs.

## Goals

- Provide a `clap` CLI with `start` and `install` commands.
- Install a Jupyter kernelspec that launches this binary.
- Maintain useful Typst context across cells without re-rendering previous
  visible content.
- Support three execution modes:
  - `eval`: evaluate Typst code and display `text/plain`.
  - `svg`: render Typst markup as `image/svg+xml`.
  - `html`: render Typst markup as `text/html`.

## Design Notes

- Use `jupyter-protocol` for Jupyter message structures and MIME bundles.
- Use `zeromq` for the kernel sockets.
- Use Typst compiler crates for evaluation and rendering.
- Tinymist DAP REPL is useful as a reference, but it does not persist console
  definitions, so this kernel owns its own session context.

## Environment

- Python/Jupyter is managed by `uv`.
- Use `UV_CACHE_DIR=/tmp/jupytypst-uv-cache` if `uv` needs a writable cache.
- Reference projects live under `local/` and are ignored by git.
