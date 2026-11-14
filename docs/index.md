# Logix Engine Docs

These are Logix Engine's docs. Its **main page**, what it is and how to get
it, is [www.openlogix.org/engine](https://www.openlogix.org/engine/); the
code is on [GitHub](https://github.com/openlogix/logix-engine).

One open engine to build and test FPGA designs and program boards, for any
IDE or AI agent.

Logix Engine is a single native program that offers FPGA operations as
[MCP](https://modelcontextprotocol.io) tools: lint, build, test, simulate,
format, draw the design and program the board. Frontends such as
[Logix IDE](https://www.openlogix.org/ide/), scripts, CI and AI agents such
as Claude Code start it and call its tools. It downloads the open source
FPGA tools it needs, and knows about boards, with their FPGAs and pins,
from [Logix Catalog](https://www.openlogix.org/logix-catalog/).

Logix Engine is in development, on macOS with Apple Silicon for now. It's
part of [Open Logix](https://www.openlogix.org), free and open source under
MIT or Apache 2.0.

## Documentation

| Page | What it covers |
| --- | --- |
| [Tutorial: Claude Code](tutorial-claude-code.md) | Step by step, from installing the engine to designing, testing and programming a board by prompts |
| [Tool API](tool-api.md) | Every tool, its inputs, outputs and errors, and recipes for frontend developers |
| [Command line](command-line.md) | Launch arguments and logging |
| [Requirements](requirements.md) | Goals, platforms, capabilities, versioning, the error model, milestones and open questions |
| [Architecture](architecture.md) | The design: components, technology choices, testing, distribution and host code examples |

## Get it

Development builds are on the
[Releases](https://github.com/openlogix/logix-engine/releases) page. The
code is on [GitHub](https://github.com/openlogix/logix-engine).
