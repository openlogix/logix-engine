# Tool API (Milestone 1)

This is the engine's tool API for Milestone 1. It is not frozen yet: the rule that the tool API never breaks applies from the first release. Each tool is an MCP tool. Its inputs are the tool's arguments, and its outputs are the tool's structured content. The tools and fields are settled for Milestone 1. The exact JSON schemas and tool descriptions will be written while it is coded.

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
| error, diagnostic, action, device, resource | object | See error fields in [Table 3](#results-and-errors), diagnostic fields in [Table 4](#results-and-errors), action codes in [Table 5](#results-and-errors), device fields in [Table 13](#list_devices-tool), and resource fields in [Table 7](#build-tool) |

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
| `data` | object | Optional extra data, such as candidate devices |

**Table 4.** Diagnostic fields

| Diagnostic field | Type | Description |
| --- | --- | --- |
| `file` | relative path | File, relative to the source folder, when known. Many tool messages have no location |
| `line` | integer | Line number, starting at 1, when known |
| `column` | integer | Optional column number, starting at 1 |
| `severity` | enum | `error`, `warning` or `info` |
| `text` | text | The problem, in words |
| `tool` | text | Optional name of the tool that reported it |

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

A host that wants to show the tools' output as it happens, such as in a console, adds `"logix/output": true` to the `_meta` of its tool call. The engine then sends each line the tools print as a `logix/output` notification while the tool runs, and all of them before the tool result. Without the flag it sends none, so other MCP clients and AI agents never get them. MCP's own logging notifications aren't used, since MCP 2026-07-28 deprecates them.

**Table 22.** logix/output notification fields

| Field | Type | Description |
| --- | --- | --- |
| `request_id` | number or string | The JSON-RPC ID of the tool call the line belongs to |
| `tool` | text | The tool that printed it, such as `yosys`, `nextpnr-ice40` or `openFPGALoader` |
| `level` | enum | `error`, `warning` or `info`, from how the tool marks the line, so hosts can color it |
| `text` | text | The line as the tool printed it, without terminal codes. A progress bar redrawn with carriage returns gives one line per redraw |

Lines that the engine reads into diagnostics still come in the tool result too. The live lines are for showing, not for parsing.

**Verbose.** Tools that run other tools take an optional boolean `verbose`, like Apio's `--verbose`. It asks those tools to print more detail, which shows in the live output and the logs, and never changes the result. Each tool maps it to what its tools offer: `build` runs yosys and nextpnr without `-q`, and `program_board` adds openFPGALoader's `--verbose` or iceprog's `-v`. A build that is up to date runs nothing, so it prints nothing extra.

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

A package is unpacked into a temporary folder and renamed into place only when it is complete, so an interrupted install never looks installed. Every file in a package must unpack inside its folder, or the install fails.

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
- **Choosing a programming variant.** Boards already define several named programming variants, such as flash and sram, or different cables, such as FT232 and FT2232, each with its own argument template and USB filter. For now, `program_board` and `list_devices` always use the default variant. Later, an optional input could name another variant, and asking for one the board doesn't have would be a request error. Adding it later doesn't break existing hosts.
