# Requirements

Draft requirements for the engine. [Architecture](architecture.md), [Tool API](tool-api.md) and [Command line](command-line.md) hold the proposed design.

## Terminology

**Table 1.** Terms used in this document

| Term | Definition |
| --- | --- |
| Engine | Short for FPGA engine. The single component this document specifies. It performs FPGA operations on behalf of its hosts |
| Host | The program that launches the engine as a child process on the same computer, shares folders with it and talks to it over MCP. A host is either a local frontend or the network adapter |
| Frontend | A program that people use to do FPGA work, such as an IDE, a CLI tool, an AI agent or a browser app. Most frontends are also hosts, but remote frontends, such as browser apps, are not: they reach the engine through the network adapter, which is the host |
| Network adapter | An optional host that relays requests from remote frontends, such as browser apps, to the engine. It handles auth and file transfer |
| Operation | A unit of functionality the engine offers, such as build, simulate or program board |
| Instance | One running engine process, from launch to exit. With stdin/stdout, each instance serves exactly one session |
| Session | One connection between a host and the engine, from start to close. The host decides how long it lasts |
| Catalog | The engine's collection of known boards, with their programming variants, and FPGAs, kept as data. It is read-only and fixed for each engine version |
| Toolchain | The open source FPGA tools the engine runs to perform operations, such as synthesis and place-and-route tools |
| Shared folders | Folders on the host's computer that both the host and the engine use: the engine folder, the source folder and the build folder. The host chooses their locations: the engine folder when it launches an instance, the source and build folders with each request. They may overlap |
| Source folder | The shared folder that holds the design's sources, such as HDL files, testbenches and constraint files. The host owns it and names it in each request. The engine reads it, and writes to it only when an operation calls for it, such as formatting. It may also hold other files, such as a frontend's project files. The host can exclude parts of it from the engine, such as Apio's `_build` folder, even when the build folder is only a subfolder of the excluded part, such as `_build/<env_name>` |
| Build folder | The shared folder where the engine writes generated files, such as bitstreams, reports and waveforms, and keeps any state between requests, such as incremental build data. The host names it in each request. Deleting it may slow the next build but never changes its result |
| Project model | How a frontend organizes a design on disk, such as Apio's `apio.ini` and directory layout. The engine does not impose one |
| MCP | Model Context Protocol, the open standard all hosts use to call engine operations as tools |

## Goals

An open source, low-level FPGA engine that makes it easy to build a wide range of frontends for FPGA development and testing. These are the tools developers use across the whole cycle, from writing and simulating a design to programming a board and verifying it on real hardware. The engine handles the underlying operations, such as synthesis, simulation, testing, board programming, automated on-board verification, and more. That way, each frontend can focus on its own user experience instead of reimplementing the toolchain.

**Table 2.** Frontend types

| Frontend type | Examples |
| --- | --- |
| Graphical tools | IceStudio |
| Command-line tools | Apio |
| Automation | Scripts and workflows |
| IDE extensions | Apio VS Code |
| Standalone IDEs | Logix |
| Browser-based IDEs and playgrounds | None yet |
| AI coding assistants | MCP-based agents |
| HDL frameworks | Amaranth, LiteX, SpinalHDL, Chisel |
| Education platforms and remote labs | None yet |

## Non-goals

The engine is not:

- **An IDE or editor.** No editing, code navigation or live checking as the user types. Hosts do that, and can run packaged tools such as the Verible language server themselves.
- **A user interface.** No windows or dialogs. The one exception is launching a packaged viewer, such as GTKWave, locally when a host asks.
- **A project manager.** No project files, settings, environments or examples. The host owns its project model.
- **A design-format converter.** Block diagrams, Amaranth, LiteX and other formats are turned into Verilog or SystemVerilog by the host before they reach the engine.
- **A network server.** Remote access goes only through the separate, optional network adapter.
- **A new toolchain.** The engine does not implement synthesis, place and route or programming itself. It installs and drives existing open source tools.
- **A front for proprietary vendor tools.** Tools such as Vivado, Quartus or Gowin's own IDE are out of scope. The engine uses open source toolchains only.
- **Self-updating.** The engine never updates itself or its packages unless a host asks.
- **A VHDL toolchain, for now.** VHDL support may become a future goal, but it is not part of the current scope.

## Frontend deployment forms

The engine must support frontends in each of the deployment forms below. Frontends can be written in any language, so the engine must not require a specific programming language. How the engine itself is deployed (library, process, service or other) is an architecture decision, as long as it supports every form below.

**Table 3.** Frontend deployment forms

| Frontend form | Examples | Implementation example |
| --- | --- | --- |
| Desktop graphical design apps | IceStudio | NW.js, Electron or Qt app |
| Desktop standalone IDEs | Logix | |
| Command-line tools | Apio | Python process |
| Scripts and automated workflows | CI runners, containers | Shell scripts, GitHub Actions |
| IDE extensions | Apio VS Code | TypeScript in the VS Code extension host |
| Programs that embed the engine | Amaranth, LiteX | Python library calls |
| Browser-based apps | Served from a remote server, or fully in-browser | JavaScript web page |
| AI coding assistants and agents | | LLM agent making tool calls |
| Remote frontends | Remote labs | Laptop app talking to a lab computer |

[Table 1 in Architecture](architecture.md#host-connections) shows how each of these forms connects to the engine as a host.

The engine always runs with a host, which is either a frontend or an adapter that connects the engine to frontends elsewhere. The engine assumes a trusted host that shares its file system. Forms where the frontend is on another machine, such as remote labs and browser apps served from a remote server, use a separate network adapter layer in front of the engine. That layer handles authentication, file transfer, sandboxed paths and the service lifecycle.

> **Note:** The engine does not run as a standalone network server. Network access is always provided by a separate network adapter in front of the engine. This layer is nice to have, not required.

## Supported platforms

The engine must run on the following platforms. Other platforms may be added in the future.

**Table 4.** Supported platforms

| Platform | Architectures |
| --- | --- |
| macOS | arm64 (Apple Silicon) |
| Linux | x86-64 |
| Windows | x86-64, arm64 |

## Engine functionality

The engine provides the core FPGA operations that frontends build on. The table below lists typical examples of this core functionality. It is not a complete list, and more operations will be added as the project evolves.

**Table 5.** Examples of engine functionality

| Operation | Description |
| --- | --- |
| Build | Synthesize, place and route, and generate the bitstream |
| Verify | Check the HDL source for errors and warnings |
| Format | Format the HDL source code |
| Graph | Generate a graphical view of the design |
| Report | Report resource utilization and timing |
| Simulate | Run a testbench and show the waveforms |
| Test | Run testbenches automatically and report pass or fail |
| Program board | Program the bitstream to the board |
| Devices and drivers | Manage drivers and devices that are required to communicate with the FPGA boards. |
| Catalog | Provide and manage a standard catalog of known FPGAs and boards, though operations will not be restricted to those items. |

## Reference hosts

Apio, IceStudio and Logix must each be implementable as a host of the engine, using the engine's operations instead of their own toolchain handling or calls to Apio. These three are the reference hosts that validate the engine's interface. The [Apio compatibility](#apio-compatibility) section covers what Apio needs. Table 6 lists what IceStudio and Logix need beyond that.

**Table 6.** Requirements from IceStudio and Logix beyond Apio

| Area | The engine must support | Needed by |
| --- | --- | --- |
| Structured diagnostics | Errors and warnings as data, with file, line, column and severity, so a host can highlight them in its editor or on its graphical blocks | IceStudio, Logix |
| Progress and cancellation | Reporting progress during long operations, and stopping an operation when the host cancels it | IceStudio, Logix |
| Results as data | Resource utilization, timing and test results as structured data, not only as text reports | IceStudio, Logix |
| Board pinouts | Each board's pins and their names in the catalog, so a graphical host can let users pick pins | IceStudio |
| Setup from a GUI | Installing and updating toolchain packages and drivers with progress reporting, without a terminal | IceStudio, Logix |
| Bundled engine | A host shipping a specific engine version, with its own engine folder | IceStudio, Logix |
| Packaged tools for hosts | Letting local hosts run packaged tools themselves, such as the Verible language server for live checks in an editor. An engine request returns the tool's command and environment, so hosts never depend on the engine folder's layout | Logix |
| Parallel work | Running a check while a build is in progress, met by the host running a second engine instance | Logix (to confirm) |

**AI agents as hosts.** A generic MCP client, such as an AI coding agent, must be able to act as the host itself, with no frontend. It launches the engine with an engine folder and needs nothing else. It learns the tools from their names, descriptions and input schemas alone, and acts on the structured errors and suggested actions. This means every tool and field needs a clear, self-contained description.

## Apio compatibility

The engine must support all of Apio's current functionality, so that Apio can be reimplemented as a host on top of it. Apio is the baseline: anything Apio supports today, the engine must support, as listed below.

**Table 7.** Requirements for reimplementing Apio as a host

| Area | The engine must support |
| --- | --- |
| FPGA architectures | Every architecture Apio supports, including Lattice iCE40, Lattice ECP5, Gowin and AMD/Xilinx 7 Series |
| HDL languages | Verilog and SystemVerilog |
| Constraint files | Each architecture's native format, such as `.pcf` for iCE40, `.lpf` for ECP5 and `.cst` for Gowin |
| Build toolchains | Synthesis, place and route, and bitstream generation for each architecture, with the tools Apio uses, such as Yosys, nextpnr, IceStorm, Project Trellis and Apicula |
| Programmers | Every programmer Apio supports, such as openFPGALoader, iceprog, dfu-util and tinyprog, including board-specific programmer options |
| Simulation and testing | Running testbenches and producing waveform files. When a local frontend asks, launching a packaged waveform viewer, such as GTKWave, fully detached so it outlives the session. The request is refused for remote frontends, which view waveforms on their own |
| Lint and format | Checking and formatting HDL source, as Apio does with Verilator and Verible |
| Reports and graphs | Resource utilization and timing reports, and design graphs in SVG, PNG and PDF |
| Catalog | All of Apio's boards and FPGAs, with each board's programming variants. A project's custom definitions are sent by the host in requests, as described in [Custom definitions](#custom-definitions) |
| Devices and drivers | Listing USB devices, and installing or removing the drivers boards need, such as FTDI drivers on Windows |
| Packages | Installing, updating, listing and repairing toolchain packages for each platform |
| Examples | Not an engine feature. Each host provides its own examples, since examples depend on the host's design format, such as IceStudio's block diagrams. Apio keeps its example projects on its own side |
| Raw tool access | Running any packaged tool directly, with the right environment, using the same engine request that returns a tool's command and environment |
| Machine-readable info | Versions, platform, catalog and package details in structured form, as Apio's `api` command provides |

Project-level settings, such as `apio.ini` and its multiple environments, stay in the frontend, as described in the [project model](#project-model). The frontend passes the selected board and options with each request.

## User design testing

The engine helps users test their own designs, from source checks to verification on real hardware. Results come back in structured form, with pass or fail per test, messages and the paths of any generated files, so that hosts such as CI pipelines and AI agents can act on them.

**Table 8.** Types of user design testing

| Test type | What the engine does | Status |
| --- | --- | --- |
| Lint | Checks the HDL source for errors and warnings | Required (Apio) |
| Simulation | Runs a testbench and writes waveform files to the build folder. A local frontend can ask the engine to open them in a waveform viewer | Required (Apio) |
| Automated testbenches | Runs a set of testbenches without user interaction and reports pass or fail for each | Required (Apio) |
| On-board verification | Programs a real board, drives its inputs and checks its outputs automatically, then reports pass or fail | Goal. The hardware interface and approach are still open |

## Testbenches

The engine follows Apio's testbench rules, so a testbench works unchanged with any build folder, such as one per `apio.ini` env.

- **Naming.** A testbench file name ends with `_tb.v` or `_tb.sv`. This is how the engine tells testbenches from design sources and keeps them out of synthesis.
- **No `$dumpfile`.** A testbench that calls `$dumpfile(...)` is a design error. The engine sets the waveform file's location itself, always inside the build folder. Testbenches still call `$dumpvars` to choose what to record.
- **Interactive define.** The engine defines a macro that is 1 when the simulation is interactive, with a waveform viewer, and 0 for automated test runs. `APIO_SIM` is also defined, with the same value, for Apio compatibility.
- **User defines.** Defines given in the request apply to simulation as well as to build.
- **Waveform viewer setup.** A viewer save file, such as a GTKWave `.gtkw` file, that the user saved next to the testbench is only read, never changed. When there is none, the engine generates one in the build folder, so the source folder isn't modified.

## Project model

The engine does not impose a specific project model. Each frontend defines its own, for example Apio with its `apio.ini` file and project directory structure. The frontend translates its project into explicit inputs for each engine operation, such as source files, top module, board and options.

## Interaction with the host

Hosts interact with the engine in the following ways.

**Table 9.** Interaction with hosts

| Aspect | Description |
| --- | --- |
| Requests and responses | The host sends requests and receives structured responses over MCP, which uses JSON |
| Progress and logs | During long operations, the engine reports progress, output and errors as they happen |
| Shared files | The host owns the source files and the engine reads them. The engine writes generated artifacts, such as bitstreams, reports and waveforms, to the build folder the host specifies |
| Deployment | The host's deployment includes the engine, so users install only the host |

## Interaction with the file system

The engine works with three shared folders. The engine has no hard-wired disk locations. All sources reach the engine through the shared folders: requests never carry source text. The host sets the engine folder when it launches each instance. Each request carries the source and build folders it uses. Any state the engine keeps between requests lives in the build folder. Restarting the engine or deleting the build folder may make the next build slower, but never changes its result.

**Table 10.** Shared folders

| Folder | Access | Contents |
| --- | --- | --- |
| Engine folder | Read and write | Files the engine installs and maintains, such as toolchain tools and packages |
| Source folder | Read and write | The design's HDL sources and testbenches, e.g. read for builds and written when formatting |
| Build folder | Read and write | Generated files such as bitstreams, reports, waveforms and intermediate build files |

Note: the folders may overlap. This allows project models such as Apio's, where the build folder is inside the source folder and the source folder also holds other files such as `apio.ini`. However, the engine does not require this or any other project model.

**Engine folder location.** Each host keeps its own engine folder inside its own home folder, for example `~/.apio/engine` or `~/.logix/engine`, and passes it at launch. Users deal only with the host and do not need to know about the engine. Hosts do not share engine folders by default. This keeps them independent, which matters because different hosts are likely to be pinned to different engine versions.

## Safe file changes

The engine guards every change it makes on disk, including creating, writing, renaming and deleting files and folders, so that a wrong path never damages the user's files.

- **Path keywords.** Like Apio, the engine checks for keywords in folder paths before changing anything. The last folder name of an engine folder path must contain "engine", such as `~/.apio/engine`. One of the last two folder names of a build folder path must contain "build", such as `_build` or `_build/default`. Checking only the last names keeps a parent folder, such as `/home/builder`, from satisfying the rule by accident. A wrong path from a host, such as the home folder, fails the check, so nothing is changed.
- **Host naming.** Hosts must name their engine folders and build folders to satisfy the keyword rules. Apio's existing `~/.apio` and `_build/<env>` layout already fits, with the engine folder at `~/.apio/engine`.
- **Engine-owned folders only.** The engine changes files only inside the engine folder and build folders. The one exception is the source folder, where it may rewrite in place the source files named in a request when an operation calls for it, such as formatting. It never creates, renames or deletes files in the source folder.
- **Paths stay inside.** Every path is resolved to its real location, following symbolic links, before a change. A path that resolves outside its folder is refused, so a symbolic link cannot lead the engine into the user's other files.
- **Forbidden locations.** The engine refuses to use the file system root, the user's home folder or the source folder itself as an engine folder or build folder. A build folder inside the source folder stays allowed, as with Apio's `_build` folder.
- **Refusal, not a guess.** If any check fails, the engine changes nothing and returns a request error that says which check failed.

## Concurrency requirements

Ideally, a single engine instance will support simultaneous operations and multiple frontends at the same time. However, this may rule out some desired implementations, such as one based on stdin/stdout. For this reason, some of the concurrency requirements below are nice to have rather than hard requirements.

**Table 11.** Concurrency requirements

| Requirement | Level | Supported |
| --- | --- | --- |
| Multiple engine instances run at the same time without conflicts | Required | YES |
| Multiple engine instances of the same host share the same engine folder | Nice to have | YES |
| One engine instance handles multiple sessions from the same frontend at the same time | Nice to have | NO |
| One engine instance serves sessions from different frontends | Nice to have | NO |
| One session runs multiple operations at the same time, e.g. simulate while building | Not required | NO |
| Concurrent sessions safely write to the same build folder | Not required | NO |

## Versioning

The engine, its packages and the catalog all use date versions. The canonical form is `yyyy-mm-dd`. The other forms below are exact conversions of it, used where a context requires them.

**Table 12.** Version formats

| Context | Form | Example | Second release on the same day |
| --- | --- | --- | --- |
| Canonical: docs, git tags, GitHub releases, version reports | `yyyy-mm-dd` | `2026-01-05` | `2026-01-05-2` |
| File names, such as package files | `yyyymmdd` | `oss-cad-suite-linux-x64-20260105.tgz` | `20260105-2` |
| Package registries, such as PyPI and npm | `yyyy.m.d`, without leading zeros | `2026.1.5` | `2026.1.5.2` |

- **Comparison.** Versions are always compared by their canonical date, never as strings in mixed forms.
- **Pinned packages.** Each engine release includes a manifest that names the exact date and checksum of every package it was tested with. By default, the engine installs exactly those, so a given engine version always produces the same toolchain.
- **Immutable releases.** Once a published engine release pins a package release, that package release never changes, and a fix gets a new date. During development, daily package releases may still be replaced, and development builds of the engine refresh their manifest to match.
- **Side-by-side packages.** The engine folder can hold several dates of the same package, so engine versions that share an engine folder do not overwrite each other. Unused dates can be removed.
- **Override for testing.** A host can request a different package date, such as a newer daily build. The engine reports that it is running an untested combination.
- **Catalog.** The catalog ships with each engine release and has the same version.
- **Tool API never breaks.** Changes only add tools or optional fields. A breaking change gets a new tool name, and the old tool keeps working until it is retired in a documented, announced way.
- **Version check.** A host asks the engine for its version and installed package dates when it starts. Each host declares the minimum engine version it needs and tells the user clearly when the engine is older.
- **No silent upgrades.** The engine never updates itself or its packages on its own. Upgrades happen only when a host asks.

## Packages

- **Source.** Packages are downloaded only from GitHub releases, identified by owner, repo and release tag. The engine's manifest lists the owner, repo, tag and checksum of each package file for each platform.
- **Verification.** After downloading a file, the engine checks it against the checksum in the manifest. If it does not match, the engine refuses to install it and reports a setup error.
- **Automatic install and repair.** Apio already manages its packages automatically for VS Code, Logix and IceStudio, and the engine must do the same. An operation that needs a missing or broken package installs or repairs it first, with progress, so hosts don't manage packages themselves. A launch option turns this off for offline or managed setups. The engine then reports a setup error instead. Since the manifest pins every package, packages change only when the engine version changes.
- **Licenses.** The tools in each package carry their own respective licenses.

## Error model

Every host must be able to tell what went wrong and whose problem it is: the user's design, the user's setup, the host or the engine.

A failing test or an unmet timing constraint is not an error. The operation did its job, and its result reports the failure as data. Errors are only for operations that could not do their job.

**Table 13.** Error categories

| Category | Meaning | Example | Typical host reaction |
| --- | --- | --- | --- |
| Design error | A problem in the user's design files | A syntax error in `main.v`, or an unknown pin name in the constraint file | Show the diagnostics at the file and line |
| Setup error | Something missing or wrong on the computer | A package is not installed, the board is not connected, or USB permission is denied | Tell the user what to fix, or offer to fix it, such as installing packages |
| Request error | The host sent an invalid request | An unknown board name, a source folder that does not exist, or a missing required field | A bug or bad configuration in the host. Report it to the user |
| Tool failure | A packaged tool crashed or misbehaved | nextpnr crashed | Show the tool's output and suggest a bug report |
| Engine error | A bug in the engine itself | An unexpected internal failure | Suggest a bug report with the error details |
| Cancelled | The host cancelled the operation | The user pressed Stop | Nothing to report |

Every error report includes:

- **Category,** from Table 13.
- **Message,** short and written for end users.
- **Details** for troubleshooting, such as the relevant tool output.
- **Diagnostics** with file, line and column, for design errors.
- **Suggested action,** when there is one, such as installing packages or connecting the board, with a machine-readable code so hosts can offer a one-click fix.

Messages never assume a particular host. For example, the engine says that a package is not installed, not which command to run, so each host can phrase the fix in its own terms.

## Device selection

Program board must find the right board when one or several devices are connected. The engine follows Apio's model, so every Apio board works unchanged.

For now, the engine supports only USB devices, not boards programmed over a serial port. This leaves out 11 Apio boards, most of them legacy: TinyFPGA, BlackIce, Olimex iCE40 EVB and Devantech boards. It is an exception to full Apio compatibility.

- **Board filters.** Each programming variant of a board defines a USB filter: vendor ID, product ID and a regular expression for the product name.
- **Programming variants.** Each board defines one or more named programming variants, such as flash and sram, or different cables, such as FT232 and FT2232, and marks one as its default. Each variant names the programmer to run, such as openFPGALoader, and gives its full argument template and the USB filter that finds the device. There are no separate programmer definitions, since in Apio the board already supplies almost all of the command. The template uses variables that the engine fills in from the selected device: vendor ID, product ID, bus, device number and serial number, and always the bitstream file.
- **Narrowing in the request.** A request can add a serial number to narrow the match.
- **Exactly one match.** The engine programs only when exactly one device matches. If none or several match, it returns a setup error that lists the candidate devices as data, so a host can let the user pick one and retry with its serial number.
- **Device scan.** An operation lists the connected USB devices, with vendor ID, product ID, product name and serial number.
- **Custom programmer command.** A request can carry a custom command template that replaces the default variant's, as Apio's `programmer-cmd` setting does.
- **Linux permissions.** The engine provides the udev rules that give users access to the devices. When a device is found but cannot be opened, the engine reports a setup error saying that permissions are missing.

## Custom definitions

The engine's catalog is read-only and fixed for each engine version. Custom boards and FPGAs belong to the host, like the rest of its project model.

- **Reading the catalog.** Hosts can read the full catalog through an engine request.
- **Merging is up to the host.** A host combines the catalog with its own local definitions, using its own rules. For example, Apio replaces catalog entries with project entries of the same ID and adds new ones.
- **ID or full definition.** Each request either names a catalog ID, such as a board ID, or carries the full definition to use. Hosts without custom definitions just send IDs.
- **Self-contained requests.** The engine never looks up or merges definitions from files. A request alone determines which board, FPGA and programming variant are used.
- **Errors.** The host reports problems in its own definition files, with file and line. The engine reports an invalid definition in a request as a request error.
- **Stable format.** The definition format is part of the tool API and follows the rule that it never breaks.
- **Catalog fixes.** A fix to the catalog needs an engine release. Until then, a host can send a corrected definition in its requests.

## Performance

CLI hosts start a new engine instance for every command, so the engine's own overhead must stay small next to the time the tools take. These are targets, to be confirmed by measurement.

- **Startup.** An instance is ready to accept requests within 200 ms of launch, excluding first-time package installation.
- **No network at startup.** Starting an instance never contacts the network. Network access happens only to install or repair packages that an operation needs, or when a host asks for it.
- **Verification on install.** Package checksums are verified when packages are installed, not every time an instance starts.
- **Operation overhead.** The engine adds no more than 100 ms to an operation beyond the time its tools take.
- **Up-to-date builds.** A build with no changes since the last build finishes within 1 second, using the incremental state in the build folder.

## Acceptance and phasing

The criteria and phases below are initial targets, not fixed commitments. They may be relaxed or changed as the project learns what matters in practice.

**Acceptance criteria.** The engine is ready to replace Apio's own toolchain handling when:

- **Apio parity:** Apio, implemented as a host on the engine, passes Apio's existing test suite, and Apio's commands work.
- **Same results:** for the golden test designs, the engine produces the same bitstreams as today's Apio with the same package versions. Where the tools are not fully deterministic, utilization and timing results match instead.
- **Real hardware:** board programming works on a reference board for each supported programmer, tested with the hardware-in-the-loop tests.
- **AI agent as host:** an AI coding agent, connected to the engine as a generic MCP client with no frontend, builds and programs a reference design on a real board, guided only by the tool descriptions and the engine's errors.
- **All platforms:** the test suite passes on every supported platform.
- **Performance:** the targets in the [Performance](#performance) section are met.

Apio's 109 boards split by architecture roughly as iCE40 about 42, ECP5 about 36, Xilinx 7 Series about 23 and Gowin about 9. This suggests the following phases.

**Table 14.** Proposed phases

| Phase | Scope | Done when |
| --- | --- | --- |
| Milestone 1. Proof of concept | A proof of concept of the engine, built on the real design so it evolves into the full engine rather than being thrown away: MCP interface, automatic package install and versioning, read-only catalog, error model, iCE40 only, and five tools: `get_engine_info`, `get_catalog`, `build`, `list_devices` and `program_board`. macOS only, with no Linux or Windows yet. GitHub workflows that build and test the engine. Developer scripts `run.sh`, `build.sh` and `clean.sh` to run, build and clean it locally. The underlying tools come from Apio's packages | An AI coding agent, such as Claude Code, is the host, connected to the engine over MCP, with no frontend. From prompts alone, it creates an iCE40 design, builds it and programs it on a real board |
| 2. Core coverage | ECP5 and Gowin, lint, report, simulate, test, format and graph, device scan and drivers, launching waveform viewers, a prototype Apio host | The prototype Apio host handles the common flows on all three architectures |
| 3. Apio parity | Xilinx 7 Series, the remaining programmers as decided in the open questions, the acceptance criteria above | Apio switches to the engine |
| 4. More hosts | IceStudio and Logix as hosts, with the requirements in Table 6 | Both run their core flows on the engine |
| Later | Network adapter, on-board verification, possibly VHDL | Decided when they come up |

Milestone 1 tests the interface with an AI coding agent as the host, before any frontend proof of concept is written. Proofs of concept of the reference hosts follow, starting with the prototype Apio host in phase 2. Full IceStudio and Logix support comes in phase 4.

## Development methodology

The engine will be implemented and maintained by human-guided AI. The human directs the work and judges the results, with a general understanding of the code and architecture but without reviewing every line. The code is therefore not required to be human-readable or intuitive. In general, the AI-generated code is treated like machine code: people are not expected to inspect or understand it. In contrast, the functionality, external interfaces and requirements must be straightforward, intuitive and easy for people to understand.

**Table 15.** Development roles

| Role | Responsibilities |
| --- | --- |
| Human | Provides instructions, tests the results and gives feedback to the AI |
| AI | Writes and maintains the code, the tests and the documentation |

The project relies heavily on AI-generated tests to catch regressions, since humans do not review every line of code.

## Project infrastructure

- **Source.** The engine's code lives in a GitHub repo, with issues and pull requests on GitHub.
- **Testing.** GitHub Actions workflows run the engine's test suite on every supported platform for every change, as Apio does today.
- **Releases.** Workflows build the per-platform engine binaries and publish them as GitHub releases with date tags, like the package repos' daily releases.
- **Hardware tests.** GitHub-hosted runners cannot reach FPGA boards, so the hardware-in-the-loop tests run separately on a lab computer with boards attached. Whether that computer runs as a self-hosted GitHub runner is still open.

## Open questions

- Whether to make openFPGALoader the only programmer. It can likely program about 93 of Apio's 109 boards, which would simplify packages and testing. The other 16, including the serial port boards and the iCESugar boards, would no longer be supported, and each switched board must be tested on hardware.
- Whether to support Linux arm64 and single-board computers attached to the target board, such as a Raspberry Pi.
- Which hardware test interface to use for the engine's own hardware tests, and whether the same interface can serve the on-board verification feature for user designs.
- Whether the network adapter is part of the first release.
- The license of the engine's own code. The recommendation is Apache-2.0, which is permissive and includes an explicit patent grant. Alternatives include GPL, to match Apio or to port Apio code, and 0BSD or MIT-0 for minimal conditions. The packaged tools keep their own licenses in any case.
