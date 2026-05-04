# Open Interpreter 1.0 Prototype

The Open Interpreter 1.0 Prototype is a provider-agnostic terminal coding agent based on
Codex.

## What it does

Open Interpreter is built to work in your terminal:

- run `interpreter` and work in the current directory
- choose the provider and model that fit your workflow
- keep configuration and session state local in `~/.openinterpreter`
- receive standalone app updates automatically by default

## Core ideas

- **Provider agnostic:** model and provider choice are first-class parts of the
  product.
- **Memory efficient:** the runtime is designed around a shared local backend so
  many tabs do not have to behave like many fully separate agent runtimes.

## License

Apache-2.0
