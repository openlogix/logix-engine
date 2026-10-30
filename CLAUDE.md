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

macOS arm64 only, iCE40 only, with the Alhambra II (`alhambra-ii`) as the reference board, six tools: `get_engine_info`, `install_packages`, `get_catalog`, `build`, `list_devices`, `program_board`. Also: automatic package install with versioning, a read-only catalog, the error model, GitHub workflows that build and test the engine, and developer scripts in `scripts/`. Toolchain packages come from Apio's packages. Don't build later-phase features unless asked.

## Code and commands

- `src/main.rs`: the command line. Everything else is a library (`src/lib.rs`), so tests can use it.
- `src/server.rs`: the MCP server (rmcp). `src/tools/`: one file per tool, each with its input and output types and an `EngineTool` impl. Add a tool to `all_definitions()` and `call()` in `src/tools/mod.rs`.
- `src/errors.rs`: the error model. `src/paths.rs`: path checks. `src/logging.rs`: log files. `src/text.rs`: the text forms. `src/state.rs`: launch settings, engine version, protocol versions, platform.
- `src/packages/`: the package manager. `manifest.json` pins each package's release, file, size and SHA-256 per platform. To update a package, take the asset's size and digest from the GitHub release (`gh release view <tag> -R <owner>/<repo> --json assets`). `src/progress.rs`: progress by phase. `src/mcp_progress.rs`: sends it as MCP progress notifications when the request has a progress token. Each tool gets a `Call` with the progress sink and a cancellation token.
- `build.rs` sets the engine version: the `LOGIX_ENGINE_VERSION` env var for releases (the release tag), otherwise today's UTC date.
- Tests: unit tests next to the code; `tests/cli.rs` (command line), `tests/mcp.rs` (end to end: launches the binary and talks MCP), `tests/schemas.rs` (published schemas), `tests/packages.rs` (package install against a local test server). The ignored test `real_oss_cad_suite_install` downloads the real package (526 MB): `cargo test --test packages -- --ignored`, and `real_install_packages_with_progress` does the same through MCP with progress: `cargo test --test mcp -- --ignored`. Run them after changing the manifest, the install code or progress reporting.
- **Before committing, run `scripts/test.sh`.** After a deliberate schema change, run `scripts/update-schemas.sh` and review the diff in `schemas/tools.json`.
- Rust is pinned in `rust-toolchain.toml`. With Homebrew's rustup, `cargo` is in `/opt/homebrew/opt/rustup/bin`, which isn't on the PATH by default. Every script sources `scripts/_rust-env.sh`, which adds it when `cargo` isn't found.

## CI and releases

Same conventions as logix-ide, using the shared actions from `openlogix/.github` (`openlogix/.github/actions/<name>@v1`; that repo documents and tests them).

- **`test.yaml`**: on every push and pull request, manually, and daily at 00:00 UTC; a newer run for the same ref cancels the older one; matrix macos-15, macos-26, macos-latest, `fail-fast: false`, 30 min. Installs the pinned Rust, then runs `scripts/test.sh`.
- **`build-pre-release.yaml`** (on every push to `main`, daily at 00:00 UTC, and manually; a newer run cancels a running one), three jobs:
  - `prepare` (Ubuntu): release tag `yyyy-mm-dd` and package tag `yyyymmdd`, an early check that no stable release has that tag, repo info, and `BUILD-INFO.json` (with `engine-version` = the release tag) as the `build-info` artifact.
  - `build-darwin-arm64` (reusable `build-darwin-arm64.yaml`, macos-latest): checks out exactly `repo-commit` into `_repo/`, runs `scripts/test.sh`, builds with `LOGIX_ENGINE_VERSION` = the release tag, packs `logix-engine-darwin-arm64-<package tag>.tgz` (one `logix-engine/` folder with the binary and `LICENSE`), and checks that the packed binary reports the release tag and has an ad-hoc signature.
  - `pre-release` (Ubuntu): virus scan, release notes (what the engine is, that these are development builds, macOS steps with `xattr -c`, trying it with `claude mcp add`, the packages table, build info, about these releases), keeps the 5 newest pre-releases, replaces a same-tag pre-release, publishes the `.tgz` and `BUILD-INFO.json`.
- Add Linux and Windows as `build-<platform>.yaml` and test matrix entries when those platforms are implemented.

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
