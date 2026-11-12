# Command line

A host launches the engine as a child process on the same computer and talks MCP with it over stdin and stdout. This document defines the engine's command line. Like the [tool API](tool-api.md), it is part of the engine's interface: arguments are only ever added, never removed or changed in meaning.

**Table 1.** Command-line arguments

| Argument | Required | Description |
| --- | --- | --- |
| `--engine-folder <absolute path>` | Yes | The host's engine folder. It must satisfy the path keyword rules. Hosts don't share engine folders by default |
| `--no-auto-download` | No | Turns off automatic package download and repair, and the engine's own reading of its remote config. A missing or broken package is then a setup error with `download_packages`; `download_packages` and `check_updates` still work, since the host asks |
| `--log-level <level>` | No | One of `error`, `warn`, `info` and `debug`. Defaults to `info` |
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

## Diagnostic settings

Environment variables the engine reads at startup, for Open Logix developers and for testing. A bad value stops the engine with a message on stderr, never a silent fallback.

| Variable | Description |
| --- | --- |
| `LOGIX_ENGINE_DEV_REMOTE_CONFIG` | A remote config version folder to read instead of the engine's own: an `https://` URL, such as a pull request's `remote-config/logix-engine/001/` on its branch or a fork, or a local folder. `get_engine_info` reports it in `remote_config` |
| `LOGIX_ENGINE_CHANNEL` | The release channel to read, such as `prod` or `dev`, instead of the one `channels.jsonc` gives this release. A name that isn't a channel name stops the engine; a channel the folder doesn't have is a setup error. `get_engine_info` reports it in `remote_config` (`channel_reason`) |
| `LOGIX_ENGINE_DEV_CATALOG_FOLDER` | A catalog folder, such as a logix-catalog clone, to read instead of the `logix-catalog` package. `get_engine_info` reports it in `catalog` |

## Logs

- **Location.** The engine always writes a log file in the `logs` subfolder of the engine folder, such as `~/.logix/ide/engine/logs`. No argument is needed, and since the engine folder is engine-owned, no new file rule is needed either.
- **One file per process.** Each instance writes its own file, named with the date and process ID, such as `20261028-4123.log`, so instances sharing an engine folder don't mix their lines.
- **Cleanup.** The engine deletes old log files automatically, keeping the last 20 files and at most 50 MB.
- **stderr.** Warnings and errors also go to stderr. Hosts such as Claude Code show an MCP server's stderr in their own logs, which helps when the engine fails at startup.
- **Level.** `--log-level` sets how much goes into the log file. A host can raise it to debug when chasing a problem.
- **Boards.** At `info`, every tool that deals with boards logs its custom boards folder, when the request names one (its path, the first 12 hex digits of a SHA-256 of its board files, so a log tells which version of them was used, and its board IDs or how many problems they have), and the board it uses (its ID, whether it comes from the catalog or the custom boards folder, and its variant's selector).
- **Finding the file.** `get_engine_info` returns the path of the current log file, so a host can show it to the user.
