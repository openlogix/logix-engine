<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://www.openlogix.org/brand/readme/engine-dark.svg">
  <img alt="Logix Engine" src="https://www.openlogix.org/brand/readme/engine-light.svg" height="96">
</picture>

<br>

[![License: MIT or Apache 2.0](https://img.shields.io/badge/License-MIT%20or%20Apache%202.0-blue)](#license)

<img src="https://www.openlogix.org/brand/readme/macos.svg" height="40" alt="macOS (Apple Silicon)" title="macOS (Apple Silicon)">

[![test](https://img.shields.io/github/actions/workflow/status/openlogix/logix-engine/test.yaml?label=test)](https://github.com/openlogix/logix-engine/actions/workflows/test.yaml)
[![build](https://img.shields.io/github/actions/workflow/status/openlogix/logix-engine/build-pre-release.yaml?label=build)](https://github.com/openlogix/logix-engine/actions/workflows/build-pre-release.yaml)
[![docs](https://img.shields.io/github/actions/workflow/status/openlogix/logix-engine/publish-docs.yaml?label=docs)](https://github.com/openlogix/logix-engine/actions/workflows/publish-docs.yaml)

**[Main page](https://www.openlogix.org/engine/)** · [Docs](https://www.openlogix.org/logix-engine/) · GitHub (here)

One open engine to build and test FPGA designs and program boards, for any IDE or AI agent.

The engine is a single native executable, written in Rust, that offers FPGA operations as [MCP](https://modelcontextprotocol.io) tools over stdin/stdout. Frontends such as Logix IDE, Logix CLI, IceStudio, IDE extensions, CI scripts and AI coding agents launch it as a child process and call its tools. The engine downloads and drives proven open source tools, such as Yosys, nextpnr and openFPGALoader, so each frontend can focus on its own user experience instead of reimplementing the toolchain.

## Status

In development, macOS on Apple Silicon only.

- **Milestone 1, done:** a proof of concept for iCE40 boards with seven tools: `get_engine_info`, `download_packages`, `get_catalog` (22 iCE40 boards), `build`, `clean_build`, `list_devices` and `program_board`. An AI agent, Claude Code, changed a design, built it and programmed an Alhambra II board through the engine alone.
- **Milestone 2, next:** Logix IDE running on the engine, keeping every feature it has today: lint, test, simulate, format, diagram, live tool output, and the same boards across the iCE40, ECP5, Gowin and Xilinx 7-series FPGA families. See [Acceptance and phasing](docs/requirements.md#acceptance-and-phasing).
- **Then:** Milestone 3 brings the engine and Logix IDE to Windows, and Milestone 4 to Linux.

Daily development builds are on the [Releases](https://github.com/openlogix/logix-engine/releases) page.

## Documentation

The same docs are published at [www.openlogix.org/logix-engine](https://www.openlogix.org/logix-engine/).

| Document | What it covers |
| --- | --- |
| [Tutorial: FPGA design with Logix Engine and Claude Code](docs/tutorial-claude-code.md) | Step by step, from installing the engine to designing, testing and programming a board by prompts |
| [Requirements](docs/requirements.md) | Goals and non-goals, supported platforms, required capabilities, file safety, versioning, error model, phasing and open questions |
| [Architecture](docs/architecture.md) | The proposed design: components, technology choices, testing, distribution and host code examples |
| [Tool API](docs/tool-api.md) | The MCP tools, their inputs, outputs and errors |
| [Command line](docs/command-line.md) | Launch arguments and logging |

## Development

The engine is written in Rust. Install it with [rustup](https://rustup.rs), which reads the Rust version pinned in [rust-toolchain.toml](rust-toolchain.toml) and installs it automatically on first build.

- **macOS with Homebrew:** run `brew install rustup`. The scripts below find it on their own. To use `cargo` directly, also add `$(brew --prefix rustup)/bin` to your `PATH`.
- **Other platforms:** follow the instructions at [rustup.rs](https://rustup.rs).

Check the setup with `cargo --version`.

| Script | What it does |
| --- | --- |
| `scripts/test.sh` | All checks and tests: format, lint, unit, schema, command-line, build and end-to-end MCP tests. Run it before committing. CI runs it too. The first run downloads the FPGA tools (the `logix-core` package) for the build tests |
| `scripts/build.sh` | Builds the release binary, `target/release/logix-engine` |
| `scripts/run-in-claude.sh` | Builds if needed and starts Claude Code with the engine connected, so you can prompt it. Claude works in `~/.logix/engine-dev/playground`, and the engine uses `~/.logix/engine-dev/engine` |
| `scripts/run-in-grok.sh` | The same with Grok as the host. It writes the engine into the playground's `.grok/config.toml` |
| `scripts/engine-cmd.sh` | Sends one prompt to Claude Code with the engine connected and prints the answer as plain text, such as `scripts/engine-cmd.sh --add-dir ~/proj "Build ~/proj for the iCESugar v1.5"`. `-c` continues the previous conversation. See `--help` |
| `scripts/serve.sh` | Builds if needed and runs the engine as an MCP server on stdin/stdout, for other MCP clients |
| `scripts/clean.sh` | Deletes the build output |
| `scripts/update-schemas.sh` | Regenerates [schemas/tools.json](schemas/tools.json), the published copy of the tool schemas, after a deliberate change |

To try the engine with Claude Code as the host, run `scripts/run-in-claude.sh` and type prompts, such as "What version is Logix Engine?". It uses the `claude` command, or the one bundled with the Claude Code VS Code extension. `scripts/run-in-grok.sh` does the same with Grok. Both start the agent from a clean slate for reproducible hand tests: none of your own agent settings, instructions or memories, a copy of the engine outside the repo, and rules asking the agent to start only from what the engine provides. To try it with another MCP client, such as the [MCP Inspector](https://github.com/modelcontextprotocol/inspector), point the client at `scripts/serve.sh`:

```
npx @modelcontextprotocol/inspector /path/to/logix-engine/scripts/serve.sh
```

## License

[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. The packaged open source tools keep their own licenses.
