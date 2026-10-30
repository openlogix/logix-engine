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
| board definition | object | A board entry in the catalog format, with its description, its FPGA, a map of programming variants keyed by variant name, and the name of its default variant |
| FPGA definition | object | An FPGA entry in the catalog format, with its architecture, part number, size and toolchain parameters |
| programming variant | object | One way to program a board, such as flash or sram, or a particular cable, such as FT232 or FT2232. It names the programmer to run, such as `openfpgaloader`, and gives the full argument template for it, with variables such as `${VID}`, `${PID}`, `${BUS}`, `${DEV}`, `${SERIAL}` and `${BITSTREAM}`, and the USB filter that finds the device. The engine maps the programmer name to its executable and the package that provides it, so there are no separate programmer definitions |
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
| `file` | relative path | File, relative to the source folder |
| `line` | integer | Line number, starting at 1 |
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

### Progress

A host that wants progress adds an MCP progress token to its request. The engine then sends progress notifications with a progress value, an optional total and an optional message, such as "Placing and routing".

Every operation that can take more than a second or two reports progress by phase:

- **Phase messages.** Each phase sends a message when it starts, such as "Installing oss-cad-suite", "Synthesizing", "Placing and routing", "Packing bitstream", "Finding device", "Writing flash" or "Verifying".
- **Percentages.** Within a phase, the engine also reports a percentage wherever the underlying tool or download prints one, such as package downloads and openFPGALoader writing and verifying flash. Phases without one send only their start message, and the host shows a spinner.
- **One increasing scale.** MCP requires the progress value to only increase within one operation. So when the operation starts, the engine decides its phases, including any package installs it needs. It sets total to 100 times the number of phases and reports progress as 100 times the phase index plus the phase's percentage. A host can show an overall bar from progress and total, and the current phase's percentage as progress modulo 100.
- **Unparsed output.** When the engine can't read a tool's output, such as with a custom `programmer_command`, that phase sends only its start message.

### Packages

Before running, each tool checks that the packages it needs from the manifest are installed and intact. It installs missing packages and repairs broken ones itself, as Apio does. A package is broken when its install record is missing or incomplete, or when one of its tools fails to start. Install progress is sent as the operation's progress notifications, and cancelling the operation cancels the install. A lock in the engine folder stops two instances from installing the same package at once.

The `--no-auto-install` command-line argument, described in [Command line](command-line.md), turns this off for offline or managed setups. A missing or broken package is then a setup error with `install_packages`, and so is an install that fails, such as when there's no network.

## build tool

Synthesizes the design, places and routes it, and generates the bitstream in the build folder. A build with no changes since the last one returns quickly with `up_to_date` set.

**Table 6.** build tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board specification | Yes | The target board |
| `top_module` | text | Yes | Name of the top-level module |
| `source_folder`, `source_files`, `constraint_file`, `build_folder` | See folder fields in [Table 2](#folders) | Yes | The constraint file is required |
| `defines` | map of text | No | Verilog and SystemVerilog macro definitions, from name to value, such as `{ "USE_PLL": "", "WIDTH": "8" }`. Names follow Verilog identifier rules, and a bad name is a request error. An empty value defines the macro with no text, for use with `` `ifdef ``. The engine always passes each value explicitly, so every tool sees the same text, and passes them in name order, so the same request always gives the same tool commands |

**Table 7.** build tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `bitstream` | relative path | Path of the bitstream, relative to the build folder |
| `up_to_date` | boolean | True if nothing had to be rebuilt |
| `resources_usage` | map of resource usage | Resource use, keyed by resource name, such as LUTs and flip-flops. Each value has integer `used` and `available` fields |
| `timing` | map of numbers | Maximum frequency in MHz for each clock net, keyed by clock net name |
| `warnings` | list of diagnostics | Warnings from the tools. See diagnostic fields in [Table 4](#results-and-errors) |

**Errors:** design errors, such as syntax errors or unknown pin names. Setup errors with `install_packages`. Request errors, such as an unknown board or a build folder that breaks the keyword rules. Tool failures, engine errors and cancellation. A clock that misses its intended frequency is not an error: the host compares the frequencies in `timing` with its own targets.

**Table 8.** build tool source file rules

| Topic | Rule |
| --- | --- |
| Include lookup | A Verilog `` `include `` is looked up first in the folder of the including file, then in `source_folder`. Included files don't need to be in `source_files` |
| Include location | Included files must be under `source_folder` and pass the same path checks as other source files. A file outside it is a request error |
| Unlisted modules | The engine doesn't search for modules. A module defined in a file that isn't in `source_files` is a design error that names the missing module |
| `up_to_date` | Included files count as inputs. Changing one makes the next build run again |
| Testbenches | The same rules apply to testbench files |

**Build record.** When a build succeeds, the engine writes a build record into the build folder. It holds the full board spec the build used and its checksum. The `program_board` tool reads it to get the board. A build that fails removes the old record, so `program_board` can't program a stale bitstream.

**Example**

Input:

```json
{
  "board": "icestick",
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
    "LUTs": { "used": 412, "available": 1280 },
    "Flip-flops": { "used": 198, "available": 1280 },
    "Block RAMs": { "used": 0, "available": 16 }
  },
  "timing": { "clk": 87.4, "uart_clk": 142.9 },
  "warnings": []
}
```

## program_board tool

Programs a bitstream into the board it was built for. The `program_board` tool takes no board input. It reads the board spec from the build record in the build folder, so the board always matches the build. The engine finds exactly one matching device, as described in the [device selection](requirements.md#device-selection) requirements, fills in the argument template of the board's default programming variant and runs it.

**Note:** unlike `apio upload`, the `program_board` tool doesn't build. It programs the bitstream that is already in the build folder. A host that wants to build and program the board in one step calls `build` first, and calls `program_board` only if the build succeeded.

**Flash or SRAM:** the board's default programming variant decides whether the bitstream goes to flash, where it stays after power-off, or to SRAM, where it is lost at power-off. The tool has no input yet for choosing another variant.

**Table 9.** program_board tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `build_folder` | build folder path | Yes | The build folder of a successful build. It holds the bitstream and the build record |
| `serial_number` | text | No | Narrows the match to the device with this serial number |
| `programmer_command` | text | No | A custom command template that replaces the default variant's command |

**Table 10.** program_board tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `device` | device | The device that was programmed. See device fields in [Table 13](#list_devices-tool) |
| `programmer` | catalog ID | The programmer that was used, such as `openfpgaloader` |

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

Lists the connected USB devices, optionally marking the ones that match the USB filter of a board's default programming variant.

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
  "boards": { "fetch": true, "ids": ["icestick"] }
}
```

Output:

```json
{
  "boards": {
    "icestick": {
      "description": "iCEstick Eval Kit",
      "fpga_id": "ice40hx1k-tq144",
      "default_variant": "flash",
      "variants": {
        "flash": {
          "programmer": "openfpgaloader",
          "args": "-b ice40_generic --vid ${VID} --pid ${PID} --busdev-num ${BUS}:${DEV} ${BITSTREAM}",
          "usb": { "vid": "0403", "pid": "6010" }
        }
      }
    }
  }
}
```

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

## Open questions

- **Extra tool arguments for build.** Apio lets users pass extra options to the synthesis and place-and-route tools, so the build tool likely needs something similar. One option is to define abstract build phases, such as synthesis, place and route, and bitstream, offer portable options such as a target frequency and a seed, and add extra arguments per phase as an explicit escape hatch, passed unchanged to whatever tool runs that phase. The build output would then report which tool and version ran each phase.
- **Include folders for build.** Some projects keep header files in a separate folder. An optional `include_folders` input, a list of relative paths under `source_folder`, would let the host name extra places to look up included files.
- **Choosing a programming variant.** Boards already define several named programming variants, such as flash and sram, or different cables, such as FT232 and FT2232, each with its own argument template and USB filter. For now, `program_board` and `list_devices` always use the default variant. Later, an optional input could name another variant, and asking for one the board doesn't have would be a request error. Adding it later doesn't break existing hosts.
