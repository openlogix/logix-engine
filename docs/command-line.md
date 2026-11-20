# Command line

A host launches the engine as a child process on the same computer and talks MCP with it over stdin and stdout. This document defines the engine's command line. Like the [tool API](tool-api.md), it is part of the engine's interface: arguments are only ever added, never removed or changed in meaning.

**Table 1.** Command-line arguments

| Argument | Required | Description |
| --- | --- | --- |
| `--engine-folder <absolute path>` | Yes | The host's engine folder. It must satisfy the path keyword rules. Hosts don't share engine folders by default |
| `--no-auto-download` | No | Turns off automatic package download and repair, and the engine's own reading of its remote config. A missing or broken package is then a setup error with `download_packages`; `download_packages` and `check_updates` still work, since the host asks |
| `--log-level <level>` | No | One of `error`, `warn`, `info` and `debug`. Defaults to `info` |
| `--log-folder <absolute path>` | No | The host's log session folder (see Logs). The engine writes its log there, and its programs' (GTKWave's), and deletes nothing in it. Without it, the engine is at the top and makes its own session folder in its engine folder's `logs` |
| `--log-name <name>` | No | The engine's name in the host's session folder: the chain of programs that started it, joined by dots, such as `my-ide.engine` (the engine writes `my-ide.engine.log`). Lowercase letters, digits, `-` and `.`. Needs `--log-folder`; defaults to `engine` |
| `--version` | No | Prints the engine version and exits, without starting MCP |
| `--help` | No | Prints the arguments and exits, without starting MCP |

Example:

```
logix-engine --engine-folder /Users/user/.myhost/engine --log-level debug
```

## Rules

- **stdout is only for MCP.** Logs, tool output and messages never go to stdout, since they would corrupt the protocol stream.
- **Bad arguments.** An unknown argument, a missing `--engine-folder` or an engine folder that fails the path checks makes the engine exit before starting MCP, with a non-zero exit code and a message on stderr.
- **Launch settings are fixed.** The arguments apply for the life of the process. A host that wants different settings launches a new instance.
- **Internal: the layout worker.** The engine starts itself as `logix-engine --diagram-layout-worker <ELK bundle>` to lay out diagrams in a process of its own (one JSON graph per line in, one result per line out), so a layout that fails can't stop the engine. It isn't for hosts and may change at any time.
- **Internal: the driver helper (Windows).** `install_driver` and `uninstall_driver` start the engine again as an administrator, as `logix-engine --elevated-driver-helper <request.json>`, to install or remove a board's WinUSB driver. It isn't for hosts and may change at any time.

## Diagnostic settings

Environment variables the engine reads at startup, for Open Logix developers and for testing. A bad value stops the engine with a message on stderr, never a silent fallback. `get_engine_info` lists the ones set in `developer_settings`, and its report in a Developer settings section.

| Variable | Description |
| --- | --- |
| `LOGIX_ENGINE_DEV_REMOTE_CONFIG` | A remote config version folder to read instead of the engine's own: an `https://` URL, such as a pull request's `remote-config/logix-engine/001/` on its branch or a fork, or a local folder. `get_engine_info` reports it in `remote_config` |
| `LOGIX_ENGINE_CHANNEL` | The release channel to read, such as `prod` or `dev`, instead of the one `assign.jsonc` gives this release. A name that isn't a channel name stops the engine; a channel the folder doesn't have is a setup error. `get_engine_info` reports it in `remote_config` (`channel_reason`) |
| `LOGIX_ENGINE_DEV_CATALOG_FOLDER` | A catalog folder, such as a logix-catalog clone, to read instead of the `logix-catalog` package. `get_engine_info` reports it in `catalog` |

## Logs

The engine follows the Open Logix log layout, the same for every Open Logix program, and third-party hosts are invited to follow it too:

- **A folder per session.** Each run of the program at the top, the one people start, makes a *session folder* in its own `logs` folder, named by its start (local time) and process ID: `<yyyymmdd>-<hhmmss>-<pid>`, such as `20261115-171205-4412`. Every program it starts, and each one those start, writes into that folder.
- **One file per program, named by its chain of programs.** The top program's file is its own name; a program it started adds its name after a dot, and so on down: `my-ide.log`, `my-ide.engine.log`, `my-ide.engine.gtkwave.log`. Open Logix programs go by their short name, without `logix-` (Logix Engine is `engine`, Logix IDE `ide`); other programs by their own (`gtkwave`). So a name says who started the program, and sorting the names gives the tree.
- **Each run appends.** A program that runs again in the session, such as an engine restarted by its host, adds to its file after a separator line naming it, its version or what it opened, the local time with its offset from UTC, and its process ID: `=== 2026-11-15 17:12:05 +02:00  Logix Engine 2026.11.15 started (process 98223) ===`. Nothing is ever renamed.
- **The engine at the top.** Without `--log-folder`, as when Claude Code starts it, the engine is the top program: it makes its own session folder in its engine folder's `logs` (such as `~/.logix/claude-code/engine-data/logs/20261115-172001-5120/engine.log`), and at startup deletes its old session folders (`clean_logs`'s policy, in the [tool API](tool-api.md)).
- **The engine under a host.** With `--log-folder` and `--log-name`, the engine writes `<name>.log` in the host's session folder and deletes nothing there: cleanup is the host's, which can call `clean_logs` for it. A host that runs several engines at once gives each its own name or folder.
- **GTKWave.** When `simulate` opens GTKWave, the engine appends a separator to `<name>.gtkwave.log` in its session folder and gives GTKWave that file as its output, so GTKWave writes to it directly, also after the engine exits. Several open windows append to the same file, so their lines may mix.
- **Size.** Above 10 MB, a log gets no more debug lines (the tools' full output) and says so; messages still go in. GTKWave's file has no cap, since GTKWave writes it; cleanup bounds it.
- **stderr.** Warnings and errors also go to stderr. Hosts such as Claude Code show an MCP server's stderr in their own logs, which helps when the engine fails at startup.
- **Level.** `--log-level` sets how much goes into the log file. A host can raise it to debug when chasing a problem.
- **Boards.** At `info`, every tool that deals with boards logs its custom boards folder, when the request names one (its path, the first 12 hex digits of a SHA-256 of its board files, so a log tells which version of them was used, and its board IDs or how many problems they have), and the board it uses (its ID, whether it comes from the catalog or the custom boards folder, and its variant's selector).
- **Finding the file.** `get_engine_info` returns the path of the current log file, so a host can show it to the user. For a problem report, attach the session folder's files.
