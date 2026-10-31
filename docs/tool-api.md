# Tool API

This is the engine's tool API: the seven tools of Milestone 1, and `lint`, `test` and `simulate` from Milestone 2. It is not frozen yet: the rule that the tool API never breaks applies from the first release. Each tool is an MCP tool. Its inputs are the tool's arguments, and its outputs are the tool's structured content. The exact JSON schemas and tool descriptions are in [schemas/tools.json](../schemas/tools.json).

## Schemas and validation

- **Export.** The engine returns each tool's name, description, `inputSchema` and `outputSchema`, in JSON Schema, when a host calls the MCP `tools/list` method. MCP clients, including AI agents, read them from there, so no separate schema file is needed.
- **Source of truth.** The schemas are generated from the engine's own Rust types, so the code, the validation and the exported schemas always agree.
- **Input validation.** The engine checks every request against the tool's input schema before doing anything. A request that doesn't match is a request error, returned as a tool error rather than an MCP protocol error. Its message and diagnostics name the field and the problem, so a host or AI agent can fix the call and retry.
- **Rules beyond the schema.** Some rules can't be written in JSON Schema, such as the path keyword rules, symbolic link checks, whether a file exists, or whether a board ID is in the catalog. The engine checks these right after the schema, and they are also request errors.
- **Tight schemas.** Where JSON Schema can express a rule, the schema states it, such as a pattern for catalog IDs, USB IDs and define names, and the allowed values of each enum field. This catches more mistakes early and shows the rules to AI agents.
- **Output.** Each tool's structured content always matches its output schema.
- **Published copy.** The schemas are exported to [schemas/tools.json](../schemas/tools.json), exactly as `tools/list` returns them. A test fails when the file and the code differ, and `scripts/update-schemas.sh` regenerates it, so schema changes show up in code review and the rule that the tool API never breaks can be checked automatically.

## Common types

### Logical types

The Type columns in this document use these logical types. Each maps to a JSON type with extra rules. Plain integer, number and boolean values use those JSON types directly.

**Table 1.** Logical types

| Logical type | JSON type | Rules |
| --- | --- | --- |
| absolute path | string | A complete path in the host computer's native form. On Windows, either `\` or `/` separators are accepted. It contains no `.` or `..` segments, and no `~`, environment variables or other shorthand: the host expands and normalizes it first. The engine still resolves symbolic links before using it. Case sensitivity follows the platform |
| relative path | string | Relative to the folder named in the field's description. Always uses `/` as the separator on every platform, never starts with `/`, and contains no `.` or `..` segments. An empty string means the folder itself, but only in fields whose description says so. Elsewhere it is a request error |
| build folder path | string | An absolute path that satisfies the path keyword rules |
| catalog ID | string | Lowercase letters, digits and hyphens |
| board specification | string or object | A catalog ID, or a full board definition |
| board definition | object | A board entry in the catalog format: its `description`, its FPGA as `fpga_id`, a map of programming variants keyed by variant name (`variants`), and the name of its default variant (`default_variant`) |
| FPGA definition | object | An FPGA entry in the catalog format: its `architecture` (`ice40` for now), `part_number`, `size`, and the toolchain parameters `nextpnr_device` and `nextpnr_package` |
| programming variant | object | One way to program a board, such as flash or sram, or a particular cable, such as FT232 or FT2232. It names the programmer to run, such as `openfpgaloader`, and gives the full argument template for it, with variables such as `${VID}`, `${PID}`, `${BUS}`, `${DEV}`, `${SERIAL}` and `${BITSTREAM}`, and the USB filter that finds the device: `vid`, `pid` and an optional `product_regex`, a regular expression the device's product name must match, for boards that share USB IDs. The engine maps the programmer name to its executable and the package that provides it, so there are no separate programmer definitions. Milestone 1 supports the programmers `openfpgaloader` and `iceprog` |
| catalog query | object | Selects entries from one catalog section. It has `fetch`, a boolean that defaults to false, and `ids`, an optional list of catalog IDs. When `fetch` is true, the section is returned with only the listed IDs, or with all its entries when `ids` is omitted. An ID that isn't in the section is a request error |
| date tag | string | `yyyy-mm-dd` |
| USB ID | string | A 16-bit USB vendor or product ID, written as exactly four lowercase hexadecimal digits, such as `1d50`. Inputs are matched without regard to case |
| text | string | A single line of Unicode text, with no line breaks or other control characters |
| multiline text | string | Unicode text that may span several lines. Lines are separated by `\n` (LF only) on every platform, with no trailing newline. The only control characters allowed are `\n` and tab. The engine converts tool output to this form, including CRLF line endings, and strips terminal color codes |
| boolean | boolean | true or false |
| enum | string | One of the values listed in the field's description |
| list of X | array | Items of type X |
| map of X | object | Keys are catalog IDs or names, and values are of type X |
| error, diagnostic, action, device, resource, testbench result, command | object | See error fields in [Table 3](#results-and-errors), diagnostic fields in [Table 4](#results-and-errors), action codes in [Table 5](#results-and-errors), device fields in [Table 13](#list_devices-tool), resource fields in [Table 7](#build-tool), testbench result fields in [Table 27](#test-tool), and command fields in [Table 30](#commands) |

### Folders

Operations that work on a design take these fields. Paths are absolute unless noted.

**Table 2.** Folder fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `source_folder` | absolute path | Yes | The source folder |
| `source_files` | list of relative paths | Yes | HDL files to use, relative to the source folder |
| `constraint_file` | relative path | Depends on the operation | Constraint file, relative to the source folder |
| `build_folder` | build folder path | Yes | The build folder, as an absolute path. It may or may not be inside the source folder |

### Board specification

The `board` field is either a catalog ID, such as `"icestick"`, or a full definition object in the catalog format. A full definition can name its FPGA by catalog ID or include its full definition. Its programming variants are always written out in full.

### Results and errors

On success, a tool returns its output object as structured content. On failure, the MCP result is marked as an error, and its structured content is an error object, with the error fields in Table 3.

**Table 3.** Error fields

| Error field | Type | Description |
| --- | --- | --- |
| `category` | enum | One of `design`, `setup`, `request`, `tool`, `engine` or `cancelled` |
| `message` | text | Short message for end users |
| `details` | multiline text | Troubleshooting details, such as tool output |
| `diagnostics` | list of diagnostics | Problems with file and line, mainly for design errors. See diagnostic fields in Table 4 |
| `action` | action | Optional suggested action, with an enum code from Table 5 and a text description |
| `data` | object | Optional extra data, such as candidate devices, or `commands`, the programs a failed call ran, as in [Commands](#commands) |

**Table 4.** Diagnostic fields

| Diagnostic field | Type | Description |
| --- | --- | --- |
| `file` | relative path | File, relative to the source folder, when known. Many tool messages have no location |
| `line` | integer | Line number, starting at 1, when known |
| `column` | integer | Optional column number, starting at 1 |
| `severity` | enum | `error`, `warning` or `info` |
| `text` | text | The problem, in words |
| `tool` | text | Optional name of the tool that reported it |
| `time_ns` | number | Optional. For a testbench failure, the simulation time it happened at, in nanoseconds, when known |

**Table 5.** Suggested action codes

| Action code | Meaning |
| --- | --- |
| `install_packages` | Required packages are missing or broken and weren't installed automatically, because auto-install is off or the install failed |
| `connect_board` | No matching board is connected |
| `select_device` | Several devices match; pick one and retry with its serial number |
| `fix_device_permissions` | A device was found but cannot be opened |
| `install_drivers` | A driver required by the board is missing |

### Presentation

The engine reports facts as plain data and leaves presentation to the host. It sends no colors or terminal codes, and strips them from tool output. It doesn't filter or rewrite tool output: tool lines reach the host as the tool printed them. Instead it labels what things are, such as diagnostics with a severity and progress phases with a message, so each host can style, filter and summarize them in its own way.

### Live tool output

A host that wants to show the tools' output as it happens, such as in a console, adds `"logix/output": true` to the `_meta` of its tool call. The engine then sends each line the tools print as a `logix/output` notification while the tool runs, and all of them before the tool result. The stream is flat and in order: an operation that runs several tools, such as `build`, sends each tool's command line followed by that tool's lines, then the next tool's. Without the flag it sends none, so other MCP clients and AI agents never get them. MCP's own logging notifications aren't used, since MCP 2026-07-28 deprecates them.

**Table 22.** logix/output notification fields

| Field | Type | Description |
| --- | --- | --- |
| `request_id` | number or string | The JSON-RPC ID of the tool call the line belongs to |
| `tool` | text | The tool that printed it, such as `yosys`, `nextpnr-ice40` or `openFPGALoader` |
| `kind` | enum | `command` for the command line of a tool about to run, sent before its output, or `output` for a line the tool printed |
| `level` | enum | `error`, `warning` or `info`, from how the tool marks the line, so hosts can color it |
| `text` | text | The line as the tool printed it, without terminal codes. A progress bar redrawn with carriage returns gives one line per redraw |

Lines that the engine reads into diagnostics still come in the tool result too. The live lines are for showing, not for parsing.

**Verbose.** Tools that run other tools take an optional boolean `verbose`, like Apio's `--verbose`. It asks those tools to print more detail, which shows in the live output and the logs, and never changes the result. Each tool maps it to what its tools offer: `build` runs yosys and nextpnr without `-q`, `lint` runs Verilator without `--quiet`, `test` and `simulate` run iverilog with `-v`, and `program_board` adds openFPGALoader's `--verbose` or iceprog's `-v`. A build that is up to date runs nothing, so it prints nothing extra.

### Commands

Results of the tools that run programs (`lint`, `test`, `simulate`, `build` and `program_board`) list the programs they ran in `commands`, in order, and so does `data.commands` in an error from them. Like Apio's command listing, it lets anyone run a step again without the engine, such as to report a bug to the tool's developers. A build that is up to date runs nothing, so its list is empty.

**Table 30.** Command fields

| Field | Type | Description |
| --- | --- | --- |
| `tool` | text | The program's name, such as `yosys` |
| `package` | text | The package it came from and its version, such as `oss-cad-suite 2026-10-22`. Omitted for a program from elsewhere, such as a custom programmer command |
| `cwd` | absolute path | The folder it ran in |
| `args` | list of text | The program as an absolute path, then its arguments, exactly as passed |
| `shell` | text | One POSIX shell command line that runs it again the same way: in the same folder, with the same minimal environment, `PATH` (the package's `bin` folder, then `/usr/bin:/bin`), `HOME` and `TMPDIR` |

Example:

```json
{
  "tool": "verilator_bin",
  "package": "oss-cad-suite 2026-10-22",
  "cwd": "/home/u/proj",
  "args": ["/home/u/.logix/packages/oss-cad-suite/2026-10-22/bin/verilator_bin", "--lint-only", "--quiet", "main.v"],
  "shell": "cd /home/u/proj && env -i PATH=/home/u/.logix/packages/oss-cad-suite/2026-10-22/bin:/usr/bin:/bin HOME=\"$HOME\" TMPDIR=\"$TMPDIR\" /home/u/.logix/packages/oss-cad-suite/2026-10-22/bin/verilator_bin --lint-only --quiet main.v"
}
```

Files the engine generated for a step, such as lint's Verilator config in the engine folder's `tmp/` or a testbench's `.limit.v` in the build folder, are named by absolute path and stay where they are, so the line works as long as they do.

### Progress

A host that wants progress adds an MCP progress token to its request. The engine then sends progress notifications with a progress value, an optional total and an optional message, such as "Placing and routing".

Every operation that can take more than a second or two reports progress by phase:

- **Phase messages.** Each phase sends a message when it starts, such as "Installing oss-cad-suite", "Synthesizing", "Placing and routing", "Packing bitstream", "Finding device", "Writing flash" or "Verifying".
- **Percentages.** Within a phase, the engine also reports a percentage wherever the underlying tool or download prints one, such as package downloads and openFPGALoader writing and verifying flash. Phases without one send only their start message, and the host shows a spinner.
- **One increasing scale.** MCP requires the progress value to only increase within one operation. So when the operation starts, the engine decides its phases, including any package installs it needs. It sets total to 100 times the number of phases and reports progress as 100 times the phase index plus the phase's percentage. A host can show an overall bar from progress and total, and the current phase's percentage as progress modulo 100.
- **Unparsed output.** When the engine can't read a tool's output, such as with a custom `programmer_command`, that phase sends only its start message.

### Packages

Before running, each tool checks that the packages it needs from the manifest are installed and intact. It installs missing packages and repairs broken ones itself, as Apio does. A package is broken when its install record is missing or incomplete, or when one of its tools fails to start. Install progress is sent as the operation's progress notifications, and cancelling the operation cancels the install. A lock in the engine folder stops two instances from installing the same package at once.

The `--no-auto-install` command-line argument, described in [Command line](command-line.md), turns this off for offline or managed setups. A missing or broken package is then a setup error with `install_packages`, and so is an install that fails, such as when there's no network. To install, the host calls the [install_packages tool](#install_packages-tool), which works whether automatic install is on or off.

Install progress is one phase per package, such as "Installing oss-cad-suite". Its percentage covers the download (the first 80%) and then unpacking.

**Engine folder layout.** Hosts don't need to know it, but it helps when troubleshooting:

| Path in the engine folder | Contents |
| --- | --- |
| `packages/<name>/<version>/` | An installed package, such as `packages/oss-cad-suite/2026-10-22/`. Several versions can sit side by side. Its install record, `logix-install.json`, is written last |
| `downloads/` | Partial downloads, deleted when an install ends |
| `locks/` | One lock file per package, held while installing it |
| `logs/` | Log files, described in [Command line](command-line.md) |
| `tmp/` | Small files the engine generates for the tools, such as the Verilator config that turns off warnings in the FPGA models. Safe to delete |

A package is unpacked into a temporary folder and renamed into place only when it is complete, so an interrupted install never looks installed. Every file in a package must unpack inside its folder, or the install fails.

## lint tool

Checks the design and, optionally, its testbenches with Verilator in lint-only mode, like `apio lint`. It is much faster than a build and finds likely mistakes that synthesis accepts, such as width mismatches. It writes nothing and needs no build folder.

It runs Verilator with Apio's options, so it reports the same problems: `--lint-only --quiet --bbox-unsup --timing -Wno-TIMESCALEMOD -Wno-MULTITOP`, the macros `SYNTHESIZE` (unless `synthesize` is false) and `APIO_SIM=0`, the top module when given, the request's macros, and the FPGA's primitive models, such as `cells_sim.v` with `NO_ICE40_DEFAULT_ASSIGNMENTS` for iCE40, so designs that use primitives such as `SB_IO` lint cleanly. Verilator treats warnings as failures, as in Apio, so `passed` is false when there are warnings too. Warnings in the primitive models aren't the user's to fix, so the engine turns them off with a Verilator config, like Apio's `hardware.vlt`. Without it they would appear when `top_module` is omitted, since Verilator then checks every unused primitive in the models.

**Problems are the result.** Lint's job is to find problems, so problems in the design are returned in `diagnostics` with `passed` set to false, not as a design error. The other errors in the list below still apply.

**Table 23.** lint tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board specification | Yes | The target board. It selects the FPGA's primitive models |
| `top_module` | text | No | Name of the top-level module. When omitted, Verilator checks every module that no other module uses |
| `source_folder`, `source_files` | See folder fields in [Table 2](#folders) | Yes | The design files |
| `testbench_files` | list of relative paths | No | Testbenches to check too, relative to the source folder. Defaults to none. A file may be listed only once in `source_files` and `testbench_files` together |
| `defines` | map of text | No | Macro definitions, as in the [build tool](#build-tool). `SYNTHESIZE` is defined unless `synthesize` is false, and `APIO_SIM` is defined as `0` |
| `synthesize` | boolean | No | Define `SYNTHESIZE`, so `` `ifdef SYNTHESIZE `` picks the code for synthesis, as in a build. False leaves it undefined, to check the simulation code instead, like `apio lint --nosynth`. Defaults to true |
| `verbose` | boolean | No | Run Verilator without `--quiet`. Defaults to false |

**Table 24.** lint tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `passed` | boolean | True when Verilator found no errors and no warnings |
| `diagnostics` | list of diagnostics | The problems found, at file, line and column where known. A Verilator warning code, such as `WIDTHTRUNC`, ends the text in parentheses, since a `/* verilator lint_off WIDTHTRUNC */` comment names it. Problems in the primitive models keep their location in the text |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the engine. See [Commands](#commands) |

**Errors:** setup errors with `install_packages`. Request errors, such as an unknown board, a missing file or an included file outside the source folder. Tool failures, when Verilator fails without reporting a problem. Engine errors and cancellation.

**Includes.** The source file rules in [Table 8](#build-tool) apply. Verilator and Icarus Verilog don't look for an included file in the including file's folder as yosys does, so the engine passes the folder of every listed and included file as an include folder. An include is then found in the source folder first, then in those folders.

**Example**

Input:

```json
{
  "board": "alhambra-ii",
  "top_module": "main",
  "source_folder": "/home/u/proj",
  "source_files": ["main.v", "uart.v"],
  "testbench_files": ["main_tb.v"]
}
```

Output:

```json
{
  "passed": false,
  "diagnostics": [
    {
      "file": "main.v", "line": 6, "column": 9, "severity": "warning",
      "text": "Operator ASSIGNDLY expects 4 bits on the Assign RHS, but Assign RHS's VARREF 'cnt' generates 8 bits. (WIDTHTRUNC)",
      "tool": "verilator"
    }
  ]
}
```

## test tool

Runs testbenches with Icarus Verilog, like `apio test`. For each testbench in order, iverilog compiles the design files, the FPGA's primitive models and the testbench into the build folder, then vvp runs it and writes its waveform there. Every testbench runs, even after one fails. By default the engine stops each testbench at its first failure, so a failing run ends fast; see [Test and simulate modes](#test-and-simulate-modes).

It uses Apio's commands: `iverilog -g2012 -o <build folder>/<testbench>.out`, the request's macros, `APIO_SIM=0`, the FPGA's primitive models, the design files and the testbench, then `vvp -i <testbench>.out -dumpfile=<build folder>/<testbench>.vcd`. The `-i` isn't in Apio's command: it makes vvp print each line at once instead of when it exits, so the engine sees a failure while the testbench runs. It doesn't change the results. Both run in the source folder, so a testbench reads data files, such as with `$readmemh`, relative to it, as in Apio. `SYNTHESIZE` isn't defined, so designs can tell simulation from synthesis.

**Testbench rules:** see [Writing testbenches](#writing-testbenches). A testbench fails by calling `$error` or `$fatal`, or by printing a failure line from Apio's assertion macros. It must not call `$dumpfile`, which is a design error at that line.

**Failures are the result.** A failing testbench is returned in `testbenches` with `passed` set to false, not as an error. Compile errors are a design error, since nothing could run.

**Table 25.** test tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board specification | Yes | The target board. It selects the FPGA's primitive models |
| `source_folder`, `source_files`, `build_folder` | See folder fields in [Table 2](#folders) | Yes | The design files, without testbenches |
| `testbench_files` | list of relative paths | Yes | The testbenches to run, in order, relative to the source folder. At least one. A file may be listed only once in `source_files` and `testbench_files` together |
| `defines` | map of text | No | Macro definitions, as in the [build tool](#build-tool). `APIO_SIM` is always defined as `0` |
| `stop_at_first_failure` | boolean | No | Stop each testbench at its first failure, so a failing run ends fast; its output and waveform then end there. False runs each testbench to the end, to see every failure. Defaults to true |
| `max_sim_time_ns` | integer | No | Stop each testbench when simulated time reaches this many nanoseconds. Defaults to no limit. See [Runs that don't end](#runs-that-dont-end) |
| `timeout_seconds` | integer | No | Stop each testbench after this many seconds of real time. Defaults to 600; 0 means no limit |
| `verbose` | boolean | No | Run iverilog with `-v`. Defaults to false |

**Table 26.** test tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `passed` | boolean | True when every testbench passed |
| `testbenches` | list of testbench results | Each testbench's result, in the order given. See Table 27 |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the engine. See [Commands](#commands) |

**Table 27.** Testbench result fields

| Field | Type | Description |
| --- | --- | --- |
| `file` | relative path | The testbench, relative to the source folder |
| `passed` | boolean | True when it ended itself with `$finish` (`ending` is `finished`) without `$fatal`, `$error` or a failure line from Apio's assertion macros |
| `diagnostics` | list of diagnostics | Compile warnings, and the testbench's `$fatal`, `$error` and `$warning` messages at file and line. The failure lines of Apio's assertion macros are errors too, at file and line when they give one, such as `ERROR at main_tb.v:52: ...` and `*** ASSERTION FAILED at main_tb.v:52: ...`; `*** ASSERTION FAILED in main_tb ...` has no location |
| `output` | multiline text | What the testbench printed, such as with `$display`. When it printed more than 5,000 lines, the first and last 2,500, with a line saying how many were left out between them |
| `waveform_file` | relative path | The waveform (`.vcd`), relative to the build folder, when the testbench wrote one with `$dumpvars` |
| `ending` | enum | How the run ended: `finished` (the testbench ended itself, with `$finish` or `$fatal`), `first_failure`, `sim_time_limit` or `timeout` (the engine stopped it; its output and waveform end there), or `no_finish` (it ran out of things to simulate without calling `$finish`, which usually means it stopped early by mistake) |
| `failure_count` | integer | How many failures the testbench reported. `diagnostics` lists the first 100, each with its time; a check that fails again at another time is another failure |

**Errors:** design errors for compile errors and `$dumpfile`. Setup errors with `install_packages`. Request errors, such as an unknown board, a missing file or a build folder that breaks the keyword rules. Tool failures, engine errors and cancellation.

**Build folder files.** For a testbench `rtl/uart_tb.v`, the build folder gets `rtl/uart_tb.out` (the compiled simulation) and `rtl/uart_tb.vcd` (the waveform): the testbench's path without its extension, as in Apio. Each run replaces them. Like a build, the first run writes the `.logix-build` marker, so `clean_build` can delete the folder.

**Example**

Input:

```json
{
  "board": "alhambra-ii",
  "source_folder": "/home/u/proj",
  "source_files": ["main.v"],
  "testbench_files": ["main_tb.v"],
  "build_folder": "/home/u/proj/_build/default"
}
```

Output:

```json
{
  "passed": false,
  "testbenches": [
    {
      "file": "main_tb.v",
      "passed": false,
      "diagnostics": [
        { "file": "main_tb.v", "line": 10, "severity": "error", "text": "q is 3, expected 9", "tool": "vvp", "time_ns": 95 }
      ],
      "output": "ERROR: main_tb.v:10: q is 3, expected 9\n       Time: 95000  Scope: main_tb",
      "waveform_file": "main_tb.vcd",
      "ending": "first_failure",
      "failure_count": 1
    }
  ]
}
```

## simulate tool

Simulates one testbench and opens its waveform in GTKWave, like `apio sim --detach`. It compiles and runs the testbench as the [test tool](#test-tool) does, with the same commands, testbench rules and build folder files, except that `APIO_SIM` is defined as `1` and the testbench always runs to the end, so the whole waveform is there to look at. Then it makes sure there is a layout file and starts GTKWave fully detached with the waveform and the layout, and returns without waiting for it. For a layout the user saved, it adds `--rcvar=do_initial_zoom_fit 1` first, as Apio does; a default layout sets its own view, which that would replace. GTKWave keeps running after the operation and after the engine exits.

**Layout files.** A GTKWave layout (`.gtkw`) holds the signals shown and how. It is a project file the user changes by saving from GTKWave, which reads and saves the same path, so the host names it in `layout_file`, usually next to the testbench, such as `main_tb.gtkw`.

- **Missing or default:** when the file doesn't exist, or is a default one the engine or Apio generated (it has their marker line), the engine writes a default layout there with the testbench's top-level signals, so GTKWave doesn't open empty. The default layout also sets the view: the whole run when the testbench passed, or, when it failed, a window around the first failure that has a time, with GTKWave's marker on the failure about two thirds of the way across, so what led up to it shows too. The window is about 80 times the waveform's typical step between changes, roughly 40 cycles of a clock, and never wider than the run. A default is replaced on each simulation, so it follows design changes.
- **Saved by the user:** a layout without the marker is the user's and is never changed. GTKWave opens it, and saving from GTKWave updates it.
- **Unsaved defaults are cleaned up:** when GTKWave exits, the engine deletes the layout if it still has the marker, so default layouts the user never saved don't pile up in the project or end up in version control. This is best-effort: if the engine exits before GTKWave, the default stays until a later simulation replaces or deletes it.
- **No layout file:** when `layout_file` is omitted, the layout goes into the build folder as `<testbench>.gtkw`, where a saved layout is lost when the folder is cleaned.

This is the one case where the engine creates or deletes a file in the source folder. See [Safe file changes](requirements.md).

**A failing testbench** is still simulated and shown, since its waveform is how the user finds the problem. `passed` is false. A testbench that writes no waveform, because it doesn't call `$dumpvars`, is a design error.

**Viewer.** `open_viewer` set to false only writes the waveform and the layout, such as for an AI agent or when no one is at the screen. Opening a viewer needs a local frontend: a network adapter turns it off or refuses it for remote frontends, which view waveforms on their own.

**Table 28.** simulate tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board specification | Yes | The target board. It selects the FPGA's primitive models |
| `source_folder`, `source_files`, `build_folder` | See folder fields in [Table 2](#folders) | Yes | The design files, without testbenches |
| `testbench_file` | relative path | Yes | The testbench to simulate, relative to the source folder |
| `layout_file` | relative path | No | The GTKWave layout file, relative to the source folder, ending in `.gtkw`. Its folder must exist; the file may not. When omitted, the layout goes into the build folder |
| `open_viewer` | boolean | No | Start GTKWave on the waveform. Defaults to true |
| `max_sim_time_ns` | integer | No | Stop the testbench when simulated time reaches this many nanoseconds. Defaults to no limit |
| `timeout_seconds` | integer | No | Stop the testbench after this many seconds of real time. Defaults to no limit |
| `defines` | map of text | No | Macro definitions, as in the [build tool](#build-tool). `APIO_SIM` is always defined as `1` |
| `verbose` | boolean | No | Run iverilog with `-v`. Defaults to false |

**Table 29.** simulate tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `passed` | boolean | As in the testbench results of the test tool: true when the testbench reached `$finish` without a failure |
| `diagnostics` | list of diagnostics | As in the testbench results of the test tool, in [Table 27](#test-tool) |
| `output` | multiline text | What the testbench printed |
| `failure_count` | integer | How many failures the testbench reported, as in the test tool |
| `ending` | enum | How the run ended, as in the test tool. Never `first_failure` |
| `waveform_file` | relative path | The waveform (`.vcd`), relative to the build folder |
| `layout_file` | absolute path | The layout file GTKWave opens |
| `layout_created` | boolean | True if the engine wrote a default layout, false if it kept the user's |
| `viewer_opened` | boolean | True if GTKWave was started |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the engine. See [Commands](#commands) |

**Errors:** design errors for compile errors, `$dumpfile` and a testbench without `$dumpvars`. Setup errors with `install_packages`, or when the layout file can't be written. Request errors, such as a missing file, a layout file that isn't `.gtkw` or whose folder doesn't exist, or a build folder that breaks the keyword rules. Tool failures, such as when GTKWave can't start, engine errors and cancellation.

**Example**

Input:

```json
{
  "board": "alhambra-ii",
  "source_folder": "/home/u/proj",
  "source_files": ["main.v"],
  "testbench_file": "main_tb.v",
  "build_folder": "/home/u/proj/_build/default",
  "layout_file": "main_tb.gtkw"
}
```

Output:

```json
{
  "passed": true,
  "diagnostics": [],
  "output": "main_tb passed\nmain_tb.v:12: $finish called at 95000 (1ps)",
  "failure_count": 0,
  "ending": "finished",
  "waveform_file": "main_tb.vcd",
  "layout_file": "/home/u/proj/main_tb.gtkw",
  "layout_created": true,
  "viewer_opened": true
}
```

## Test and simulate modes

The same testbench serves two purposes: `test` is a batch check that should fail fast, and `simulate` produces a waveform the user can look at to find the problem. The testbench only states what is correct; the engine applies each mode's policy:

| | `test` | `simulate` |
| --- | --- | --- |
| At the first failure | Stops the testbench, unless `stop_at_first_failure` is false | Lets it run to the end |
| Waveform | Kept, up to the failure | Complete |
| Finding the failure | Each failure's file, line and simulation time (`time_ns`) | The same, and a default layout opens GTKWave zoomed in around the first failure, with its marker on it |
| Limits | `timeout_seconds` (default 600) and `max_sim_time_ns` | The same, with no default |
| `passed` | False after any failure, or when stopped by a limit | The same |

### Runs that don't end

A testbench that never calls `$finish`, such as one with a free-running clock, or one stuck in a loop, would run forever. Two limits stop it, and a testbench stopped by either fails, with `ending` saying which:

- **`max_sim_time_ns`** stops it when simulated time passes the limit, so the result is the same on every computer. The engine compiles a small extra module, `<testbench>.limit.v` in the build folder, that ends the simulation at that time; Icarus runs it alongside the testbench without changes to the user's files. Its 1 ns time unit makes the waveform's time unit at most 1 ns.
- **`timeout_seconds`** stops it after that much real time. It also catches a loop that doesn't advance simulated time. `test` stops after 10 minutes unless told otherwise, so an AI agent or CI run can't hang; `simulate` has no default, since someone is watching.

A testbench that runs out of things to simulate without calling `$finish` ends with `no_finish` and fails too: it usually means it stopped early by mistake.

To stop a testbench, the engine asks vvp to exit, which writes out the waveform so far. Any tool the engine stops, such as when the host cancels, is asked to exit first and killed if it hasn't after two seconds.

## Writing testbenches

These are the engine's guidelines for testbenches. Testbenches written for Apio keep working: `APIO_SIM` is still defined (0 in `test` and `lint`, 1 in `simulate`), and the failure lines of Apio's assertion macros count as failures.

1. **Report a failure with `$error`, then keep going.** Put the expected and actual values in the message. The engine reads `$error` with its file, line and simulation time, and decides whether to stop.
2. **Use `$fatal` only when going on makes no sense,** such as a missing data file. It stops the run in both modes.
3. **Don't check the mode.** No `` if (!`APIO_SIM) $fatal ``: the engine stops `test` at the first failure and lets `simulate` run to the end.
4. **End with `$finish`.** A testbench that runs out of things to do without it fails, and one that never ends is stopped by a [limit](#runs-that-dont-end) and fails.
5. **Call `$dumpvars(0, <testbench module>)` at the start** to write a waveform, and **never call `$dumpfile`**: the engine decides where the waveform goes.
6. **Read data files relative to the source folder,** such as `$readmemh("data/rom.hex", rom)`. Testbenches run there.
7. **`SYNTHESIZE` isn't defined in `test` and `simulate`,** so `` `ifdef SYNTHESIZE `` / `` `else `` selects the simulation code.
8. **Name testbenches `<name>_tb.v` or `<name>_tb.sv`** and give them a `` `timescale ``. The engine doesn't need the names, since requests list the testbenches, but Apio and hosts that follow its naming find them that way.

**Example**

```verilog
`timescale 1ns/1ps

// Reports a failed check with the actual and expected values. Not fatal.
`define CHECK(actual, want) if ((actual) !== (want)) $error("%s is %0h, expected %0h", `"actual`", (actual), (want));

module counter_tb;
  reg clk = 0;
  wire [3:0] q;
  counter dut(.clk(clk), .q(q));
  always #5 clk = ~clk;

  initial begin
    $dumpvars(0, counter_tb);
    #95  `CHECK(q, 4'd9)
    #10  `CHECK(q, 4'd10)
    $display("End of simulation");
    $finish;
  end
endmodule
```

A failed check reports, for example, `q is a, expected b` at `counter_tb.v:15` with its time. Two Icarus Verilog pitfalls when writing such a macro: keep it on one line, since iverilog reports wrong line numbers after a macro continued over several lines, and don't use a parameter's name inside the message string, since iverilog replaces it there too.

## build tool

Synthesizes the design, places and routes it, and generates the bitstream in the build folder. A build with no changes since the last one returns quickly with `up_to_date` set.

For iCE40 it runs yosys (`synth_ice40`), nextpnr-ice40 and icepack with the same commands as Apio, so the bitstreams match Apio's for the same package versions.

**Table 6.** build tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board specification | Yes | The target board |
| `top_module` | text | Yes | Name of the top-level module |
| `source_folder`, `source_files`, `constraint_file`, `build_folder` | See folder fields in [Table 2](#folders) | Yes | The constraint file is required |
| `defines` | map of text | No | Verilog and SystemVerilog macro definitions, from name to value, such as `{ "USE_PLL": "", "WIDTH": "8" }`. Names follow Verilog identifier rules, and a bad name is a request error. An empty value defines the macro with no text, for use with `` `ifdef ``. The engine always passes each value explicitly, so every tool sees the same text, and passes them in name order, so the same request always gives the same tool commands. `SYNTHESIZE` is always defined, as in Apio |
| `verbose` | boolean | No | Let yosys and nextpnr print more detail, shown in the live output and the logs. Defaults to false. See [Verbose](#live-tool-output) |

**Table 7.** build tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `bitstream` | relative path | Path of the bitstream, relative to the build folder |
| `up_to_date` | boolean | True if nothing had to be rebuilt |
| `resources_usage` | map of resource usage | Resource use, keyed by the toolchain's resource names. For iCE40: `ICESTORM_LC` (logic cells, each a LUT and a flip-flop), `ICESTORM_RAM` (block RAMs), `SB_IO` (I/O pins), `SB_GB` (global buffers), `ICESTORM_PLL` and others. Each value has integer `used` and `available` fields |
| `timing` | map of numbers | Maximum frequency in MHz for each clock net, keyed by the clock net name nextpnr reports, rounded to 0.01 MHz |
| `warnings` | list of diagnostics | Warnings from the tools. See diagnostic fields in [Table 4](#results-and-errors) |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the engine. See [Commands](#commands) |

**Errors:** design errors, such as syntax errors or unknown pin names. Setup errors with `install_packages`. Request errors, such as an unknown board or a build folder that breaks the keyword rules. Tool failures, engine errors and cancellation. A clock that misses its intended frequency is not an error: the host compares the frequencies in `timing` with its own targets.

**Table 8.** build tool source file rules

| Topic | Rule |
| --- | --- |
| Include lookup | A Verilog `` `include `` is looked up first in the folder of the including file, then in `source_folder`. Included files don't need to be in `source_files` |
| Include location | Included files must be under `source_folder` and pass the same path checks as other source files. A file outside it is a request error |
| Unlisted modules | The engine doesn't search for modules. A module defined in a file that isn't in `source_files` is a design error that names the missing module |
| `up_to_date` | A build is up to date when the engine version, the toolchain package, the board, the request's fields and the contents of every source, included and constraint file are the same as in the last successful build, and its bitstream is still there. Changing any of them makes the next build run again |
| Testbenches | The same rules apply to testbench files |

**Build record.** When a build succeeds, the engine writes a build record into the build folder. It holds the full board spec the build used and its checksum. The `program_board` tool reads it to get the board. A build that fails removes the old record, so `program_board` can't program a stale bitstream.

**Build folder files.** The engine creates the build folder if needed, and first writes a marker file, `.logix-build`, which tells the [clean_build tool](#clean_build-tool) the folder is a build folder. A build makes `hardware.json` (synthesized netlist), `hardware.asc` (placed and routed design), `hardware.pnr` (nextpnr's report), `hardware.bin` (the bitstream), the tool logs `synth.log` and `pnr.log`, and the build record `build-record.json`, written last. These names are a fixed engine convention, the same for every build and host, taken from Apio. Each build replaces the files the previous build made, such as the bitstream, intermediate files and reports. When a build has to run again, it deletes the previous build's outputs first, so a failed build never leaves an old bitstream that looks current. An up-to-date build keeps them. The engine never deletes files in the build folder that it didn't make, such as files put there by the host or the user.

**Example**

Input:

```json
{
  "board": "alhambra-ii",
  "top_module": "main",
  "source_folder": "/home/u/proj",
  "source_files": ["main.v", "uart.v"],
  "constraint_file": "pinout.pcf",
  "build_folder": "/home/u/proj/_build/default"
}
```

Output:

```json
{
  "bitstream": "hardware.bin",
  "up_to_date": false,
  "resources_usage": {
    "ICESTORM_LC": { "used": 412, "available": 7680 },
    "ICESTORM_PLL": { "used": 0, "available": 2 },
    "ICESTORM_RAM": { "used": 2, "available": 32 },
    "SB_GB": { "used": 1, "available": 8 },
    "SB_IO": { "used": 5, "available": 107 },
    "SB_WARMBOOT": { "used": 0, "available": 1 }
  },
  "timing": { "CLK$SB_IO_IN_$glb_clk": 87.4 },
  "warnings": [
    { "file": "uart.v", "line": 12, "severity": "warning", "text": "Identifier `\\rx_done' is implicitly declared.", "tool": "yosys" }
  ]
}
```

## program_board tool

Programs a bitstream into the board it was built for. The `program_board` tool takes no board input. It reads the board spec from the build record in the build folder, so the board always matches the build. The engine finds exactly one matching device, as described in the [device selection](requirements.md#device-selection) requirements, fills in the argument template of the board's default programming variant and runs it.

**Note:** unlike `apio upload`, the `program_board` tool doesn't build. It programs the bitstream that is already in the build folder. A host that wants to build and program the board in one step calls `build` first, and calls `program_board` only if the build succeeded.

**How it runs:** the engine reads the board from the build record, finds the one matching device, fills in the variant's argument template, splitting it into words first so paths with spaces stay whole, and runs the programmer in the build folder. Progress has the phases "Finding device", "Writing flash" and "Verifying", with percentages read from the programmer's output. With `programmer_command` the phases are "Finding device" and "Programming", without percentages. A programmer that fails because the device is missing or can't be opened gives a setup error with `connect_board` or `fix_device_permissions`; other failures are tool failures with the programmer's output in `details`. The build record's board checksum is checked first, so a damaged record is a request error.

**Flash or SRAM:** the board's default programming variant decides whether the bitstream goes to flash, where it stays after power-off, or to SRAM, where it is lost at power-off. The tool has no input yet for choosing another variant.

**Table 9.** program_board tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `build_folder` | build folder path | Yes | The build folder of a successful build. It holds the bitstream and the build record |
| `serial_number` | text | No | Narrows the match to the device with this serial number |
| `programmer_command` | text | No | A custom command template that replaces the default variant's whole command, like Apio's `programmer_cmd`. Its first word is a program from the toolchain package, such as `openFPGALoader`, or an absolute path. It uses the same variables as argument templates and must use `${BITSTREAM}` |
| `verbose` | boolean | No | Let the programmer print more detail. Not applied to a `programmer_command`. Defaults to false |

**Table 10.** program_board tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `device` | device | The device that was programmed. See device fields in [Table 13](#list_devices-tool) |
| `programmer` | catalog ID | The programmer that was used, such as `openfpgaloader`, or `custom` when `programmer_command` was given |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the engine. See [Commands](#commands) |

**Errors:** setup errors with `connect_board` when no device matches, `select_device` when several match, `fix_device_permissions` or `install_drivers`. With `select_device`, `data.candidates` lists the matching devices so the host can let the user pick one and retry. A request error when the build folder has no build record. Other request errors, tool failures, engine errors and cancellation.

**Example of a `select_device` error**

```json
{
  "category": "setup",
  "message": "Found 2 matching devices.",
  "action": { "code": "select_device", "description": "Choose one device and retry with its serial number." },
  "data": {
    "candidates": [
      { "kind": "usb", "vid": "0403", "pid": "6010", "product": "Dual RS232-HS", "serial_number": "FT4ABCD1" },
      { "kind": "usb", "vid": "0403", "pid": "6010", "product": "Dual RS232-HS", "serial_number": "FT4ABCD2" }
    ]
  }
}
```

## list_devices tool

Lists the connected USB devices, optionally marking the ones that match the USB filter of a board's default programming variant. The engine reads USB devices with libusb, built into the engine, the same library the programmers use, so the bus and device numbers mean the same to both. Devices are sorted by vendor ID, product ID, bus and device number. A device matches a filter when its USB IDs are equal and, when the filter has `product_regex`, its product name contains a match for it. Names that can't be read, such as for a device that can't be opened, are left out.

**Table 11.** list_devices tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board specification | No | When given, each device reports whether it matches the USB filter of this board's default variant |

**Table 12.** list_devices tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `devices` | list of devices | The connected devices. See device fields in Table 13 |

**Table 13.** Device fields

| Device field | Type | Description |
| --- | --- | --- |
| `kind` | enum | Always `usb` for now. Other kinds may be added later |
| `vid`, `pid` | USB ID | USB vendor and product IDs |
| `manufacturer`, `product` | text | Names reported by the device, when available |
| `serial_number` | text | Serial number, when available |
| `bus`, `device` | integer | USB bus and device number, for USB devices |
| `matches_board` | boolean | Whether the device matches the given board, when one was given |

**Errors:** setup errors with `install_drivers` or `fix_device_permissions` when devices cannot be scanned. Engine errors.

**Example**

```json
{
  "devices": [
    {
      "kind": "usb", "vid": "0403", "pid": "6010",
      "manufacturer": "Lattice", "product": "Lattice FTUSB Interface Cable",
      "serial_number": "FT4ABCD1", "bus": 1, "device": 7, "matches_board": true
    }
  ]
}
```

## get_catalog tool

Returns the engine's read-only catalog. Hosts use it to list boards and to merge it with their own custom definitions.

**Table 14.** get_catalog tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `boards` | catalog query | No | Which boards to return. Omitted means none |
| `fpgas` | catalog query | No | Which FPGAs to return. Omitted means none |

**Table 15.** get_catalog tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `boards` | map of board definitions | Board definitions by catalog ID |
| `fpgas` | map of FPGA definitions | FPGA definitions by catalog ID |

**Errors:** request errors for unknown sections or IDs. Engine errors.

**Example**

Input:

```json
{
  "boards": { "fetch": true, "ids": ["alhambra-ii"] }
}
```

Output:

```json
{
  "boards": {
    "alhambra-ii": {
      "description": "Alhambra II",
      "fpga_id": "ice40hx4k-tq144-8k",
      "default_variant": "flash",
      "variants": {
        "flash": {
          "programmer": "openfpgaloader",
          "args": "--force-terminal-mode --verify -b ice40_generic --vid ${VID} --pid ${PID} --busdev-num ${BUS}:${DEV} ${BITSTREAM}",
          "usb": { "vid": "0403", "pid": "6010", "product_regex": "^Alhambra II.*" }
        }
      }
    }
  },
  "fpgas": {}
}
```

**Catalog contents.** The catalog is data in the engine's repo, `catalog/boards.json` and `catalog/fpgas.json`, imported from Apio's definitions. Milestone 1 has every iCE40 FPGA and the 22 iCE40 boards that program with `openfpgaloader` or `iceprog`. The other iCE40 boards and the other architectures are added as their programmers and toolchains are supported.

## get_engine_info tool

Returns the engine's version, platform and package status. A host calls it when it starts, to check that the engine is new enough and to show what is installed. It is fast and local: it never uses the network and doesn't verify checksums.

**Table 16.** get_engine_info tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| (none) | | | The tool takes no inputs |

**Table 17.** get_engine_info tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `engine_version` | date tag | The engine release |
| `protocol_versions` | list of text | The MCP protocol versions the engine accepts, such as 2026-07-28 and 2025-11-25 |
| `platform` | text | The platform the engine runs on, such as `darwin-arm64` |
| `engine_folder` | absolute path | The engine folder, set by the host with the `--engine-folder` command-line argument when it launches the engine |
| `auto_install` | boolean | Whether automatic package install and repair is on. It is on by default, and the host turns it off with the `--no-auto-install` command-line argument when it launches the engine |
| `command_line` | list of text | The command-line arguments the engine was launched with, as given, for diagnostics |
| `log_file` | absolute path | The log file this engine process writes, in the `logs` subfolder of the engine folder |
| `packages` | list of packages | Every package in the engine's manifest. Each has a text `name`, a date tag `version` and a `status`: `installed`, `missing` or `broken` |

**Errors:** engine errors only.

**Example**

Output:

```json
{
  "engine_version": "2026-12-01",
  "protocol_versions": ["2026-07-28", "2025-11-25"],
  "platform": "darwin-arm64",
  "engine_folder": "/Users/u/.apio/engine",
  "auto_install": true,
  "command_line": ["--engine-folder", "/Users/u/.apio/engine"],
  "log_file": "/Users/u/.apio/engine/logs/20261028-4123.log",
  "packages": [
    { "name": "oss-cad-suite", "version": "2026-11-20", "status": "installed" },
    { "name": "examples", "version": "2026-10-15", "status": "missing" }
  ]
}
```

## install_packages tool

Installs or repairs the engine's packages. Packages that are already installed and intact are left alone, unless `force` is set. Other tools install the packages they need on their own, so a host calls this only to install ahead of time, such as before going offline, or to act on the `install_packages` action, such as when automatic install is off. It installs whether or not automatic install is on, since the host asked. It reports progress, one phase per package, and can be cancelled.

**Table 18.** install_packages tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `packages` | list of text | No | Names of the packages to install or repair, as `get_engine_info` lists them. Omitted means every package in the manifest |
| `force` | boolean | No | Download and install the packages again even if they are installed, replacing the installed copies, like Apio's `apio packages install --force`. It fixes files that were damaged or deleted after the install, which the status check doesn't see. The installed copy stays until the new one is complete. Defaults to false |

**Table 19.** install_packages tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `installed` | list of text | Names of the packages this call installed, repaired or reinstalled. Empty when they were all installed already and `force` was off |
| `packages` | list of packages | Every package in the manifest with its status afterwards, as in `get_engine_info` |

**Errors:** request errors for unknown package names. Setup errors with `install_packages` when a download or install fails. Engine errors and cancellation.

**Example**

Input:

```json
{}
```

Output:

```json
{
  "installed": ["oss-cad-suite"],
  "packages": [
    { "name": "oss-cad-suite", "version": "2026-10-22", "status": "installed" }
  ]
}
```

## clean_build tool

Deletes build folders, like `apio clean`. It is given a folder and deletes only build folders, which the engine marks with a `.logix-build` file when it first builds in them:

- **A build folder,** such as `/home/u/proj/_build/default`: the tool deletes it with everything in it.
- **A folder of build folders,** such as `/home/u/proj/_build` with one build folder per Apio env: the tool deletes each build folder directly inside it, then deletes the folder itself if nothing else is left in it.

It never deletes a folder without the marker or files outside the build folders, so a wrong path from a host can't remove the user's files. The folder must satisfy the build folder path rules. A folder that doesn't exist is not an error: nothing is deleted.

**Table 20.** clean_build tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `folder` | build folder path | Yes | A build folder, or a folder whose direct subfolders are build folders |

**Table 21.** clean_build tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `deleted` | list of absolute paths | The folders that were deleted, including the given folder when it was deleted. Empty when there was nothing to delete |

**Errors:** request errors when the folder breaks the path rules. Setup errors when a file can't be deleted, such as when it's in use. Engine errors.

**Example**

Input:

```json
{ "folder": "/home/u/proj/_build" }
```

Output:

```json
{
  "deleted": [
    "/home/u/proj/_build/default",
    "/home/u/proj/_build/icezum",
    "/home/u/proj/_build"
  ]
}
```

## Open questions

- **Skipping build steps whose inputs didn't change.** Today a build is up to date as a whole, by a checksum of all its inputs, and otherwise reruns synthesis, place and route, and packing. Apio, with SCons, skips steps whose own inputs are unchanged, so a change to only the constraint file skips synthesis. Per-step checksums in the build record would do the same. Most edits change the sources, which rerun every step anyway, so this waits until large designs make pin-only rebuilds slow.
- **What clean_build may delete.** Today it deletes only folders the engine marked with `.logix-build`, so it leaves old build folders made by Apio alone until the engine builds in them. The alternative is to delete any folder that passes the build folder path checks, as `apio clean` does. That cleans Apio's old folders too, but a wrong path such as `/home/u/builds/myproject` passes those checks, since one of its last two folder names contains "build". A middle way would be to also refuse folders that directly contain design files, such as `.v`, `.sv` or `.pcf` files.
- **Extra tool arguments for build.** Apio lets users pass extra options to the synthesis and place-and-route tools, so the build tool likely needs something similar. One option is to define abstract build phases, such as synthesis, place and route, and bitstream, offer portable options such as a target frequency and a seed, and add extra arguments per phase as an explicit escape hatch, passed unchanged to whatever tool runs that phase. The build output would then report which tool and version ran each phase.
- **Include folders for build.** Some projects keep header files in a separate folder. An optional `include_folders` input, a list of relative paths under `source_folder`, would let the host name extra places to look up included files.
- **Separating simulation from viewing.** Today `simulate` writes the waveform and also opens GTKWave. Longer term, `simulate` could only write the waveform and return the top-level signals, with opening a viewer as an optional step or its own tool. GTKWave layouts would then be one viewer's concern, and other viewers, such as Surfer or a waveform view inside an IDE, would fit the same way. It only adds fields and tools, so it breaks nothing.
- **Choosing a programming variant.** Boards already define several named programming variants, such as flash and sram, or different cables, such as FT232 and FT2232, each with its own argument template and USB filter. For now, `program_board` and `list_devices` always use the default variant. Later, an optional input could name another variant, and asking for one the board doesn't have would be a request error. Adding it later doesn't break existing hosts.
