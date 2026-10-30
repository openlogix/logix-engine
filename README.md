# logix-engine

One open engine to build, test and program FPGAs, for any IDE or AI agent.

The engine is a single native executable, written in Rust, that offers FPGA operations as [MCP](https://modelcontextprotocol.io) tools over stdin/stdout. Frontends such as Apio, IceStudio, Logix, IDE extensions, CI scripts and AI coding agents launch it as a child process and call its tools. The engine installs and drives proven open source tools, such as Yosys, nextpnr and openFPGALoader, so each frontend can focus on its own user experience instead of reimplementing the toolchain.

## Status

In development. The engine runs as an MCP server with its first tools: `get_engine_info`, `install_packages`, which downloads and installs the FPGA toolchain, `get_catalog`, with 22 iCE40 boards, `build`, and `clean_build`. Board programming comes next. Daily development builds are on the [Releases](https://github.com/openlogix/logix-engine/releases) page. The first target is **Milestone 1**, a proof of concept on macOS for iCE40 boards with seven tools: `get_engine_info`, `install_packages`, `get_catalog`, `build`, `clean_build`, `list_devices` and `program_board`. It is done when an AI coding agent, such as Claude Code, connected over MCP with no other frontend, creates an iCE40 design from prompts, builds it and programs it on an Alhambra II board.

## Documentation

| Document | What it covers |
| --- | --- |
| [Requirements](docs/requirements.md) | Goals and non-goals, supported platforms, Apio compatibility, file safety, versioning, error model, phasing and open questions |
| [Architecture](docs/architecture.md) | The proposed design: components, technology choices, testing, distribution and host code examples |
| [Tool API (Milestone 1)](docs/tool-api.md) | The MCP tools, their inputs, outputs and errors |
| [Command line (Milestone 1)](docs/command-line.md) | Launch arguments and logging |

## Development

The engine is written in Rust. Install it with [rustup](https://rustup.rs), which reads the Rust version pinned in [rust-toolchain.toml](rust-toolchain.toml) and installs it automatically on first build.

- **macOS with Homebrew:** run `brew install rustup`. The scripts below find it on their own. To use `cargo` directly, also add `$(brew --prefix rustup)/bin` to your `PATH`.
- **Other platforms:** follow the instructions at [rustup.rs](https://rustup.rs).

Check the setup with `cargo --version`.

| Script | What it does |
| --- | --- |
| `scripts/test.sh` | All checks and tests: format, lint, unit, schema, command-line, build and end-to-end MCP tests. Run it before committing. CI runs it too. The first run downloads the FPGA toolchain (526 MB) for the build tests |
| `scripts/build.sh` | Builds the release binary, `target/release/logix-engine` |
| `scripts/run.sh` | Builds if needed and starts Claude Code with the engine connected, so you can prompt it. Claude works in `~/.logix-dev/playground`, and the engine uses `~/.logix-dev/engine` |
| `scripts/serve.sh` | Builds if needed and runs the engine as an MCP server on stdin/stdout, for other MCP clients |
| `scripts/clean.sh` | Deletes the build output |
| `scripts/import-apio-catalog.py` | Regenerates the board and FPGA catalog in [catalog/](catalog/) from Apio's definitions in `~/.apio/packages` |
| `scripts/update-schemas.sh` | Regenerates [schemas/tools.json](schemas/tools.json), the published copy of the tool schemas, after a deliberate change |

To try the engine with Claude Code as the host, run `scripts/run.sh` and type prompts, such as "What version is the Logix engine?". It uses the `claude` command, or the one bundled with the Claude Code VS Code extension. To try it with another MCP client, such as the [MCP Inspector](https://github.com/modelcontextprotocol/inspector), point the client at `scripts/serve.sh`:

```
npx @modelcontextprotocol/inspector /path/to/logix-engine/scripts/serve.sh
```

## License

[Apache-2.0](LICENSE). The packaged open source tools keep their own licenses.
