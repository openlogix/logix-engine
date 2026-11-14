# Requirements

Draft requirements for the engine. [Architecture](architecture.md), [Tool API](tool-api.md) and [Command line](command-line.md) hold the proposed design.

## Terminology

**Table 1.** Terms used in this document

| Term | Definition |
| --- | --- |
| Engine | Short for Logix Engine. The single component this document specifies. It performs FPGA operations on behalf of its hosts |
| Host | The program that launches the engine as a child process on the same computer, shares folders with it and talks to it over MCP. A host is either a local frontend or the network adapter |
| Frontend | A program or workflow that people use to do FPGA work. Examples include IDEs, CLI tools, AI agents, browser apps, and test or automation scripts and workflows. Most frontends are also hosts, but remote frontends, such as browser apps, are not: they reach the engine through the network adapter, which is the host |
| Network adapter | An optional host that relays requests from remote frontends, such as browser apps, to the engine. It typically handles auth and file transfer |
| Operation | A unit of functionality the engine offers, such as build, simulate or program board |
| Instance | One running engine process, from launch to exit. With stdin/stdout, each instance serves exactly one session |
| Session | One connection between a host and the engine, from start to close. The host decides how long it lasts |
| Catalog | The engine's collection of known boards, with their board variants, and FPGAs, kept as data. The engine downloads it as the `logix-catalog` package, the release its remote config names. Hosts can extend or override it at runtime by providing custom definitions, which the engine then uses for builds, programming and other operations |
| Board | A product the user picks by its name. Different names are different boards, unrelated even when their hardware is identical. See [Boards and board variants](#boards-and-board-variants) |
| Board variant | One way to program a board, such as through its USB port to flash or through an FT232H adapter to RAM: its selector (a value of each standard dimension) and how to program it. A board lists its board variants; requests select one by its selector, in the `board_variant` field |
| Toolchain | The open source FPGA tools the engine runs to perform operations, such as synthesis and place-and-route tools |
| Shared folders | Folders on the host's computer that both the host and the engine use: the engine folder, the source folder and the build folder. The host chooses their locations: the engine folder when it launches an instance, the source and build folders with each request. They may overlap |
| Source folder | The shared folder that holds the design's sources, such as HDL files, testbenches and constraint files. The host owns it and names it in each request. The engine reads it, and writes to it only when an operation calls for it, such as formatting, or writing a default waveform layout the request names. It may also hold other files, such as a frontend's project files. The host can exclude parts of it from the engine, such as a `_build` folder, even when the build folder is only a subfolder of the excluded part, such as `_build/<env_name>` |
| Build folder | The shared folder where the engine writes generated files, such as bitstreams, reports and waveforms, and keeps any state between requests, such as incremental build data. The host names it in each request. Deleting it loses no data: the next operation that writes to it creates it again, maybe more slowly, with the same result. It may be inside the source folder or outside it, but is never the source folder itself |
| Project model | How a frontend organizes a design on disk, such as a project file and directory layout. The engine does not impose one and needs no project file. For AI sessions with no other requirements, it suggests a simple default (docs/tool-api.md, Default project model), which isn't meant for frontends and hosts |
| MCP | Model Context Protocol, the open standard all hosts use to call engine operations as tools |

## Goals

An open source, low-level FPGA engine that makes it easy to build a wide range of frontends for FPGA development and testing. These are the tools developers use across the whole cycle, from writing and simulating a design to programming a board and verifying it on real hardware. The engine handles the underlying operations, such as synthesis, simulation, testing, discovery of connected FPGA boards, board programming, automated on-board verification, and more. That way, each frontend can focus on its own user experience instead of reimplementing the toolchain.

**HDL languages.** The engine supports Verilog and SystemVerilog, from the first release on.

**Table 2.** Frontend types

| Frontend type | Examples |
| --- | --- |
| Graphical tools | Logix Studio, IceStudio |
| Command-line tools | Logix CLI, Apio |
| Automation | Scripts and workflows |
| IDE extensions | Logix VSCode, Apio VS Code |
| Standalone IDEs | Logix IDE |
| Browser-based IDEs and playgrounds | None yet |
| AI coding assistants | Logix AI, MCP-based agents |
| HDL frameworks | Amaranth, LiteX, SpinalHDL, Chisel |
| Education platforms and remote labs | Logix Remote |

## Non-goals

The engine is not:

- **An IDE or editor.** No editing, code navigation or live checking as the user types. Hosts do that, and can run packaged tools such as the Verible language server themselves.
- **A user interface.** No windows or dialogs. The one exception is launching a packaged viewer, such as GTKWave, locally when a host asks.
- **A project manager.** No project files, settings, environments or examples. The host owns its project model.
- **A design-format converter.** Block diagrams, Amaranth, LiteX and other formats are turned into Verilog or SystemVerilog by the host before they reach the engine.
- **A network server.** Remote access goes only through the separate, optional network adapter.
- **A new toolchain.** The engine does not implement synthesis, place and route or programming itself. It downloads and drives existing open source tools.
- **A front for proprietary vendor tools.** Tools such as Vivado, Quartus or Gowin's own IDE are out of scope. The engine uses open source toolchains only.
- **Self-updating.** The engine never updates itself. Its packages change only to what its remote config names (see No unreported upgrades, under Versioning).

## Frontend deployment forms

The engine must support frontends in each of the deployment forms below. Frontends can be written in any language, so the engine must not require a specific programming language. How the engine itself is deployed (library, process, service or other) is an architecture decision, as long as it supports every form below.

**Table 3.** Frontend deployment forms

| Frontend form | Examples | Implementation example |
| --- | --- | --- |
| Desktop graphical design apps | Logix Studio, IceStudio | NW.js, Electron or Qt app |
| Desktop standalone IDEs | Logix IDE |  |
| Command-line tools | Logix CLI, Apio | Python process |
| Scripts and automated workflows | CI runners, containers | Shell scripts, GitHub Actions |
| IDE extensions | Logix VSCode, Apio VS Code | TypeScript in the VS Code extension host |
| Programs that embed the engine | Amaranth, LiteX | Python library calls |
| Browser-based apps | Served from a remote server, or fully in-browser | JavaScript web page |
| AI coding assistants and agents | Logix AI | LLM agent making tool calls |
| Remote frontends | Logix Remote, remote labs | Laptop app talking to a lab computer |

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
| Windows | x86-64 |

## Engine functionality

The engine provides the core FPGA operations that frontends build on. The table below lists typical examples of this core functionality. It is not a complete list, and more operations will be added as the project evolves.

**Table 5.** Examples of engine functionality

| Operation | Description |
| --- | --- |
| Build | Synthesize, place and route, and generate the bitstream |
| Verify | Check the HDL source for errors and warnings |
| Format | Format the HDL source code |
| Diagram | Draw the design as schematic diagrams, one per module, with a page that shows them all |
| Report | Report resource utilization and timing |
| Simulate | Run a testbench and show the waveforms |
| Test | Run testbenches automatically and report pass or fail |
| Program board | Program the bitstream to the board |
| Devices and drivers | Manage drivers and devices that are required to communicate with the FPGA boards. |
| Catalog | Provide and manage a standard catalog of known FPGAs and boards, though operations will not be restricted to those items. |

## Reference hosts

Open Logix's own frontends, starting with Logix IDE, and other open source FPGA tools, such as Apio and IceStudio, must each be implementable as a host of the engine, using the engine's operations instead of their own toolchain handling. These reference hosts validate the engine's interface. The Required capabilities section lists what the engine offers them all; Table 6 lists what graphical and IDE hosts need beyond that.

**Table 6.** Requirements from graphical and IDE hosts

| Area | The engine must support | Needed by |
| --- | --- | --- |
| Structured diagnostics | Errors and warnings as data, with file, line, column and severity, so a host can highlight them in its editor or on its graphical blocks | IceStudio, Logix IDE |
| Progress and cancellation | Reporting progress during long operations, and stopping an operation when the host cancels it | IceStudio, Logix IDE |
| Results as data | Resource utilization, timing and test results as structured data, not only as text reports | IceStudio, Logix IDE |
| Board pinouts | Each board's pins and their names in the catalog, so a graphical host can let users pick pins | IceStudio |
| Setup from a GUI | Downloading and updating toolchain packages, and installing drivers, with progress reporting, without a terminal | IceStudio, Logix IDE |
| Bundled engine | A host shipping a specific engine version, with its own engine folder | IceStudio, Logix IDE |
| Packaged tools for hosts | Letting local hosts run packaged tools themselves, such as the Verible language server for live checks in an editor. An engine request returns the tool's command and environment, so hosts never depend on the engine folder's layout | Logix IDE |
| Parallel work | Running a check while a build is in progress, met by the host running a second engine instance | Logix IDE (to confirm) |

**AI agents as hosts.** A generic MCP client, such as an AI coding agent, must be able to act as the host itself, with no frontend. It launches the engine with an engine folder and needs nothing else. It learns the tools from their names, descriptions and input schemas alone, and acts on the structured errors and suggested actions. This means every tool and field needs a clear, self-contained description.

## Required capabilities

The engine must support at least the capabilities below, which today's open source FPGA tools offer their users, so that any of them can be built as a host on top of the engine.

**Table 7.** Required capabilities

| Area | The engine must support |
| --- | --- |
| FPGA families | The families of the supported boards' FPGAs, the names on the chips: iCE40, ECP5, GW1N, GW2A, GW5A, Artix-7, Spartan-7 and Zynq-7000. The engine maps each to one of its four toolchains (`ice40`, `ecp5`, `gowin`, `openxc7`), a name for logs, diagnostics and the API, never the main word people see |
| HDL languages | Verilog and SystemVerilog |
| Constraint files | Each family's native format, such as `.pcf` for iCE40, `.lpf` for ECP5 and `.cst` for Gowin |
| Build toolchains | Synthesis, place and route, and bitstream generation for each family, with open source tools such as Yosys, nextpnr, IceStorm, Project Trellis and Apicula |
| Programmers | The programmers of the supported boards, today openFPGALoader, iceprog, dfu-util and icesprog, including board-specific programmer options. Boards whose programmers are legacy are dropped, with the reason recorded (see Milestone 2) |
| Simulation and testing | Running testbenches and producing waveform files. When a local frontend asks, launching a packaged waveform viewer, such as GTKWave, fully detached so it outlives the session. The request is refused for remote frontends, which view waveforms on their own |
| Lint and format | Checking and formatting HDL source, with Verilator and Verible |
| Reports and graphs | Resource utilization and timing reports, and design graphs in SVG, PNG and PDF |
| Catalog | Boards and FPGAs, with each board's variants. A project's custom definitions are sent by the host in requests, as described in [Custom definitions](#custom-definitions) |
| Devices and drivers | Listing USB devices, and installing or removing the drivers boards need, such as FTDI drivers on Windows |
| Packages | Downloading, updating, listing and repairing toolchain packages for each platform |
| Examples | Not an engine feature. An example is a project, and the project model belongs to the frontend: the engine has none (see [Project model](#project-model)). Each frontend defines its own project model and source format, such as IceStudio's block diagrams, and provides examples in it. |
| Raw tool access | Running any packaged tool directly, with the right environment, using the same engine request that returns a tool's command and environment |
| Machine-readable info | Versions, platform, catalog and package details in structured form |

Project-level settings, such as a frontend's project files and environments, stay in the frontend, as described in the [project model](#project-model). The frontend passes the selected board and options with each request.

## User design testing

The engine helps users test their own designs, from source checks to verification on real hardware. Results come back in structured form, with pass or fail per test, messages and the paths of any generated files, so that hosts such as CI pipelines and AI agents can act on them.

**Table 8.** Types of user design testing

| Test type | What the engine does | Status |
| --- | --- | --- |
| Lint | Checks the HDL source for errors and warnings | Required |
| Simulation | Runs a testbench and writes waveform files to the build folder. A local frontend can ask the engine to open them in a waveform viewer | Required |
| Automated testbenches | Runs a set of testbenches without user interaction and reports pass or fail for each | Required |
| On-board verification | Programs a real board, drives its inputs and checks its outputs automatically, then reports pass or fail | Goal. The hardware interface and approach are still open |

## Testbenches

The engine applies these testbench rules, so a testbench works unchanged with any build folder.

- **Naming.** A testbench file name ends with `_tb.v` or `_tb.sv`. This is how the engine tells testbenches from design sources and keeps them out of synthesis.
- **No `$dumpfile`.** A testbench that calls `$dumpfile(...)` is a design error. The engine sets the waveform file's location itself, always inside the build folder. Testbenches still call `$dumpvars` to choose what to record.
- **Interactive define.** The engine defines a macro that is 1 when the simulation is interactive, with a waveform viewer, and 0 for automated test runs. It is `LOGIX_SIM`.
- **User defines.** Defines given in the request apply to simulation as well as to build.
- **Waveform viewer setup.** A viewer save file, such as a GTKWave `.gtkw` file, that the user saved next to the testbench is only read, never changed. When there is none, the engine generates one in the build folder, so the source folder isn't modified.

## Project model

The engine does not impose a specific project model. Each frontend defines its own, for example Logix IDE with its `logix-ide.toml` file and project folder. The frontend translates its project into explicit inputs for each engine operation, such as source files, top module, board and options.

## Interaction with the host

Hosts interact with the engine in the following ways.

**Table 9.** Interaction with hosts

| Aspect | Description |
| --- | --- |
| Requests and responses | The host sends requests and receives structured responses over MCP, which uses JSON |
| Progress and logs | During long operations, the engine reports progress, output and errors as they happen. In particular, the output of the tools it runs reaches the host line by line while the tools run, so a host can show a live log, such as in Logix IDE's console, instead of a dump at the end of each operation |
| Shared files | The host owns the source files and the engine reads them. The engine writes generated artifacts, such as bitstreams, reports and waveforms, to the build folder the host specifies |
| Deployment | The host's deployment includes the engine, so users install only the host |
| Local engine (a recommendation for hosts) | By default a host runs only the GitHub package of the engine release it pins, or, with remote updates, the one its remote config names. For Open Logix developers, users they guide, and manual and automated testing of Open Logix components, Open Logix's frontends follow one convention to run another engine binary, such as one built from a local clone: the environment variable `LOGIX_DEV_ENGINE_BINARY`, the path to a `logix-engine` executable, honored in every build, with its own engine folder, shown for the whole session, and an error for a value that isn't an executable file. The engine doesn't read it; the full rule is in the website repo's `CLAUDE.md` (shared Open Logix rules) |

## Interaction with the file system

The engine works with three shared folders. The engine has no hard-wired disk locations. All sources reach the engine through the shared folders: requests never carry source text. The host sets the engine folder when it launches each instance. Each request carries the source and build folders it uses. Any state the engine keeps between requests lives in the build folder. Restarting the engine may make the next build slower, but never changes its result.

**Build folders are disposable.** A host or a person can delete a build folder whenever no operation is using it, and no data is lost:

- It holds only files the engine generates from the sources and the request. The engine never keeps the only copy of anything in it.
- Every operation that writes to a build folder creates it again, such as `build`, `test`, `simulate` and `diagram`. The next build may be slower, but its result is the same.
- An operation that only reads a build's outputs, such as `program_board`, can't work until the next build. It returns a request error that says to build first.
- Exceptions are files that a host or a person puts into a build folder, and a GTKWave layout saved from a `simulate` request without `layout_file`, which goes into the build folder. A host that keeps layouts names a `layout_file` in the source folder.

**Table 10.** Shared folders

| Folder | Access | Contents |
| --- | --- | --- |
| Engine folder | Read and write | Files the engine downloads and maintains, such as toolchain tools and packages |
| Source folder | Read and write | The design's HDL sources and testbenches, e.g. read for builds and written when formatting |
| Build folder | Read and write | Generated files such as bitstreams, reports, waveforms and intermediate build files |

Note: the folders may overlap. This allows project models where the build folder is inside the source folder, and the source folder also holds other files, such as a frontend's project file. However, the engine does not require this or any other project model.

**Engine folder location.** Each host keeps its own engine folder inside its own home folder, for example `~/.logix/ide/engine` (Logix IDE: each Open Logix program has its own folder in `~/.logix`) or `~/.myhost/engine`, and passes it at launch. Users deal only with the host and do not need to know about the engine. Hosts do not share engine folders by default. This keeps them independent, which matters because different hosts are likely to be pinned to different engine versions.

## Safe file changes

The engine guards every change it makes on disk, including creating, writing, renaming and deleting files and folders, so that a wrong path never damages the user's files.

- **Path keywords.** The engine checks for keywords in folder paths before changing anything. The last folder name of an engine folder path must contain "engine", such as `~/.logix/ide/engine`. One of the last two folder names of a build folder path must contain "build", such as `_build` or `_build/icesugar-1-5`. Checking only the last names keeps a parent folder, such as `/home/builder`, from satisfying the rule by accident. A wrong path from a host, such as the home folder, fails the check, so nothing is changed.
- **Host naming.** Hosts must name their engine folders and build folders to satisfy the keyword rules, such as `~/.logix/ide/engine` and `_build`.
- **Engine-owned folders only.** The engine changes files only inside the engine folder and build folders. The exception is the source folder, where it may rewrite in place the source files named in a request when an operation calls for it, such as formatting. It may also create a generated project file the request names, such as a default GTKWave layout for `simulate`, and replace or delete it later only while it still carries the engine's or another tool's generated-file marker, so files the user saved are never changed. It never renames or deletes any other file in the source folder.
- **Paths stay inside.** Every path is resolved to its real location, following symbolic links, before a change. A path that resolves outside its folder is refused, so a symbolic link cannot lead the engine into the user's other files.
- **Forbidden locations.** The engine refuses to use the file system root, the user's home folder or the source folder itself as an engine folder or build folder. A build folder inside the source folder stays allowed, such as a `_build` folder.
- **Refusal, not a guess.** If any check fails, the engine changes nothing and returns a request error that says which check failed.

## File names in messages

Messages point people at the file to fix, named the way they know it. The engine doesn't know how a host keeps a user's files: a host may hand the engine copies, such as unsaved editor text written to a scratch folder or an example a new project starts from, whose paths mean nothing to the user. So the host decides how files are named, and the engine applies it:

- **The request says how to name paths.** Every tool that reports files takes optional display paths: files, each named as a whole, and folders, whose files keep their path below the folder under the folder's new name, which may be empty (names relative to the folder) or any text. File entries come before folder entries, and within each, the first that matches wins. Matching is by real location, not by spelling: different spellings of the same file or folder, such as through a symbolic link, match.
- **Everywhere a person reads a path.** Diagnostics, error messages, testbench output, the live tool output and the diagram's labels.
- **Never where a program needs the real path.** The rerunnable command lines in results and paths given as fields, such as a bitstream or a waveform, stay real.
- **Default: as the request gave it.** A path that matches no entry, or any path when the request has none, is shown as the request gave it: source files relative to the source folder, custom board files by the folder's absolute path plus their names. A path the request never gave, such as a toolchain file a tool mentions, is shown as the tool printed it.

The engine stays general: it knows nothing of projects, examples or scratch copies, only the names the request gives. A host that hands the engine the user's own files needs no display paths.

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

The engine, its packages and the catalog all use date versions, as every Open Logix component does. Each form is an exact conversion of the release date, used where a context requires it. There is at most one release a day.

**Table 12.** Version formats

| Context | Form | Example |
| --- | --- | --- |
| Tags: git tags, GitHub releases, pins | `yyyy-mm-dd` | `2026-01-05` |
| File names, such as package files | `yyyymmdd` | `logix-core-linux-x86-64-20260105.tgz` |
| The version people see: version reports, docs | `yyyy.mm.dd` | `2026.01.05` |
| Only where a tool requires semver, such as Cargo | `yyyy.m.d`, without leading zeros | `2026.1.5` |

- **Comparison.** Versions are always compared by their canonical date, never as strings in mixed forms.
- **Pinned packages, by the remote config** (Remote updates, the website repo's `CLAUDE.md`, Remote updates; decided 2026-11-12). The engine's remote config, the version folder `remote-config/logix-engine/<NNN>/` in this repo, names the release of every package, per release channel (`channels.jsonc` lists which engine releases are `prod`; the rest are `dev`; a channel without its own config file uses `prod.jsonc`); the engine reads it from GitHub by the config version it hard-codes (`001`). There's no built-in list, as in Apio: the repo's files are the pin lists, and the tests run once per config file there. A change to a file reaches every engine on that config version and channel without an engine release. A `prod.jsonc` names only stable releases; a `dev.jsonc` may name pre-releases, which are kept while a config names them.
- **One release of each package.** Once a new release is downloaded and in place, the old one is deleted. Hosts don't share engine folders, so no other engine needs it.
- **Override for testing.** `LOGIX_ENGINE_DEV_REMOTE_CONFIG` names another remote config to read, a URL or a local file, such as a pull request's version. A bad value is an error, never a fallback.
- **Catalog.** The catalog is a package like the others, with its own date, named by the remote config.
- **Tool API never breaks.** Changes only add tools or optional fields. A breaking change gets a new tool name, and the old tool keeps working until it is retired in a documented, announced way.
- **Version check.** A host asks the engine for its version and downloaded package dates when it starts. Each host declares the minimum engine version it needs and tells the user clearly when the engine is older.
- **No unreported upgrades.** The engine never updates itself. Its packages change only to what its remote config names: before an operation, when its cached remote config is more than 24 hours old, or when a host asks (`check_updates`). Every update is reported, in the operation's progress (the release it downloads), the log, and `check_updates`'s result. The launch option that turns off automatic download turns this off too.

## Packages

- **Two packages.** `logix-catalog`, the board catalog (one file for every platform), and `logix-core`, the FPGA tools (one file per platform), both built by Open Logix from pinned upstream releases. The catalog is small and comes first, so hosts can list boards while the tools download.
- **A checked configuration.** Each engine release pins one `logix-catalog` and one `logix-core`, and its tests check that they fit: every FPGA in the catalog must be in the list of FPGAs the core's tools can build, which the core generates from their databases (`logix-core.json` `fpgas`). A core that drops or renames an FPGA, or a catalog that adds one the tools can't build, fails the tests before it's pinned (decided 2026-11-10). With the remote config, a change to it is tested the same way before it merges: the tests read the repo's config, so the catalog and core it names must fit, and the engine's tests pass with them.
- **Source.** Packages are downloaded only from GitHub releases, identified by owner, repo and release tag, as the remote config names them, with each file's SHA-256 from its release's `SHA256SUMS`.
- **Verification.** After downloading a file, the engine checks it against that SHA-256. If it does not match, the engine refuses to use it and reports a setup error. A receipt in each package folder records what was downloaded (repo, release, file, URL, platform, size, SHA-256, the engine that downloaded it, and the layout); a package whose receipt doesn't match the config's release, URL, platform, SHA-256 or layout is downloaded again, which also catches a release rebuilt under the same tag.
- **Automatic download and repair.** An operation that needs a missing or broken package downloads or repairs it first, with progress, so hosts don't manage packages themselves. A launch option turns this off for offline or managed setups. The engine then reports a setup error instead. Packages change when the engine's remote config does.
- **Licenses.** The tools in each package carry their own respective licenses.

## Error model

Every host must be able to tell what went wrong and whose problem it is: the user's design, the user's setup, the host or the engine.

A failing test or an unmet timing constraint is not an error. The operation did its job, and its result reports the failure as data. Errors are only for operations that could not do their job.

**Table 13.** Error categories

| Category | Meaning | Example | Typical host reaction |
| --- | --- | --- | --- |
| Design error | A problem in the user's design files | A syntax error in `main.v`, or an unknown pin name in the constraint file | Show the diagnostics at the file and line |
| Setup error | Something missing or wrong on the computer | A package is not downloaded, the board is not connected, or USB permission is denied | Tell the user what to fix, or offer to fix it, such as downloading packages |
| Request error | The host sent an invalid request | An unknown board name, a source folder that does not exist, or a missing required field | A bug or bad configuration in the host. Report it to the user |
| Tool failure | A packaged tool crashed or misbehaved | nextpnr crashed | Show the tool's output and suggest a bug report |
| Engine error | A bug in the engine itself | An unexpected internal failure | Suggest a bug report with the error details |
| Cancelled | The host cancelled the operation | The user pressed Stop | Nothing to report |

Every error report includes:

- **Category,** from Table 13.
- **Message,** short and written for end users.
- **Details** for troubleshooting, such as the relevant tool output.
- **Diagnostics** with file, line and column, for design errors.
- **Suggested action,** when there is one, such as downloading packages or connecting the board, with a machine-readable code so hosts can offer a one-click fix.

Messages never assume a particular host. For example, the engine says that a package is not downloaded, not which command to run, so each host can phrase the fix in its own terms.

## Boards and board variants

A board that can be programmed in several ways, such as through a cable or to flash or RAM, is one board with board variants, not several boards. The format is in the [tool API](tool-api.md) (variants, variant).

- **A board is a name.** The board's name is how users select it. Two boards with different names are different boards, unrelated even when their hardware is identical, such as Alchitry Au and Au+, or ColorLight i5 and i9. Board variants exist only within one name.
- **Sizes are boards.** A board sold with several FPGA sizes, such as the Arty A7-35T and A7-100T or the ULX3S 12K to 85K, is one board per size: vendors name the sizes, and a larger FPGA in the same package can have pins a smaller one lacks (ECP5 in the 381-ball package has 197 I/O pins on the 12F and 205 on the 85F), so a pin file isn't guaranteed to work on every size.
- **What a board variant may change.** Only how the board is programmed, such as the connection or the memory. Every board variant of a board has the same FPGA and the same pin file, so the same project, its sources and pin file, works unchanged with every board variant.
- **Same pin file.** All board variants of a board must use the same pin file: every LED, button, clock and connector is on the same FPGA pin. Versions of a board whose LED, or any other signal, is on a different pin are different boards, each with its own name, even when everything else is the same. So is a mode that needs a different design, such as logic the design must include.
- **Standard dimensions.** Every board has the same two dimensions, with meanings the engine defines, so hosts can rely on them and decide how to show them (for example, a choice only where a dimension has more than one value, and a single value as a fact, such as "uploads to RAM"):
  - **`connection`**, how the computer reaches the board: `usb` (the board's own USB port, through the programmer chip on the board), `bootloader` (the board's USB port, with the board in its bootloader mode), an external JTAG adapter (`ft2232h`, `ft232h`, `cmsis-dap`, `usb-blaster`), or `unspecified` when the catalog doesn't know (shown as "USB", with help pointing to the board's documentation).
  - **`memory`**, where the design goes: `flash` (kept when the board is powered off) or `ram` (faster to upload; lost when the board is powered off). It follows from the command: openFPGALoader writes RAM unless told to write flash (`-f`, `--write-flash`), except on iCE40, which it always programs through flash; iceprog, icesprog and the DFU bootloaders write flash.
- **Labels and help come from the engine** (`STANDARD_DIMENSIONS` in `src/catalog.rs`), so a value means the same on every board. Values are named by what the user plugs in or chooses, in plain words: "FT232H adapter" (the USB adapter around the FTDI chip, connected to the board's JTAG pins), "CMSIS-DAP adapter", "USB-Blaster" (a product name), "USB bootloader", not the programmer's terms (openFPGALoader calls adapters cables, which users may not recognize). The catalog file lists only each board variant's values; `get_catalog` adds the board's `dimensions`, each standard dimension with the values its board variants use, labels and help included. New values are added to the engine's list, never invented by a board.
- **Drivers** (planned, for Windows and Linux). Which driver a user needs depends mostly on the connection and the OS: FTDI chips need a WinUSB driver on Windows and a udev rule on Linux, CMSIS-DAP usually nothing on Windows, DFU bootloaders WinUSB, macOS nothing. This will be engine data on each connection value (per OS), with the USB IDs for udev rules from the board variant's USB filter, and an override on a board variant for a board that needs something special. `unspecified` connections have none. Managing them is a goal of Milestones 3 and 4 (Acceptance and phasing), and the engine's job (decided 2026-11-13: Engine first, since every frontend needs it): detect when one is needed, install it as automatically and safely as possible, and uninstall it on request; a host only asks the user and calls the engine.
- **Board variants.** A board's `variants` list every way it can be programmed. Each has a `selector`, a value of both standard dimensions, such as `{"connection": "ft232h", "memory": "ram"}`, and everything needed to program the board that way: the programmer, its argument template and USB filter. A board variant has no ID of its own: its selector names it (decided 2026-11-06, replacing IDs such as `ft232h-ram`, which repeated the two values in a string and ranked one dimension before the other; a map keyed by the values was rejected because JSON keys are strings, and nested maps would make one dimension the outer one). Combinations that aren't listed don't exist, so there are no compatibility rules. A board variant also has `tested`, optional `user_notes` and optional `maintainer_notes` (never required, the user's decision, 2026-11-07). One concept, decided 2026-11-04: board variants replaced the earlier pair of variant values and configurations, for a clearer model.
- **Defaults and names.** The first board variant is the board's default. A board that can upload to flash has a flash board variant first (decided 2026-11-04: the design then stays when the board is powered off, as users expect; RAM is the faster choice for those who pick it); among those, the first is the usual connection. A catalog test checks it. Board variants have no names or descriptions of their own: hosts show them by their values' labels, such as "ButterStick r1.0, FT232H adapter, RAM". Messages name only the dimensions a board has a choice in ("Icesugar Pro (RAM)", but just "Alhambra II").
- **Catalog checks.** A test checks every board: each board variant's selector has a known value of both standard dimensions, no two board variants have the same selector, there is at least one board variant, and the memory matches the command.
- **Requests.** Tools that take a board also take an optional `board_variant`, the selector of one of its board variants, such as `{"connection": "ft232h", "memory": "ram"}`; without one the engine uses the default. A selector the board has no board variant for is a request error that lists its selectors. The build record and `list_devices` name board variants by selector too.
- **Builds.** The build record keeps the board and its variant. Builds and uploads compare only what the bitstream depends on, the FPGA's settings for the build tools (decided 2026-11-06; the field list is in docs/tool-api.md, What counts as the same board): another board variant, such as RAM instead of flash, or a changed name, note or programmer command, doesn't rebuild, and the build record takes the request's board. `program_board` looks the build's board and variant up again in its request and programs with their current definition. `program_board` checks the chosen device against that board variant's USB filter, and its messages name the board.
- **Wrong size.** Before writing over JTAG, the engine reads the connected FPGA's ID code (openFPGALoader `--detect`, with only the board variant's connection options, so it can't write) and refuses a different FPGA, such as an Arty A7-100T connected while the build is for the 35T, with a plain error naming both and suggesting the boards with the connected FPGA that the device most likely is (the strongest match level, as in the device scan). Each FPGA in the catalog has its `jtag_idcode`, taken from openFPGALoader's own list (`part.hpp`, version 1.2.0, the version in the toolchain package) by part number, never by size; a test checks that every JTAG board has one. Not checked: iCE40 boards (programmed through their flash, so the programmer never reads the FPGA's ID), other programmers, custom commands, and 21 FPGAs openFPGALoader doesn't list (Zynq XC7Z035, and three GW1N-9 entries whose die version is unclear), none of them on a catalog board. The ID code identifies the chip, not the size a board uses it as: two FPGA entries made from the same chip share it, so a wrongly declared size of the same chip can't be caught.
- **The catalog.** 81 boards, 128 board variants. By connection: 73 `usb`, 9 `bootloader`, 42 external adapters, 4 `unspecified` (Mimas ECP5 Mini, MicroPhase A7-Lite, each in flash and RAM); by memory: 78 flash, 50 RAM. The original commands of all Xilinx boards, most ECP5 boards and the Tang Nano upload to RAM; those boards also have a flash board variant (added 2026-11-04, untested: the RAM command plus `--write-flash`; on Xilinx openFPGALoader first loads its SPI-over-JTAG bridge for the FPGA part, so a command without `--board` names the part with `--fpga-part`; ColorLight boards add `--unprotect-flash`, as the i5's does, and a user note that the flash's factory firmware is replaced), and flash is now their default (see Defaults and names). Still RAM only: the Zynq boards (PYNQ-Z1, PYNQ-Z2, ZedBoard, Zybo Z7; openFPGALoader can't write their flash, which only the processor side reaches), the QMTECH XC7A100T (no bridge bitstream for its xc7a100tfgg676 package in oss-cad-suite) and the ULX3S (an open question: some have a DFU bootloader at the start of the flash, which a JTAG flash write would overwrite, as on the ButterStick). Boards with a choice (32): iCESugar-Pro, ColorLight i5 and the boards above with a flash board variant (memory: flash, RAM); ColorLight 5A-75E V7.1, ColorLight i9 v7.2 and FleaFPGA Ohm (connection: their JTAG adapters, each in flash and RAM); ButterStick r1.0, with two dimensions (connection: USB bootloader, FT2232H or FT232H adapter; memory: the bootloader writes flash and the adapters RAM, as their original commands do; a flash write over an adapter isn't offered, since it could overwrite the bootloader at the start of the flash); every other board has one board variant. The Fomu's memory is flash (its bootloader stores the design there) but whether the design comes back at power-up is to check on hardware.
- **Examples.** Examples belong to frontends and to a board, so they are the same for every board variant.

## Device selection

The engine identifies the connected devices and the boards they may be; the host chooses the device, and the engine checks the choice and programs it. Every frontend gets the same matching and the same behavior, and none needs to know about USB filters.

For now, the engine supports only USB devices, not boards programmed over a serial port. This leaves out 11 boards, most of them legacy: TinyFPGA, BlackIce, Olimex iCE40 EVB and Devantech boards.

- **Board filters.** Each board variant of a board defines a USB filter: vendor ID, product ID, and optionally a regular expression for the product name, one for the serial number, and the device's release number (USB `bcdDevice`), which on FTDI chips and the debuggers that act like them tells the chip type. A product name in a catalog filter comes from evidence, recorded in the variant's `maintainer_notes`: the board maker's own programming tool or files, the board's programmer (such as fujprog for the ULX3S) or published device listings, never from one sample. Where a board may ship with its USB chip's factory name, the filter accepts that name too, so the board still matches, as a possible match.
- **Board variants.** Each board defines one or more board variants, such as flash or RAM, or a particular adapter; the first is its default (see [Boards and board variants](#boards-and-board-variants)). Each board variant names the programmer to run, such as openFPGALoader, and gives its full argument template and the USB filter that finds the device. There are no separate programmer definitions, since the board variant already supplies almost all of the command. The template uses variables that the engine fills in from the selected device: vendor ID, product ID, bus, device number and serial number, and always the bitstream file.
- **Device scan.** An operation lists the connected USB devices, with vendor ID, product ID, manufacturer, product name and serial number, which are enough for a user to report a device whose filter needs fixing.
- **Boards per device.** Each listed device names the boards whose filters it matches, strongest first, and for each the board variants it fits, each with a match level. No filter proves which board is connected, since boards the catalog doesn't know can look the same, so these are suggestions, and hosts show them as such.
- **Match levels.** Each board variant a device fits has a level that says what the match means for the user, not why it matched: `strong` (the device is very likely this board), `possible` (it could be, but a few other boards look the same) or `weak` (it fits, but so do many boards). Hosts show the level and may compare levels (weak < possible < strong); they never infer the cause. Boards have no level of their own, so hosts use the board variant they mean. Decided 2026-11-05, replacing a confidence from 1 to 3 that the Logix IDE had started to read as causes: hosts need words they can show, not a number to interpret.
- **The criteria are internal.** How the engine decides a level is an implementation detail: it may change in any engine version without notice, and the tool API doesn't describe it. A human-readable explanation of a match, which hosts would show as is, may be added later. The criteria today, for maintainers:
  - **strong:** the filter's serial number pattern matched, or its name pattern matched a name specific to the board, such as "Alhambra II v1.0A".
  - **possible:** the name pattern matched a USB chip's factory default name, which many boards keep, such as FTDI's "Dual RS232-HS" (`FACTORY_DEFAULT_NAMES` in `src/devices.rs`, the same for catalog and custom boards); or the device matches only by USB IDs, and only a few boards (at most 3, `FEW_BOARDS`), so the match is worth naming.
  - **weak:** only the USB IDs matched, shared by more boards, or the device can't be opened, so its names are unknown.
  - Planned: a matching FPGA ID code will make a match strong, and a different one will drop the board.
- **Matching at request time.** Hosts can define their own boards at any time, so the engine matches devices on every request against the catalog and the custom boards the request carries, never against data prepared in advance.
- **Devices for a build.** The scan can be limited to the devices that fit a build's board and board variant, each with its match level for that build, so every host's Upload works the same way without knowing board IDs, custom boards included.
- **The host chooses.** Programming takes the ID of a device from the scan; the engine never picks one. A host that sees exactly one suitable device may send it without asking; otherwise it asks the user. A device ID is opaque and short-lived, and may depend on the platform and the computer: it stays valid while the device stays on the same USB port.
- **Commands name the device.** Where the programmer can be told which device to open, the command does: openFPGALoader through an FTDI chip with `--busdev-num ${BUS}:${DEV}` (72 board variants), iceprog with `-d d:${BUS}/${DEV}`, and dfu-util with `-p ${USB_PATH}`, the device's USB port path (9 board variants; dfu-util writes the path in the same `bus-port.port` form as the engine, both from libusb, so they stay `tested`), with a test for each. USB-Blaster and CMSIS-DAP adapters (through openFPGALoader 1.2.0, whose `--busdev-num` applies only to FTDI cables) and icesprog can't be told, so for those the engine's check below refuses when another device with the same USB IDs is connected.
- **The engine checks.** Before programming, the engine checks that the device is still there (same port, USB IDs and serial number), that it matches the build's board variant, and, when the command can't name the device by bus and device number or by a serial number only it has, that no other connected device has its USB IDs, since the programmer would open the first one.
- **Unreachable devices.** A device that can't be opened, for lack of a driver or permissions, is treated as not connected: it can't be chosen. When its USB IDs fit a board, the scan lists it apart, with that partial signature and the fix, so the host can suggest installing a driver or fixing permissions.
- **Custom programmer command.** A request can carry a custom command template that replaces the board variant's.
- **Linux permissions.** The engine provides the udev rules that give users access to the devices. When a device is found but cannot be opened, the engine reports a setup error saying that permissions are missing.

## Custom definitions

The catalog is the `logix-catalog` release the engine's remote config names. Hosts extend or override it at runtime with their own boards, which apply in their requests for building, programming and the other operations. Custom boards belong to the host, like the rest of its project model.

- **One folder, everywhere.** Every tool that deals with boards (build, lint, test, simulate, program_board, list_devices, get_catalog, compare_boards) takes the same optional `custom_boards_folder`, the path of a folder of board files in Logix Catalog's format, one board each, named by board ID (decided 2026-11-08, replacing a single file that mapped IDs to boards: the catalog's own layout, so a board copies between a project and the catalog as a file; files the engine reads, so it can report problems in them at line and column, as for sources). It is independent of the board a request selects: `board` is always an ID, the catalog's or a custom board's. Hosts send the same folder with every request.
- **Custom replaces catalog.** A custom board with a catalog board's ID replaces that catalog board for the request (decided 2026-11-06).
- **Readable.** Custom boards are the catalog's format, written by hosts or people (decided 2026-11-06).
- **Boards, not FPGAs.** A custom board names its FPGA by catalog ID (`fpga_id`). The FPGA catalog has every FPGA the underlying tools support, so there is nothing to add: custom boards add boards and how to program them, never FPGAs (decided 2026-11-07, as Apio does it; full FPGA definitions in custom boards were dropped).
- **Reading the catalog.** Hosts can read the full catalog through an engine request, with their custom boards listed among its boards.
- **Self-contained requests.** The engine never looks up or merges definitions from files on its own: it reads only the custom boards folder a request names. A request alone determines which board, FPGA and board variant are used.
- **Errors at line and column.** The engine checks every file in the custom boards folder in every tool and reports each problem as a diagnostic at file, line and column, as for source files, in a request error, so hosts show them like build errors (decided 2026-11-06: hosts send the file even when it has errors). Strict: any problem in any file stops every tool that takes the folder, even when the request selects a catalog board or another, valid board in the folder, so a mistake never goes unnoticed and nothing runs with a definition the user didn't intend (decided 2026-11-06, over reporting other boards' problems as warnings).
- **Uploads match builds.** A build record keeps the board it was built for; `program_board` refuses an upload only when the board the request gives under that ID has different FPGA build settings, or is gone. Only what the bitstream depends on counts, not where the board comes from (decided 2026-11-06).
- **Stable format.** The definition format is part of the tool API and follows the rule that it never breaks.
- **Catalog fixes.** A fix to the catalog needs only a catalog release and a change to the engine's remote config, no engine release. Until then, a host can send a corrected board with the catalog board's ID in its custom boards.

## Performance

CLI hosts start a new engine instance for every command, so the engine's own overhead must stay small next to the time the tools take. These are targets, to be confirmed by measurement.

- **Startup.** An instance is ready to accept requests within 200 ms of launch, excluding first-time package downloads.
- **No network at startup.** Starting an instance never contacts the network. Network access happens only before an operation that needs packages (to download or repair them and, with remote updates, to refresh a remote config more than 24 hours old), or when a host asks for it.
- **Verification on download.** Package checksums are verified when packages are downloaded, not every time an instance starts.
- **Operation overhead.** The engine adds no more than 100 ms to an operation beyond the time its tools take.
- **Up-to-date builds.** A build with no changes since the last build finishes within 1 second, using the incremental state in the build folder.

## Acceptance and phasing

The criteria and phases below are initial targets, not fixed commitments. They may be relaxed or changed as the project learns what matters in practice.

**Acceptance criteria.** The engine is ready for its first stable release when:

- **Same results:** for the golden test designs, the engine produces the same bitstreams as the standard open source flows with the same package versions. Where the tools are not fully deterministic, utilization and timing results match instead.
- **Real hardware:** board programming works on a reference board for each supported programmer, tested with the hardware-in-the-loop tests.
- **AI agent as host:** an AI coding agent, connected to the engine as a generic MCP client with no frontend, builds and programs a reference design on a real board, guided only by the tool descriptions and the engine's errors.
- **All platforms:** the test suite passes on every supported platform.
- **Performance:** the targets in the [Performance](#performance) section are met.

The 109 boards of the original board list split by FPGA family roughly as iCE40 about 42, ECP5 about 36, Xilinx 7 Series about 23 and Gowin about 9. This suggests the following phases.

**Table 14.** Proposed phases

| Phase | Scope | Done when |
| --- | --- | --- |
| Milestone 1. Proof of concept (done 2026-10-29) | A proof of concept of the engine, built on the real design so it evolves into the full engine rather than being thrown away: MCP interface, automatic package download and versioning, read-only catalog, error model, iCE40 only, and seven tools: `get_engine_info`, `download_packages`, `get_catalog`, `build`, `clean_build`, `list_devices` and `program_board`. macOS only, with no Linux or Windows yet. GitHub workflows that build and test the engine. Developer scripts to run, build, clean and test it locally. The underlying tools come from existing open source toolchain packages | An AI coding agent, such as Claude Code, is the host, connected to the engine over MCP, with no frontend. From prompts alone, it changes or creates an iCE40 design, builds it and programs it on an Alhambra II board. Met on 2026-10-29 |
| Milestone 2. Logix IDE on the engine (done 2026-11-13) | Everything Logix IDE offered before it moved to the engine, as listed in Table 15: lint, test and simulate with a waveform viewer, format, diagram, live tool output, and every board Logix IDE offers today, across iCE40, ECP5, Gowin and Xilinx 7 Series, with the programmers they need. A board is dropped only for a very good reason, recorded in this document with the reason. macOS only, like Logix IDE's releases | Logix IDE's macOS release runs on the engine alone and keeps every feature in Table 15. Its board selector offers the same boards as before, apart from boards dropped for a recorded reason. Every toolchain builds and programs a board, tested on hardware where a board is available |
| Milestone 3. Windows | Logix Engine and Logix IDE on Windows x64: engine releases with the toolchain packages for Windows, the drivers boards need there (Drivers, under Boards and board variants), driver management (below), and a Logix IDE installer that needs nothing else (the WebView2 runtime included). CI builds and tests both on Windows | Logix IDE's Windows release runs on the engine alone and keeps every feature in Table 15, with the same boards as on macOS. The test suites pass on Windows, and every programmer programs a board on Windows, tested on hardware where a board is available. Driver management: Logix Engine detects when a connected board needs a driver it doesn't have, and installs it when a host asks (Logix IDE asks the user first; every frontend and AI agent gets the same through the engine's API), as automatically and safely as possible (only what the board needs, with Windows' own consent prompt, nothing changed silently), and can uninstall what it installed |
| Milestone 4. Linux | Logix Engine and Logix IDE on Linux x64, the same way: engine releases for Linux, the udev rules that give users access to their boards (Linux permissions, under Device selection), driver management (below), and a Logix IDE package that works out of the box (AppImage, and `.deb` with dependencies the package manager resolves). CI builds and tests both on Linux | As for Milestone 3, on Linux. Driver management: Logix Engine detects when a connected board can't be opened for lack of a udev rule, and Logix IDE installs the rule, as automatically and safely as possible (only the board's USB IDs, with the system's own password prompt, nothing changed silently), and can uninstall what it installed |
| 5. More boards | The remaining boards and programmers as decided in the open questions, and the acceptance criteria above | The acceptance criteria are met |
| 6. More hosts | More Open Logix frontends, such as Logix CLI and Logix Studio, and other tools as hosts, with the requirements in Table 6 | Each runs its core flows on the engine |
| Later | Network adapter, on-board verification, possibly VHDL | Decided when they come up |

Milestone 1 tested the interface with an AI coding agent as the host, before any frontend used the engine. Milestone 2 makes Logix IDE the first frontend on the engine, on macOS. Milestones 3 and 4 bring both to Windows and then Linux (decided 2026-11-10). Other hosts follow.

**Table 15.** Logix IDE features that Milestone 2 must keep

| Logix IDE feature | Engine support needed |
| --- | --- |
| Board selector | `get_catalog` with the same boards, apart from boards dropped for a recorded reason |
| Examples menu | None. Examples stay with the host, as in Table 7's Examples row: Logix IDE provides its own |
| Verify | A `lint` tool; `build` with its resource and timing results |
| Upload | `program_board`, with the programmers of the supported boards |
| Run Tests | A `test` tool following the testbench rules |
| Simulate | A `simulate` tool that writes the waveform and opens a viewer without waiting for it |
| Auto Format | A `format` tool and the package with Verible |
| Show Diagram | A `diagram` tool that writes the diagram: a page with every module, and each module's SVG |
| Console | Live tool output, line by line, while each operation runs |
| First start | `download_packages`, with progress |
| Stop | Cancellation of every tool |

**Hardware tests (done 2026-11-10).** All four toolchains (iCE40, ECP5, Gowin and Xilinx 7 Series) built a design and programmed a board through Logix IDE on the engine, tested by the user on real boards. Table 15's features are all in Logix IDE.

**Also done for Milestone 2 (added 2026-11-10; Milestone 2 declared done by the user on 2026-11-13, after the first promotion made Logix Engine and Logix IDE 2026-11-12 prod).** Besides Table 15 and the hardware tests above:

- **Done (2026-11-11): Xilinx clock names.** logix-core pins openXC7 2026-11-09, whose nextpnr-xilinx names a clock net after the design's signal (`CLK`), so `clocks_summary` shows it as for the other families (the build tests check it). Checked by the user in Logix IDE's Board Utilization on an Arty A7-35T (2026-11-11): `sys_clk`, its net `sys_clk`.
- **Done (2026-11-10): decoupled from Apio's releases.** The engine downloads two packages of Open Logix's own: `logix-catalog` (the boards and FPGAs, from `openlogix/logix-catalog`, small, first) and `logix-core` (the tools, from `openlogix/logix-core`: the YosysHQ OSS CAD Suite, openXC7 and Verible, plus Graphviz on Windows (replaced by ELK's `elkjs` on 2026-11-11, for the diagrams), each its pinned upstream release unchanged in a folder of its own, one reproducible `.tgz` per platform). The core's `logix-core.json` says how to run each tool (its `PATH` folders); the catalog has nothing built in. The design is in `openlogix/logix-core`'s `docs/design.md`. Still to do: test each new core on every platform's own runner before pinning it.
- **Done (2026-11-11): a getting started document for Logix IDE.** [Getting started](https://www.openlogix.org/logix-ide/getting-started/) takes a new user from nothing to their own design running on their board: install, first start, the board, Blinky, Verify, Board Utilization, the diagram, tests, simulation, Upload, a change, and saving.
- **Done (2026-11-11): a better design diagram.** The `graph` tool (yosys `show` and Graphviz's dot, crude) is replaced by the `diagram` tool, decided with the user after a prototype: schematic symbols laid out by ELK (logix-core's `elkjs`, run by the engine in QuickJS, so the engine needs no other program and logix-core no Graphviz), every module of the design, labels for clocks, resets and fixed values, a model of the design for AI agents, and one self-contained page with drill-down, pan, zoom and hover that any browser opens, which an AI agent can open for the user (`open`) without generating anything. Logix IDE shows it in a window of its own. Graphviz is dropped from logix-core.
- **Added 2026-11-11 (the user's decision): macOS first.** Milestone 2 does as much as can be done on macOS before Milestones 3 and 4 port the engine and Logix IDE to Windows and Linux. So these topics, still to discuss and decide with the user, are part of it (a third, the relative complexity of modules, was dropped by the user, 2026-11-11):
  - **Done (2026-11-12): remote updates.** How Logix IDE, Logix Engine, the boards and the examples get updated, including new or fixed boards and examples without a new Logix IDE or engine release. The design is in the website repo's `CLAUDE.md` (Remote updates): a remote config per component and config version (pre-releases allowed during development; requiring stable releases is deferred), read every 24 hours and checked before each operation, as Apio does. The engine downloads its core and catalog by its own config; Logix IDE downloads the engine and its examples (`logix-ide-examples`) by its own, so the engine isn't bundled with Logix IDE. `repos-janitor` in `openlogix/.github` checks the configs and releases across repos every 6 hours.
  - **Done (2026-11-11): board design notes for AI.** They are Logix Catalog's `ai_hints` (2026-11-09), which `get_catalog` returns; a check of the boards without a usable clock found their clocks' jumpers already recorded as `requires` setup entries. A full review of every board's hints and facts comes after the Windows and Linux ports (the user's decision, 2026-11-11).

**Milestone 3 plan and decisions (the user's, 2026-11-13).** Logix Engine and Logix IDE on Windows x64, in five phases:

1. **CI on Windows first**, so every later change is tested there: `windows-latest` in the engine's and Logix IDE's test matrices, a `build-windows-x86-64.yaml` in each, `windows-x86-64` in the platforms the remote configs, `repos-janitor` and the README icons check, and logix-core's Windows package run on a Windows runner (is `gowin_pack` with its Python there, and `iceprog.exe`?).
2. **The engine:** `.exe` tool names (and openXC7's `fasm2frames.cmd` run through `cmd`), `PATH` built with `std::env::join_paths` and the variables Windows tools need (`SystemRoot`, `TEMP`, `USERPROFILE`), Stop ending a tool's whole process tree (a Job Object) while still letting `vvp` write its waveform (Ctrl+Break, not SIGTERM), a Windows form of the reproducible command lines, detached GTKWave and diagram pages, and tests that don't assume `/bin/sh` or symlinks.
3. **Logix IDE:** Windows versions of `platform.rs`'s commands (Explorer, the clipboard, URLs, the Recycle Bin, the OS version, bringing GTKWave to the front), replacing the engine while Windows locks the old one's files, platform wording ("File Explorer", Ctrl), and the installer.
4. **Drivers, in the engine:** which driver each connection needs on Windows (WinUSB for FTDI and DFU, none for CMSIS-DAP), detecting a missing or wrong one, and installing and uninstalling it for one device, behind Windows' own consent prompt. Engine first: every frontend and AI agent gets it through the engine's API; Logix IDE asks the user and calls it. The installer is libwdi's `wdi-simple` (libwdi's command-line example, which libwdi doesn't publish built). Decided 2026-11-13 (the user's): logix-core builds it in its own workflow from libwdi's pinned source (`pbatard/libwdi`, tag and commit), unmodified, and ships it in its Windows package as `wdi-simple/` (logix-core's `docs/design.md`, wdi-simple); no personal repo is used. **WinUSB only**, for every connection that needs a driver (FTDI's interface 0, which iceprog, openFPGALoader's FTDI cables and the Digilent, Alchitry, Tang Nano and ULX3S boards use, and DFU bootloaders), as Apio tells its users to choose in Zadig for the same boards (it was libusbK once); CMSIS-DAP and the iCESugar's iCELink need none. The engine derives which driver a variant needs from its programmer and connection (no catalog field), reads each interface's current driver from Windows (SetupAPI), reports both in `list_devices`, and gets `install_driver` (`wdi-simple`, behind UAC) and `uninstall_driver` (Windows' `pnputil`, behind UAC). Windows binds a driver by USB IDs and interface, not by serial number: libwdi installs on every connected device with those IDs, and one plugged in later may keep its driver (Windows chooses then; to check on hardware). So `install_driver` refuses while other connected devices have the same USB IDs, naming them, unless `allow_same_ids` (the user unplugs them or agrees), and vendor tools (such as Vivado for a Digilent board) can't use the interface until it's uninstalled; the confirmation says so. **Elevation** (found 2026-11-14 in libwdi's source): libwdi creates and signs the driver's catalog only when its process already is an administrator (`libwdi.c`, `wdi_prepare_driver`), and Windows 10 and later refuse an unsigned package, so `wdi-simple` must run elevated as a whole, as Zadig does; the engine starts itself again as an administrator (`ShellExecuteEx` runas, so Windows' prompt names Logix Engine, one prompt per change) for exactly two typed operations (`src/elevated.rs`), with every step's output captured for the log. **Uninstall,** at least as thorough as an expert by hand (Zadig has none): every driver package libwdi wrote for the interface's hardware ID (provider `libwdi`, `libusbK` or `libusb-win32`, and libwdi's `DeviceID`; from Windows' INF folder), deleted with `pnputil /delete-driver /uninstall /force`; libwdi's certificate for it (`CN=<hardware ID> (libwdi autogenerated)`) removed from the machine's Root and TrustedPublisher stores; then `pnputil /scan-devices`, and the engine checks the result.
5. **Hardware tests, docs and release:** every programmer programs a board on Windows, Windows install steps in the docs and release text, then a promotion.

Decisions: Logix IDE's Windows installer is an `.msi` (Tauri's WiX bundle, no cost; it handles WebView2 itself). No code signing for now (maybe later): the release text explains Windows' SmartScreen warning, as it explains macOS's quarantine flag. Development stays on macOS: Windows code is built and tested by CI on Windows runners, and the user tests on a Windows x64 PC with boards, using CI's release builds only (no development tools there), so whatever goes wrong must be diagnosable from the installed product (System Info, the logs, Report a Problem).

**Boards dropped from Milestone 2 (decided 2026-11-03).** The catalog covers 95 of the 109 boards of the original board list (as 84 boards, some with board variants; 81 boards now, after the later changes below). The 14 others are dropped because their programmers are legacy or inactive, or not in the packages the engine uses. None of the programmers the engine supports can program them:

| Programmer | Boards | Reason |
| --- | --- | --- |
| tinyprog | tinyfpga-bx, tinyfpga-ex-rev1, tinyfpga-ex-rev2 | Legacy: the TinyFPGA project is inactive. Needs a serial port |
| tinyfpgab | tinyfpga-b2 | Legacy, older than tinyprog. Needs a serial port |
| blackiceprog | blackice, blackice-ii, blackice-mx | Legacy: myStorm is discontinued. Needs a serial port |
| icefunprog | icefun, icewerx | Niche and old. Needs a serial port |
| iceprogduino | ice40-hx1k-evb, ice40-hx8k-evb | Legacy. Needs a serial port |
| iceburn | iceblink40-hx1k | Legacy: the board is discontinued |
| pi-sicle-loader | pi-sicle | Legacy, and works only when running on a Raspberry Pi |
| apollo | cynthion-r1-4 | Current, but apollo isn't in the toolchain packages. Can be added later as its own package |

**Later changes (decided 2026-11-08).** Boards that can't be bought anymore, and haven't been for some time, are dropped (the user's rule): `kefir` (Kéfir I: its maker says no supplier offers it) and `upduino`, `upduino2` and `upduino21` (UPduino v1, v2.0 and v2.1: replaced by the UPduino v3.1, which is in the catalog). The Tang Primer 20K core board is used on one of two carrier boards with different LEDs, so `sipeed-tang-primer-20k` became two boards, `sipeed-tang-primer-20k-dock` and `sipeed-tang-primer-20k-lite`, programmed the same way.

Five boards are programmed differently from their original commands, so the catalog marks them `tested: false`: the four ULX3S boards use openFPGALoader instead of fujprog, which is legacy, and odt-icyblue-feather uses iceprog, as the board maker's examples do, because icesprog can't program its FTDI chip. The iCESugar boards get a serial number filter (`serial_regex`), since the iCESugar, iCESugar-Pro and iCESugar-nano share USB IDs and product names.

## Development methodology

The engine will be implemented and maintained by human-guided AI. The human directs the work and judges the results, with a general understanding of the code and architecture but without reviewing every line. The code is therefore not required to be human-readable or intuitive. In general, the AI-generated code is treated like machine code: people are not expected to inspect or understand it. In contrast, the functionality, external interfaces and requirements must be straightforward, intuitive and easy for people to understand.

**Table 16.** Development roles

| Role | Responsibilities |
| --- | --- |
| Human | Provides instructions, tests the results and gives feedback to the AI |
| AI | Writes and maintains the code, the tests and the documentation |

The project relies heavily on AI-generated tests to catch regressions, since humans do not review every line of code.

## Project infrastructure

- **Source.** The engine's code lives in a GitHub repo, with issues and pull requests on GitHub.
- **Testing.** GitHub Actions workflows run the engine's test suite on every supported platform for every change.
- **Releases.** Workflows build the per-platform engine binaries and publish them as GitHub releases with date tags, like the package repos' daily releases.
- **Hardware tests.** GitHub-hosted runners cannot reach FPGA boards, so the hardware-in-the-loop tests run separately on a lab computer with boards attached. Whether that computer runs as a self-hosted GitHub runner is still open.
- **License.** The engine's own code is licensed under MIT or Apache-2.0, at the user's option, like every Open Logix repo: MIT lets GPL-2.0 projects use it, and Apache-2.0 includes an explicit patent grant. The packaged tools keep their own licenses.

## Open questions

- **A tool that writes pin files** (raised 2026-11-08; deferred by the user until the official release, Logix IDE, Logix Catalog and Logix Engine released on all three platforms, not just published). Every frontend that starts a design for a board needs a pin file in the board's format (PCF, LPF, CST or XDC), and today only Logix IDE's example generator knows the four formats. The idea: a tool such as `make_constraint_file` that takes the board and the design's ports, each a catalog resource (whole, or some of its bits under another port name) or explicit pins, adds the board's required drives, and returns the file's text and extension and the resolved ports (pins, direction, active level, clock frequency). Discussed and judged too complex for now. Open when it's taken up: the name, how resources with several kinds of pins (UART, RGB LED, LED matrix, header) become ports, and whether a resource that needs a jumper is an error or a warning.
- **Starting-design hints** (raised 2026-11-08; deferred by the user with the pin file tool above). Which of a board's resources a first design, such as a blinky, uses: its clock and its LED. Today Logix IDE's example generator picks them by the catalog's rules (resources that need a jumper left out; the clock with the lowest name; the first LED resource by type, then name). The idea: the engine works the hints out when it loads a board, for catalog and custom boards alike, and `get_catalog` returns them, with an optional authored field in board files for the board maker's choice. Deferred because today there's almost no choice to make (2026-11-08: no board has more than one usable clock, one has two LED resources, the iCEBreaker), and only the IDE's generator would use them. Revisit when boards with several clocks arrive (the LiteX and Amaranth importers) or a second tool starts designs. These would be structured fields that tools read, unlike `ai_hints` (2026-11-09: free text on boards and resources for AI agents writing designs; see docs/tool-api.md, AI hints).
- **A one-shot command-line mode** (raised 2026-11-08, to consider). Today a one-shot call is a short MCP session with a single tool call, and the engine has no separate command-line mode (docs/architecture.md). The idea: run one tool from the command line, such as `logix-engine --engine-folder … --tool build --input request.json --output result.json`, which reads the request from a file, writes the result to a file and exits, with an exit code for success or failure. It would let shell scripts, build systems and programs that don't speak MCP use the engine directly. To weigh: whether it adds enough over a short MCP session, how progress and live tool output would reach the caller, and that the command line, like the tool API, can only grow once released. Since 2026-11-08 the engine's API is the intended way for every tool, script and agent to read the catalog (the shared rule), so scripts need an easy way to call `get_catalog`; this mode would be it.
- **Serial ports, for programming and serial data** (raised 2026-11-14; to revisit after Milestone 4, not before, the user's decision). The engine sees USB devices through libusb only, so boards whose programmer talks over a serial port were dropped (the table of dropped boards above). Serial port support could serve:
  - **Programming.** The iCEFUN (Devantech; still sold, such as at RobotShop) and the iCEWerx (`icefunprog`), and the Olimex iCE40HX1K-EVB and iCE40HX8K-EVB (`iceprogduino`, already in the OSS CAD Suite; their availability is unchecked). Apio gets `icefunprog` from PyPI (pitrz's Python tool, GPLv3, with `pyserial`) and runs it in its own Python; the OSS CAD Suite in logix-core doesn't ship it, and Devantech's C `iceFUNprog` builds only on POSIX systems. Two options: ship the Python tool with a Python and `pyserial` on each platform in logix-core (the kind of dependency behind apio#1194), or implement its small protocol in the engine (erase, write 256-byte pages, verify, release the FPGA), which would break the rule that the engine only drives existing tools.
  - **Serial data.** A serial monitor in hosts such as Logix IDE (as Arduino's), for boards with a USB serial port to the FPGA (such as the Alhambra II, the iCESugar boards, the Tang Nanos and the Arty).
  - **What they share.** Finding the serial port of a matched USB device on each OS (`COMx` with SetupAPI, `/dev/cu.*` with IOKit, `/dev/ttyACM*` or `/dev/ttyUSB*` from sysfs; on a two-channel FTDI chip, the port on interface 1), listing it in `list_devices` with its board, permissions on Linux, and opening and reading the port when the programmer is built in. Only the monitor needs a long-running session (streamed output, input, baud rate, stopping), coordination with programming (the port closed during it), and a host UI.
- **Running the tools directly, for diagnostics** (raised 2026-11-11, to consider after Milestone 4). Apio's `apio raw` runs any of its packaged tools with the right environment, and it was very useful for diagnosing problems. Three ways the engine could do the same: a tool that runs a given tool command in its environment (such as `yosys -V` or `openFPGALoader --scan-usb`) and returns its output; a way to export that environment (each tool's folders on `PATH` and its variables, from `logix-core.json`), so a person or a script can run the tools directly in a terminal; or a script the engine returns (a shell script, a `.cmd` on Windows) that sets up that environment and runs a tool command, which the caller runs as is or changes first, such as to try other options. Either is for diagnostics, not for normal use: hosts keep using the engine's tools. Today each result's `commands` list already has a shell line that runs that exact command again without the engine, a start.
- **Letting AI clients examine build and simulation artifacts** (raised 2026-11-11, to explore after Milestone 5). To fix a design in a loop, an AI client sees only what its testbenches print and what the tools return (diagnostics, the build report, the commands). Explore tools that let it look into what a build or a simulation leaves in its folder, without parsing tool files itself: the waveforms (signal values over time, around a time or an event), the synthesized netlist (what drives a signal, what a module became), timing (the slowest paths), and the place-and-route log. Open: which questions such tools answer, how much they return (waveforms get large), and whether they're engine tools or a documented artifact layout.
- **Partial selectors in requests** (raised 2026-11-06). A request could give only some dimensions, such as `{"memory": "ram"}`, meaning the first board variant that matches. That is the Logix IDE's own variant choice (`variant_id`), so it would move that logic into the engine. Not done: requests give both values or none.
- A much better device exploration and identification system (raised 2026-11-05). Today a board's filter either names its device's product or not, and a filter with only USB IDs can't be ruled out by any name, so a device that names itself, such as "Alhambra II v1.0A" (manufacturer AlhambraBits), still fits the 27 catalog boards whose filters have only the FTDI chip's IDs, as weak matches. Ideas to weigh:
  - **Kinds of names.** A product or manufacturer name is generic (a USB chip's factory default, such as "Dual RS232-HS"), vendor-specific (such as "Digilent USB Device" or the manufacturer "AlhambraBits") or board-specific. A vendor-specific name rules out other vendors' boards even when their filters have no name: an AlhambraBits device isn't a Xilinx board. That needs each board's vendor in the catalog, and a list of known vendor names next to `FACTORY_DEFAULT_NAMES`.
  - **What the market knows.** Which vendors make which FPGA families, so a vendor's name narrows the boards to its own.
  - **FPGA identification** (part 3 of the device identification plan): reading the FPGA's ID code over JTAG, which confirms or drops ECP5, Gowin and Xilinx boards without knowing their USB names. It doesn't help iCE40 boards programmed through their flash.
  - **What the research found** (2026-11-05, first pass over the 45 boards whose filters then had only USB IDs; 12 now have product names, and 33 boards still have only USB IDs; for most of the rest no public source gives their USB names: the BRS-100's schematic blanks out its USB part, and the one sample, which matches a Tang Nano 9K in names and FPGA, is the only evidence). About 16 boards are programmed through a cable the user plugs in (ColorLight, ECP5 Mini, FleaFPGA, ZXTRES and others), so no USB name can ever name them, only the FPGA's ID code. Digilent's names ("Digilent USB Device", "Digilent Adept USB Device") are vendor names, also used by Digilent's stand-alone cables; Digilent serial numbers may start with a product code (210319 on an Arty A7, 210183 on a Basys3; unconfirmed). Sipeed's debuggers report "Sipeed-Debug" (CH552), "JTAG Debugger" with manufacturer "SIPEED" (BL702) or "SIPEED USB Debugger" (BL616), shared with Sipeed's other debuggers; the BL702 reports device release 0500 where a real FT2232H reports 0700, so the four BL702 boards (Tang Nano 1K, 4K, 9K, Primer 20K) now have `release` 0500 and the name "JTAG Debugger" in their filters, and a real FT2232H no longer matches them. The FPGA's ID code alone names only 7 of the 53 catalog boards with JTAG (the Arty A7-35T, Basys3, Cmod A7, Alchitry Au, Alinx AX7035 and ZXTRES share one), but with the USB names it settles many: a Digilent device with an XC7A100T is an Arty A7-100T or a Nexys A7-100T.
  - **Signatures from users.** Most catalog boards' real USB names have never been recorded, because no maintainer has the board. Users who have a board could contribute a report (see Board files and a Board Wizard below): never automatic and never telemetry, only when the user chooses to share. For people without a frontend, a GitHub issue form with each OS's built-in command (`system_profiler SPUSBDataType`, `lsusb -v`, a PowerShell line) and later a WebUSB page on the website can collect the same signature. A maintainer turns reports into catalog filters, citing them in `maintainer_notes`; one report is one sample.
- Showing clients less of the engine's device model (raised 2026-11-05, partly done). USB filters are internal matching data, yet `get_catalog` returns every variant's `usb` filter, and `list_devices` reports matching details such as `release`. Clients only need the match levels, so since 2026-11-05 `get_catalog` returns boards without their filters and `list_devices` doesn't report the release. Filters remain in custom boards, which hosts write (decided 2026-11-06: readable custom boards in the catalog's format).
- **Board files and a Board Wizard** (built 2026-11-05, dropped 2026-11-06). Engine-made, opaque board files (`make_board_file` from a connected device and a catalog board, upload evidence recorded by `program_board`, a shareable report from `share_board_file`) and the Logix IDE's wizard for them were built, then dropped: the user found the result confusing, and chose plain custom boards in one consistent list across all tools (see Custom definitions). Ideas kept for later: the engine writing a custom board's USB filter from a connected device (product name, release, serial number to tell identical boards apart), and a report users can paste into a GitHub issue, with the serial number only as a shape.
- Which more frontend logic to move into the engine (the Logix IDE is the reference frontend; see CLAUDE.md, A reference for frontends). Candidates from the IDE, 2026-11-05: choosing a board variant from dimension values (the IDE's `variant_id`: the first variant with all chosen values), a device's name for people (product, else manufacturer, else its USB IDs), the strongest matches of a device versus the weaker ones to count (the IDE's `likelyBoards` and `otherBoards`), and the plain summary of a test run. Each should move only if it doesn't depend on a frontend's own model.
- Resolved (2026-11-11): Xilinx clocks are named after the design's clock signal. Older nextpnr-xilinx builds reported a clock net name synthesis made up, such as `$abc$21083$aiger$o100`; openXC7 2026-11-09 reports the signal (`CLK`).
- **Done (2026-11-08): the catalog is Logix Catalog** (`openlogix/logix-catalog`; its `docs/model.md` and `docs/design.md` are the source of truth). The engine reads its format (one board file each, typed `resources`, one schema `logix-board-v1.json`), downloads a pinned release as the `logix-catalog` package (since 2026-11-10; a local clone given by `LOGIX_ENGINE_DEV_CATALOG_FOLDER` at runtime instead), reports the catalog version in `get_engine_info`, and takes custom boards as `custom_boards_folder`.
- Whether to move more boards to openFPGALoader, which would simplify packages and testing. Its DFU support may cover the dfu-util boards, but each moved board must be tested on hardware.
- Whether to offer flash on the ULX3S: some boards have a DFU bootloader at the start of the flash, which a JTAG flash write (`-f`) would overwrite. openFPGALoader's `ulx3s_dfu` board writes through that bootloader instead, but only on boards that have it.
- How to program one particular board when several boards of the same model are connected. Device IDs tell them apart by USB port, and Upload asks which one, but the user may not know which physical board each entry is: identical boards report the same names, and many have no serial number or the same one. Some programmers can't be told which device to open at all (icesprog, openFPGALoader with USB-Blaster or CMSIS-DAP adapters), so today the engine refuses when another device with the same USB IDs is connected; dfu-util is told the USB port since 2026-11-04. Ideas to weigh: show where each board is plugged in, in words a user can follow (such as the port or hub); let the user identify a board by unplugging it and watching which entry disappears; pass a serial number where the programmer supports it; and let a host remember a name the user gives a board, keyed by its serial number when it has a unique one.
- What the scope and lifetime of a device ID should be. Today an ID is made from the USB port path, the USB IDs and the serial number, and the API promises only that it is opaque and short-lived. In practice it stays valid when the device is plugged in again in the same port and across engine restarts, but not when it moves to another port, and maybe not after the computer restarts, since bus numbers can change then. Questions: should the API promise any of these, or stay at "list the devices again before choosing"? Should there also be a stable identity a host may store, such as to remember "my second Alhambra", based on a unique serial number where there is one? This ties in with choosing one of several identical boards (above), which goes beyond choosing a board and its board variant.
- What the ButterStick r1.0 bootloader reports as its product name. Its filter, `^butterstick (dfu v1.1)`, treats the parentheses as a regular expression group, so it matches "butterstick dfu v1.1" but not "butterstick (dfu v1.1)"; a real board's name decides the fix.
- What the catalog should say about drivers for each board variant, so a host can tell which driver a variant needs on each platform and whether it is missing. The planned driver data per connection value (see Drivers under Boards and board variants) covers the common cases, such as WinUSB for FTDI chips on Windows and a udev rule on Linux; open is how a variant states an exception, how the engine detects a missing or wrong driver (such as FTDI's own driver claiming a device that needs WinUSB), and how that reaches the host as an action it can show or fix.
- Whether to support Linux arm64 and single-board computers attached to the target board, such as a Raspberry Pi. (Windows arm64 isn't planned for the near future, decided 2026-11-10.)
- Which hardware test interface to use for the engine's own hardware tests, and whether the same interface can serve the on-board verification feature for user designs.
- Whether the network adapter is part of the first release.
