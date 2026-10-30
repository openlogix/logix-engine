# CLAUDE.md

Guidance for Claude working in this repo. The documents in `docs/` are the source of truth for what the engine must do. Read the relevant one before changing behavior or interfaces.

## Project

logix-engine is an open source FPGA engine: one Rust binary per platform that exposes FPGA operations as MCP tools over stdin/stdout. Hosts (Apio, IceStudio, Logix, IDE extensions, CI, AI agents) launch it as a child process. It drives existing open source tools (Yosys, nextpnr, openFPGALoader and others) and never implements a toolchain itself.

| Document | Read it when |
| --- | --- |
| [docs/requirements.md](docs/requirements.md) | Deciding what the engine must or must not do |
| [docs/architecture.md](docs/architecture.md) | Adding components, dependencies or tests |
| [docs/tool-api.md](docs/tool-api.md) | Adding or changing a tool, field, error or progress report |
| [docs/command-line.md](docs/command-line.md) | Changing launch arguments or logging |

## Current scope: Milestone 1

macOS arm64 only, iCE40 only, five tools: `get_engine_info`, `get_catalog`, `build`, `list_devices`, `program_board`. Also: automatic package install with versioning, a read-only catalog, the error model, GitHub workflows that build and test the engine, and developer scripts `run.sh`, `build.sh` and `clean.sh`. Toolchain packages come from Apio's packages. Don't build later-phase features unless asked.

## Rules that are easy to break

- **stdout is only for MCP.** Logs and tool output go to the log file and stderr, never stdout.
- **The tool API and command line never break** after the first release. Changes only add tools, optional fields or arguments. A breaking change gets a new tool name.
- **Stateless requests.** Every request carries its own folders and board. The engine never reads host project files or merges catalog definitions from disk. Requests never carry source text.
- **Safe file changes.** Check the path keyword rules, resolve symbolic links, and refuse forbidden locations before any disk change. The engine writes only in the engine folder and build folders, plus in-place rewrites of named source files when an operation calls for it, such as formatting.
- **Errors are data.** Use the six error categories and the suggested action codes from the tool API. A failing test or missed timing is a result, not an error. Messages never assume a particular host.
- **Schemas come from Rust types.** Don't hand-write JSON schemas. Every tool and field needs a clear, self-contained description, because AI agents learn the tools from them alone.
- **No network at startup, no silent upgrades.** Network access happens only to install or repair packages an operation needs, or when a host asks.
- **Date versions.** Canonical form `yyyy-mm-dd`. Compare by date, never as mixed-form strings.

## How work is done here

The human directs the work and tests the results. Claude writes and maintains the code, the tests and the docs. Humans don't review every line, so:

- Add or update tests with every change. Tests are the main guard against regressions.
- Keep external interfaces, requirements and docs simple and easy for people to understand. Code readability matters less than correctness and test coverage.
- Update the docs in `docs/` in the same change as the code they describe. If code and docs disagree, ask which one is right.
- Record open questions in the relevant doc's "Open questions" section, or as a GitHub issue if the user asks.
