# logix-engine

One open engine to build, test and program FPGAs, for any IDE or AI agent.

The engine is a single native executable, written in Rust, that offers FPGA operations as [MCP](https://modelcontextprotocol.io) tools over stdin/stdout. Frontends such as Apio, IceStudio, Logix, IDE extensions, CI scripts and AI coding agents launch it as a child process and call its tools. The engine installs and drives proven open source tools, such as Yosys, nextpnr and openFPGALoader, so each frontend can focus on its own user experience instead of reimplementing the toolchain.

## Status

Design phase. Nothing is implemented yet. The first target is **Milestone 1**, a proof of concept on macOS for iCE40 boards with five tools: `get_engine_info`, `get_catalog`, `build`, `list_devices` and `program_board`. It is done when an AI coding agent, such as Claude Code, connected over MCP with no other frontend, creates an iCE40 design from prompts, builds it and programs it on an Alhambra II board.

## Documentation

| Document | What it covers |
| --- | --- |
| [Requirements](docs/requirements.md) | Goals and non-goals, supported platforms, Apio compatibility, file safety, versioning, error model, phasing and open questions |
| [Architecture](docs/architecture.md) | The proposed design: components, technology choices, testing, distribution and host code examples |
| [Tool API (Milestone 1)](docs/tool-api.md) | The MCP tools, their inputs, outputs and errors |
| [Command line (Milestone 1)](docs/command-line.md) | Launch arguments and logging |

## Development

The engine is written in Rust. Install it with [rustup](https://rustup.rs), which reads the Rust version pinned in [rust-toolchain.toml](rust-toolchain.toml) and installs it automatically on first build.

- **macOS with Homebrew:** run `brew install rustup`, then add `$(brew --prefix rustup)/bin` to your `PATH`.
- **Other platforms:** follow the instructions at [rustup.rs](https://rustup.rs).

Check the setup with `cargo --version`.

## License

[Apache-2.0](LICENSE). The packaged open source tools keep their own licenses.
