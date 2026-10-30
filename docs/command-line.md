# Command line (Milestone 1)

A host launches the engine as a child process on the same computer and talks MCP with it over stdin and stdout. This document defines the engine's command line. Like the [tool API](tool-api.md), it is part of the engine's interface: arguments are only ever added, never removed or changed in meaning.

**Table 1.** Command-line arguments

| Argument | Required | Description |
| --- | --- | --- |
| `--engine-folder <absolute path>` | Yes | The host's engine folder. It must satisfy the path keyword rules. Hosts don't share engine folders by default |
| `--no-auto-install` | No | Turns off automatic package install and repair. A missing or broken package is then a setup error with `install_packages` |
| `--log-level <level>` | No | One of `error`, `warn`, `info` and `debug`. Defaults to `info` |
| `--version` | No | Prints the engine version and exits, without starting MCP |
| `--help` | No | Prints the arguments and exits, without starting MCP |

Example:

```
logix-engine --engine-folder /Users/u/.apio/engine --log-level debug
```

## Rules

- **stdout is only for MCP.** Logs, tool output and messages never go to stdout, since they would corrupt the protocol stream.
- **Bad arguments.** An unknown argument, a missing `--engine-folder` or an engine folder that fails the path checks makes the engine exit before starting MCP, with a non-zero exit code and a message on stderr.
- **Launch settings are fixed.** The arguments apply for the life of the process. A host that wants different settings launches a new instance.

## Logs

- **Location.** The engine always writes a log file in the `logs` subfolder of the engine folder, such as `~/.apio/engine/logs`. No argument is needed, and since the engine folder is engine-owned, no new file rule is needed either.
- **One file per process.** Each instance writes its own file, named with the date and process ID, such as `20261028-4123.log`, so instances sharing an engine folder don't mix their lines.
- **Cleanup.** The engine deletes old log files automatically, keeping the last 20 files and at most 50 MB.
- **stderr.** Warnings and errors also go to stderr. Hosts such as Claude Code show an MCP server's stderr in their own logs, which helps when the engine fails at startup.
- **Level.** `--log-level` sets how much goes into the log file. A host can raise it to debug when chasing a problem.
- **Finding the file.** `get_engine_info` returns the path of the current log file, so a host can show it to the user.
