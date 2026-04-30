# jupytypst

`jupytypst` is a Jupyter kernel and terminal REPL for Typst.

It runs Typst in code mode, keeps session state across cells or REPL inputs,
and can render output as SVG or HTML.

## Install

Install from Git:

```sh
cargo install --git https://github.com/ParaN3xus/jupytypst jupytypst
```

Install the Jupyter kernelspec:

```sh
jupytypst install --user --replace
```

After installing, restart your editor/Jupyter client or Jupyter server if the
kernel does not appear immediately.

## Usage

Start the Jupyter kernel through the installed kernelspec, or run it directly:

```sh
jupytypst start --connection-file <connection.json>
```

Run the terminal REPL:

```sh
jupytypst repl
```

Cells are evaluated as Typst code mode. For example:

```typc
let f(a, b) = a + b
f(1, 2)
```

Literal markup should be wrapped as content:

```typc
[Hello from Typst]
```

The default output format is SVG for notebooks and HTML for the REPL. Notebook
cells can override the format with:

```typc
// jupytypst: format=html
```

Supported formats are currently `svg` and `html`.

## License

This project is licensed under the Apache License 2.0. See [LICENSE](LICENSE).

## Legal

This project is not affiliated with, created by, or endorsed by Typst the brand.
