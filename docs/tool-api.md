# Tool API

The Logix Engine is an MCP server that offers FPGA tools: lint, test, simulate, format, diagram, build, program a board, list devices, compare boards and more. This document describes those tools for the people who use them:

- **AI agent users.** An AI agent connected to the Logix Engine reads the tool names, descriptions and schemas itself and decides which tools to call. You mostly need the concepts: boards and board variants, folders, testbenches, and what results and errors mean.
- **Application developers.** A hardcoded client, such as the Logix IDE, launches the Logix Engine, calls its tools with JSON arguments over MCP and reads the JSON results. You need the full reference: every input and output field, the errors, and the progress and live output notifications.

In this document, the *host* is the program that talks to the Logix Engine: your application, or your AI agent's MCP client.

This is the engine's tool API: every tool it offers, with its inputs, outputs and errors. The API can still change before the first release. After that, changes only add tools and fields, so existing clients keep working. Each tool is an MCP tool. Its inputs are the tool's arguments, and its outputs are the tool's structured content. The exact JSON schemas and tool descriptions come from the MCP tools/list method, with a published copy in [schemas/tools.json](https://github.com/openlogix/logix-engine/blob/main/schemas/tools.json).

## How an MCP server works

The Logix Engine is a program, `logix-engine`, that a host runs and talks to. It has no window and no network port. Everything goes through the Model Context Protocol (MCP), an open standard for calling a program's functions with JSON.

1. **Launch.** The host starts the Logix Engine as a child process, with command-line arguments such as `--engine-folder` (see Command line). For an AI agent, this launch command is part of the agent app's MCP server settings.
2. **Channel.** The host writes requests to the engine's standard input and reads responses from its standard output. Each message is one JSON-RPC 2.0 document on a single line.
3. **Handshake.** The host sends `initialize`, and the two sides agree on an MCP protocol version (the Logix Engine accepts 2026-07-28 and 2025-11-25). The engine's reply carries its name and version, the website URL, and short instructions for AI agents: what the engine does, that requests are self-contained, the error model, a short form of the Default project model, and links to this document and Command line on GitHub, so an agent can find the full reference.
4. **Discovery.** The host calls `tools/list` and gets every tool's name, description, input schema and output schema. An AI agent reads these to decide what to call; an application can rely on this document instead.
5. **Calls.** The host calls `tools/call` with a tool's name and its arguments as a JSON object. The response carries the tool's result as JSON in `structuredContent`, or an error object when `isError` is true.
6. **Notifications.** While a tool runs, the engine can send one-way messages that need no reply: progress, and live tool output when the host asks for it. The host can also send a cancel notification to stop a call.
7. **Shutdown.** The host closes the engine's standard input or ends the process.

**Methods and tools are two levels.** A *method* is a message type that MCP itself defines, the same for every MCP server: `initialize`, `tools/list`, `tools/call` and a few others. A *tool* is a function this server offers, such as `build` or `lint`. Tools are never called directly: every tool call is a `tools/call` request whose `name` field names the tool. So the Logix Engine has a handful of methods and many tools.

**Versions.** Two versions matter, and the tool API has no separate version of its own:

- **MCP protocol version**, agreed in the `initialize` handshake. It covers the message format, not the Logix Engine tools, and MCP client libraries handle it.
- **Logix Engine version**, `engine_version` from the `get_engine_info` tool, its release date as `yyyy.mm.dd`, such as `2026.12.01` (its release tag is the same date, `2026-12-01`). It also versions the tool API. After the first release, changes only add tools and fields.

To check, a hardcoded client calls `get_engine_info` right after the handshake and checks `engine_version` against the versions it supports, by its own rule, such as a range or a list of tested releases (versions compare correctly as plain strings). If the version isn't supported, the client calls no other tools and tells the user what it needs (see Start up under Recipes). An AI agent needs no check, since it reads the current tools from `tools/list`.

The host may send several calls without waiting; each response carries the ID of its request. The rest of this document describes each tool.

A call and its response look like this (wrapped for reading; on the wire each is one line):

```json
{"jsonrpc": "2.0", "id": 7, "method": "tools/call",
 "params": {"name": "clean_build",
            "arguments": {"folder": "/home/user/proj/builds"}}}

{"jsonrpc": "2.0", "id": 7,
 "result": {"structuredContent": {"deleted": ["/home/user/proj/builds/icesugar-1-5", "/home/user/proj/builds"]},
            "isError": false}}
```

Most languages have an MCP client library that handles the handshake, IDs and message framing, so an application usually only calls tools and reads results.

## Schemas and validation

- **Export.** The Logix Engine returns each tool's name, description, `inputSchema` and `outputSchema`, in JSON Schema, when a host calls the MCP `tools/list` method. MCP clients, including AI agents, read them from there, so no separate schema file is needed.
- **Input validation.** The engine checks every request against the tool's input schema before doing anything. A request that doesn't match is a request error, returned as a tool error rather than an MCP protocol error. Its message and diagnostics name the field and the problem, so a host or AI agent can fix the call and retry.
- **Rules beyond the schema.** Some rules can't be written in JSON Schema, such as the path keyword rules, symbolic link checks, whether a file exists, or whether a board ID is in the catalog. The engine checks these right after the schema, and they are also request errors.
- **Tight schemas.** Where JSON Schema can express a rule, the schema states it, such as a pattern for catalog IDs, USB IDs and define names, and the allowed values of each enum field. This catches more mistakes early and shows the rules to AI agents.
- **Output.** Each tool's structured content always matches its output schema.
- **Published copy.** To read the schemas without running the engine, see [schemas/tools.json](https://github.com/openlogix/logix-engine/blob/main/schemas/tools.json) in the logix-engine GitHub repository. It holds the same JSON that `tools/list` returns. It is useful for generating client code or browsing the API. A running engine's `tools/list` is the authoritative answer for that version.

## Common types

### Logical types

The Type columns in this document use these logical types. Each maps to a JSON type with extra rules. Plain integer, number and boolean values use those JSON types directly.

**Table 1.** Logical types

| Logical type | JSON type | Rules |
| --- | --- | --- |
| absolute path | string | A complete path in the host computer's native form. On Windows, either `\` or `/` separators are accepted. It contains no `.` or `..` segments, and no `~`, environment variables or other shorthand: the host expands and normalizes it first. The Logix Engine still resolves symbolic links before using it. Case sensitivity follows the platform |
| relative path | string | Relative to the folder named in the field's description. Always uses `/` as the separator on every platform, never starts with `/`, and contains no `.` or `..` segments. An empty string means the folder itself, but only in fields whose description says so. Elsewhere it is a request error |
| build folder path | string | An absolute path that satisfies the path keyword rules |
| catalog ID | string | Lowercase letters, digits and hyphens |
| board ID | string | A board's ID: a catalog ID, or the ID of a board in the request's custom boards folder (its file name without `.json`) |
| display paths | object | How to name paths in what people read (see Display paths) |
| custom boards folder | absolute path | A folder with the host's own boards, one file each, named by board ID (lowercase letters, digits and hyphens) plus `.json`, such as a project's `boards/` folder: board definitions in the catalog's format, with the FPGA as `fpga_id`, a catalog FPGA. See Board specification |
| board definition | object | A board in Logix Catalog's format: its `name` for people, its FPGA as `fpga_id`, its `variants` (a list of variants, each with its selector, the first being the default), its optional `requires` (what every design for the board must do; see requirement), its optional `resources` (see resource), its optional `ai_hints` (see AI hints, under Board specification) and, in `get_catalog` results, its `dimensions`. Every board has the two standard dimensions, `connection` then `memory`, with values the engine defines: connection `usb` (the board's own USB port, through the programmer chip on the board), `bootloader` (its USB port in bootloader mode), the external JTAG adapters `ft2232h`, `ft232h`, `cmsis-dap` and `usb-blaster`, or `unspecified` (not known; shown as "USB"); memory `flash` (kept at power-off) or `ram` (lost at power-off). `dimensions` lists each with the values the board's variants use, in the engine's order, for showing the choices: a dimension has an `id`, a `label` for people (such as "Memory") and its `values`, each with an `id` (such as `ram`), a `label` (such as "RAM") and `help` text. It is derived by the engine, so inputs may leave it out (it is ignored). A dimension with one value is a fact about the board rather than a choice. Versions of a board with any signal on a different pin are different boards, and a board sold in several FPGA sizes is one board per size |
| selector | object | A value of each standard dimension, `connection` and `memory`, such as `{"connection": "usb", "memory": "flash"}`. Each variant has one, which says where it sits among the board's choices, and requests pick a variant with one (`board_variant`). Both fields are required, with the values the engine defines (see board definition) |
| variant | object | One way to program a board, in a board definition's `variants`: its `selector`, and how to program the board that way. It names the programmer to run, such as `openfpgaloader`, and gives the full argument template for it, with variables such as `${VID}`, `${PID}`, `${BUS}`, `${DEV}`, `${SERIAL_NUM}`, `${USB_PATH}` (the device's USB port path, such as `1-2.3`, as dfu-util's `-p` takes it) and `${BITSTREAM}`, and the USB filter that finds the device: `vid`, `pid`, an optional `product_regex`, a regular expression the device's product name must match, for boards that share USB IDs, an optional `serial_regex`, a regular expression the device's serial number must match, for boards that share USB IDs and product names but whose serial numbers tell them apart, and an optional `release`, the device's release number (USB `bcdDevice`, four lowercase hex digits, such as `0500`), for boards that share USB IDs but whose USB chips differ, such as a board debugger that acts like an FTDI FT2232C/D (`0500`) rather than an FT2232H (`0700`). The engine maps the programmer name to its executable and the package that provides it, so there are no separate programmer definitions. The supported programmers are `openfpgaloader`, `iceprog`, `dfu-util` and `icesprog`, all from the toolchain package. Catalog variants also have `tested` and optional `user_notes` (see Catalog contents under the get_catalog tool); custom boards take the same fields, `maintainer_notes` included. No two variants of a board have the same selector. Variants differ only in how the board is programmed: they all have the board's FPGA and the same pin file (every LED, button and clock on the same pin), so the same project works with each |
| resource | object | What a board wires to FPGA pins, in a board definition's optional `resources`, a map from name to resource: for creating designs and pin files for the board, such as a blinky; builds never use them. The name is the design's port name, a Verilog identifier: one pin gives a plain port (`led`), several a vector (`leds[7:0]`), bit 0 first. The order of resources in the map means nothing; results list them as the board file does, for easier reading. `type` says which fields it has: `clock` (`pin`, `mhz`), `leds`, `buttons` and `switches` (`pins`, `active`), `rgb_led` (`red`, `green`, `blue`, `active`), `led_matrix` (`leds`, a pin per LED, and `colors`, a pin per color, `red`, `green`, `blue`: an LED lights when its pin and a color's pin are both active; `active`), `uart` (`tx`, the FPGA's output, `rx`, optional `rts`, `cts`), `pmod` (`pins`, its 8 signal pins in header order, 1 to 4 then 7 to 10), `header` (`pins`, every position, `null` for power and ground), and `unclassified` (`pins`: every FPGA pin the board wires to something no other resource describes). `active` is `high` or `low`; left out, it's unknown. Every resource may have `io_standard` (one common name for every family, such as `LVCMOS33`; not used on iCE40), `source` (where its facts come from: `kind`, one of `hardware`, `schematic`, `maker`, `litex`, `amaranth` or `user`, and `ref`) `requires` (what must be done for it to work, such as a jumper a clock needs; see requirement), `user_notes` (warnings for people) and `ai_hints` (see AI hints, under Board specification). Resources may share pins (an LED on a PMOD pin); within one, pins are unique. A board lists what is known: a type it doesn't list is unknown, unless it has an `unclassified` resource, which lists all its wired pins. Example: `"clk": {"type": "clock", "pin": "35", "mhz": 12}`. The full format is Logix Catalog's [docs/model.md](https://github.com/openlogix/logix-catalog/blob/main/docs/model.md) |
| requirement | object | What must be done for a board, or a resource, to work, in its optional `requires` list, by `type`: `drive`, pins a design must drive to a level, as a port of its own (`name`, a port name unique on the board among resources and drives; `pins`, bit 0 first; `level`, `high` or `low`; optional `io_standard`, `why` for people and `source`), or `setup`, something a person must do to the hardware (`text`, what to do, and optional `source`). A board's requirements apply to every design for it, a resource's to designs that use it. Designs generated for a board include its drives, leave out resources that need a setup, so they work on the board as shipped, and include the drives of the resources they use. Example: `{"type": "drive", "name": "wifi_gpio0", "pins": ["L2"], "level": "high", "why": "GPIO0 of the board's ESP32: held low, it resets the board."}` |
| board variant | object | In requests: some or all of a selector's values, such as `{"connection": "ft232h", "memory": "ram"}` or just `{"memory": "ram"}`. The board's first variant with these values is used, so a host can keep only the values the user chose, and `{}` means the first variant, the default. Omitted means the default too. Values no variant has are a request error that lists the board's selectors. Results that depend on the variant, such as `build`'s `board_variant`, give the full selector used |
| FPGA definition | object | An FPGA entry in the catalog format: its `family`, the name on the chip (`ice40`, `ecp5`, `gw1n`, `gw2a`, `gw5a`, `artix7`, `spartan7` or `zynq7000`; the catalog file's name, `fpgas/<family>.json`), `part_number`, `size`, and the toolchain parameters: `nextpnr_device` for every family; `nextpnr_package` for iCE40 and ECP5; `nextpnr_speed`, the speed grade, for ECP5 only; for Gowin only, `packer_device` (the device `gowin_pack` uses) and, when needed, `nextpnr_family` (the die nextpnr uses). What the family implies, such as yosys's Gowin family or the Project X-Ray database folder, isn't repeated. Each family is built by one toolchain (`ice40`, `ecp5`, `gowin` or `openxc7`; `get_catalog` `families` gives it). ECP5, Gowin and Xilinx FPGAs may have a `jtag_idcode`, such as `0x0362d093`, the ID code openFPGALoader's `--detect` reports, which `program_board` checks before writing (see program_board). It identifies the chip, not the size a board uses it as. A field that doesn't belong to the family's toolchain is a request error |
| catalog query | object | Selects entries from one catalog section. It has `fetch`, a boolean that defaults to false, and `ids`, an optional list of catalog IDs. When `fetch` is true, the section is returned with only the listed IDs, or with all its entries when `ids` is omitted. An ID that isn't in the section is a request error |
| date tag | string | `yyyy-mm-dd` |
| USB ID | string | A 16-bit USB vendor or product ID, written as exactly four lowercase hexadecimal digits, such as `1d50`. Inputs are matched without regard to case |
| text | string | A single line of Unicode text, with no line breaks or other control characters |
| multiline text | string | Unicode text that may span several lines. Lines are separated by `\n` (LF only) on every platform, with no trailing newline. The only control characters allowed are `\n` and tab. The engine converts tool output to this form, including CRLF line endings, and strips terminal color codes |
| boolean | boolean | true or false |
| enum | string | One of the values listed in the field's description |
| list of X | array | Items of type X |
| map of X | object | Keys are catalog IDs or names, and values are of type X |
| error, diagnostic, action, device, resource, testbench result, command | object | See error fields in Table 3, diagnostic fields in Table 4, action codes in Table 5, device fields in Table 13 (list_devices), resource fields in Table 7 (build), testbench result fields in Table 27 (test), and command fields in Table 30 (Commands) |

### Folders

Operations that work on a design take these fields. Paths are absolute unless noted.

**Table 2.** Folder fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `source_folder` | absolute path | Yes | The source folder |
| `source_files` | list of relative paths | Yes | HDL files to use, relative to the source folder |
| `constraint_file` | relative path | Depends on the operation | Constraint file, relative to the source folder |
| `build_folder` | build folder path | Yes | The build folder, as an absolute path. It may or may not be inside the source folder, but it can't be the source folder itself |

**Build folders are disposable.** A host or a person can delete a build folder whenever no operation is using it, and no data is lost: it holds only generated files, and the next operation that writes to it creates it again, with the same result. Until the next build, `program_board` returns a request error that says to build first. The only exceptions are files that a host or a person puts there, and a GTKWave layout that a `simulate` request without `layout_file` puts there (see Layout files). The full rule is in docs/requirements.md, Interaction with the file system.

### Display paths

Every tool that reports files (`lint`, `test`, `simulate`, `format`, `diagram`, `build`, `program_board`, `list_devices`, `install_driver`, `uninstall_driver` and `get_catalog`) takes an optional `display_paths`, which says how to name paths in what people read. A host that gives the engine copies of the user's files, such as unsaved editor text in a scratch folder, uses it so messages name the files the user knows.

```json
"display_paths": {
  "files": [{"path": "/home/you/.myide/scratch/7/custom-boards.json", "as": "/home/you/blinky/custom-boards.json"}],
  "folders": [{"path": "/home/you/.myide/scratch/7", "as": ""}]
}
```

**Table 2a.** Display paths fields

| Field | Type | Description |
| --- | --- | --- |
| `files` | list of objects | Files named as a whole, each with `path` (absolute path) and `as` (text): a path to this file reads as `as`. Optional |
| `folders` | list of objects | Folders, each with `path` (absolute path) and `as` (text): a path inside the folder keeps its part below the folder, after `as` and `/`, or alone when `as` is empty, so `""` gives names relative to the folder. The folder itself reads as `as`, or `.` when `as` is empty. Optional |

- **Which entry.** File entries come first, then folder entries; within each list, the first that matches wins. A folder matches by whole path parts: `/a/b` matches `/a/b/x.v`, never `/a/bc/x.v`.
- **Real locations.** Paths are compared by where they really are, not by how they're spelled: through symbolic links (such as `/tmp` and `/private/tmp` on macOS), and as the same file on disk, so a case-insensitive volume's spellings match. A file that doesn't exist is compared by its path's parts.
- **Where.** Diagnostics (`file`, and paths in `text`), error `message` and `details`, testbench `output`, the live tool output (`text` and `original_text`) and the source places in `diagram`'s model and page. In text, the engine finds paths it knows, as the request or a tool spelled them, and names them by the same rules.
- **Never.** Command lines (`commands`, and `command` lines in the live output), which must stay rerunnable, and fields that are paths, such as `bitstream`, `page_file`, `waveform_file` and `layout_file`.
- **No match.** A path that matches no entry, or any path when the request has no `display_paths`, is shown as the request gave it: source files relative to the source folder, custom board file as its absolute path. A path the request never gave, such as a toolchain file a tool mentions, is shown as the tool printed it.
- **A path in an entry that isn't absolute** is a request error.

### Board specification

The `board` field is a board ID: a catalog ID, such as `"icestick"`, or the ID of one of the host's own boards. Every tool that deals with boards (`build`, `lint`, `test`, `simulate`, `program_board`, `list_devices`, `install_driver`, `uninstall_driver`, `get_catalog` and `compare_boards`) takes the same optional `custom_boards_folder`, the absolute path of a folder with the host's own boards, which is independent of the board a request selects:

- **Format.** The folder holds one file per board, in [Logix Catalog](https://www.openlogix.org/catalog/)'s format, named by the board's ID: `my-tang-nano-9k.json` is the board `my-tang-nano-9k`. The engine reads every `*.json` file in the folder and ignores other files; an empty folder means no boards. Each file has `name`, the FPGA as `fpga_id` (a catalog ID: the catalog has every FPGA the toolchain supports, so custom boards add boards, never FPGAs), its `variants`, written out in full, with their USB filters, and optional `resources`. It's exactly a Logix Catalog board file, so a board copies between the catalog and a project as a file, name and all; one from `get_catalog` needs its variants' `usb` filters added, which `get_catalog` leaves out. `dimensions` are ignored.
- **Schema.** The format has a JSON Schema, which Logix Catalog owns, at `https://www.openlogix.org/schemas/logix-board-v1.json` (`boards_schema` in `get_engine_info`). Every catalog board file names it. A file may name it with a `"$schema"` key, so editors that read JSON Schemas, such as VS Code, JetBrains IDEs, Zed or Neovim, check the file and complete its fields as it's written. The key is optional; when it's there, its value must be the schema the engine reads, or the file has a problem at that line. The engine checks more than the schema can, such as that `fpga_id` is a catalog FPGA. Until the first release the schema changes without a new version; after it, it only gains optional fields, and a breaking change gets a new address (`-v2`).

  ```json
  {
    "$schema": "https://www.openlogix.org/schemas/logix-board-v1.json",
    "name": "My Tang Nano 9K",
    "fpga_id": "gw1nr-lv9qn88pc6-i5",
    "variants": [
      {
        "selector": {"connection": "usb", "memory": "flash"},
        "programmer": "openfpgaloader",
        "args": "--force-terminal-mode --verify -b tangnano9k -f --busdev-num ${BUS}:${DEV} ${BITSTREAM}",
        "usb": {"vid": "0403", "pid": "6010", "product_regex": "^JTAG Debugger$", "release": "0500"},
        "tested": true
      }
    ],
    "resources": {
      "clk": {"type": "clock", "pin": "52", "mhz": 27, "io_standard": "LVCMOS33"}
    }
  }
  ```
- **Replacing catalog boards.** A custom board with a catalog board's ID replaces that catalog board for the request: for building, matching devices and listing.
- **Problems at file, line and column.** The engine reads the folder the request names, and nothing else. Every tool checks every file in it, so a problem in any is a request error in every tool, whichever board the request selects. Its `diagnostics` give each problem at file, line and column, as for source files: invalid JSON (with the parser's message), a field of the wrong type, a misspelled or missing field, a file name that isn't a board ID, an unknown FPGA ID, a `$schema` other than the board schema, a requirement's or resource's problems (a name that isn't a port name or is taken by another resource or drive, a pin twice in one resource, a PMOD without its 8 pins, a clock without a frequency, an I/O standard the FPGA family doesn't use), and in variants an invalid connection or memory value in the selector, a selector another variant has, a programmer, argument template, USB ID, release or regular expression. A definition problem points at the value, or for a missing field at the object it belongs in. Problems come all at once, by file name, then in file order: each board and each variant is checked on its own, so a problem in one never hides problems in another. Invalid JSON is the exception: it is one problem, the parser's, since nothing after it can be read. At most 10 problems are listed; when there are more, a last `info` diagnostic without a line says how many more. Diagnostics name each file by the folder as the request gave it, plus the file name, or as `display_paths` names it. Hosts can show them as markers in their editor.
- **Send the same folder to every tool.** The engine keeps nothing between requests. A build's board is kept in its build record. `program_board` looks up the build's board again in its request and refuses the upload only when the bitstream no longer fits it (see What counts as the same board below); changes to other boards never matter.

Tools that take a board also take an optional `board_variant`: some or all of the selector of one of the board's variants, such as `{"connection": "usb", "memory": "ram"}` or `{"memory": "ram"}`; the board's first variant with those values is used. The build record keeps the chosen board and variant for `program_board`.

**AI hints.** A board and each of its resources may have `ai_hints`: a list of short lines (each one line of at most 90 characters; an empty list is a problem, leave the field out) for AI agents, and people, writing designs for the board, with what the other fields can't express, such as an RGB LED on the iCE40 UltraPlus's RGB LED driver pins, which a design drives through `SB_RGBA_DRV`. They're advice to weigh, not instructions, since anyone can contribute to the catalog or write a custom board. They're always the last field of their board or resource, in board files and in results, and `get_catalog` returns them unchanged. The engine never reads them for builds or programming. Logix Catalog's `docs/model.md` (AI hints) says what goes in them.

**What counts as the same board.** Builds and uploads compare only what the bitstream depends on: the FPGA's settings for the build tools. Everything else about a board can change after a build without a rebuild.

| Board field | Compared | Why |
| --- | --- | --- |
| FPGA `family`, `nextpnr_device`, `nextpnr_package`, `nextpnr_speed`, `nextpnr_family`, `packer_device` | Yes | The build tools use them: a change makes another bitstream. `build` rebuilds, and `program_board` refuses an older build (build again) |
| Board ID and where the board comes from (the catalog or the custom boards folder) | No | A board identical to another, under any ID, builds the same bitstream. `program_board` uses the build's board ID only to look the board up again |
| Board `description` | No | A name for people |
| FPGA `part_number` and `size` | No | Names for people and messages |
| FPGA `jtag_idcode` | No | Used only when programming, to check the connected FPGA: `program_board` uses the current value |
| Variant: which one (`board_variant`), and its `programmer`, `args` and `usb` filter | No | They only say how the board is programmed: a build for another variant is up to date, and `program_board` programs with the current definition of the build's variant |
| Variant `tested` and `user_notes`, `dimensions`, `resources` and `ai_hints` | No | Information for people and for creating designs |

The other build inputs, such as the sources, the options, the toolchain and the engine version, still count as before (see build). `program_board` looks up the build's board ID and variant selector in its own request (its custom boards folder first, else the catalog) and programs with what it finds; a board or variant that is gone, or a different FPGA setting from the table, is a request error: build again.

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
| `data` | object | Optional extra data, such as candidate devices, or `commands`, the programs a failed call ran, as in Commands |

**Table 4.** Diagnostic fields

| Diagnostic field | Type | Description |
| --- | --- | --- |
| `file` | text | The file, when known: as the request named it (a source file relative to the source folder, a custom board file as the folder's absolute path plus its name), or as `display_paths` names it. Many tool messages have no location |
| `line` | integer | Line number, starting at 1, when known |
| `column` | integer | Optional column number, starting at 1 |
| `severity` | enum | `error`, `warning` or `info` |
| `text` | text | The problem, in words |
| `tool` | text | Optional name of the tool that reported it |
| `time_ns` | number | Optional. For a testbench failure, the simulation time it happened at, in nanoseconds, when known |

**Table 5.** Suggested action codes

| Action code | Meaning |
| --- | --- |
| `download_packages` | Required packages are missing or broken and weren't downloaded automatically, because auto-download is off or the download failed |
| `connect_board` | No matching board is connected |
| `fix_device_permissions` | A device was found but cannot be opened |
| `install_drivers` | A driver required by the board is missing (Windows): `install_driver` installs it, after asking the user |
| `disconnect_devices` | Other connected devices have the board's USB IDs and its programmer can't tell them apart; disconnect them and retry |

### Presentation

The engine reports facts as plain data and leaves presentation to the host. It sends no colors or terminal codes, and strips them from tool output. Tool lines reach the host as the tool printed them, with a few exceptions in the live output only: the engine leaves out empty and blank lines (such as a progress bar's eraser) and lines it knows to be harmless noise (such as warnings about optional speedups a tool doesn't have), removes noise around a message (such as nextpnr's `Info: ` prefix) without changing what it says, and may add a line of its own after a tool's line to explain it. Which lines are affected can change in any version; results and diagnostics are never affected. Otherwise it labels what things are, such as diagnostics with a severity and progress phases with a message, so each host can style, filter and summarize them in its own way.

### Live tool output

A host that wants to show the tools' output as it happens, such as in a console, adds `"logix/output": true` to the `_meta` of its tool call. The engine then sends each line the tools print as a `logix/output` notification while the tool runs, and all of them before the tool result. The stream is flat and in order: an operation that runs several tools, such as `build`, sends each tool's command line followed by that tool's lines, then the next tool's. Without the flag it sends none, so other MCP clients and AI agents never get them. MCP's own logging notifications aren't used, since MCP 2026-07-28 deprecates them.

**Table 22.** logix/output notification fields

| Field | Type | Description |
| --- | --- | --- |
| `request_id` | number or string | The JSON-RPC ID of the tool call the line belongs to |
| `tool` | text | The tool that printed it, such as `yosys`, `nextpnr-ice40` or `openFPGALoader`, or `logix-engine` for the engine's own messages |
| `kind` | enum | `command` for the command line of a tool about to run, sent before its output, `output` for a line the tool printed, or `message` for a line from the engine itself, such as how `program_board` will program the board. Hosts should show unknown kinds like `output` |
| `level` | enum | `error`, `warning` or `info`, from how the tool marks the line, so hosts can color it |
| `text` | text | The line as the tool printed it, without terminal codes and without any noise the engine removed (see Presentation), with paths named as `display_paths` says. A progress bar redrawn with carriage returns gives one line per redraw |
| `original_text` | text | In `output` lines whose noise the engine removed: the line as the tool printed it. Omitted when `text` is the line as printed |
| `redraw` | boolean | In `output` lines: true when the tool prints over this line (a carriage return ended it, as progress bars do), so a console may replace it with the tool's next line, as a terminal does. False otherwise |

Lines that the engine reads into diagnostics still come in the tool result too. The live lines are for showing, not for parsing.

**Verbose.** Tools that run other tools take an optional boolean `verbose`. It asks those tools to print more detail, which shows in the live output and the logs, and never changes the result. Each tool maps it to what its tools offer: `build` runs yosys and nextpnr without `-q`, `lint` runs Verilator without `--quiet`, `test` and `simulate` run iverilog with `-v`, `format` adds Verible's `--verbose`, and `program_board` adds openFPGALoader's `--verbose`, or iceprog's or dfu-util's `-v` (icesprog has no such option). A build that is up to date runs nothing, so it prints nothing extra.

### Commands

Results of the tools that run programs (`lint`, `test`, `simulate`, `format`, `build` and `program_board`) list the programs they ran in `commands`, in order, and so does `data.commands` in an error from them. It lets anyone run a step again without the engine, such as to report a bug to the tool's developers. A build that is up to date runs nothing, so its list is empty.

**Table 30.** Command fields

| Field | Type | Description |
| --- | --- | --- |
| `tool` | text | The program's name, such as `yosys` |
| `package` | text | The package it came from, its version and the core's tool, such as `logix-core 2026.11.10 (oss-cad-suite)`. Omitted for a program from elsewhere, such as a custom programmer command |
| `cwd` | absolute path | The folder it ran in |
| `args` | list of text | The program as an absolute path, then its arguments, exactly as passed |
| `shell` | text | One POSIX shell command line that runs it again the same way: in the same folder, with the same minimal environment, `PATH` (the package's `bin` folder, then `/usr/bin:/bin`), `HOME` and `TMPDIR` |

Example:

```json
{
  "tool": "verilator_bin",
  "package": "logix-core 2026.11.10 (oss-cad-suite)",
  "cwd": "/home/user/proj",
  "args": ["/home/user/.logix/ide/engine/packages/logix-core/2026-11-10/oss-cad-suite/bin/verilator_bin", "--lint-only", "--quiet", "main.v"],
  "shell": "cd /home/user/proj && env -i PATH=/home/user/.logix/ide/engine/packages/logix-core/2026-11-10/oss-cad-suite/bin:/usr/bin:/bin HOME=\"$HOME\" TMPDIR=\"$TMPDIR\" /home/user/.logix/ide/engine/packages/logix-core/2026-11-10/oss-cad-suite/bin/verilator_bin --lint-only --quiet main.v"
}
```

Files the engine generated for a step, such as lint's Verilator config in the engine folder's `tmp/` or a testbench's `.limit.v` in the build folder, are named by absolute path and stay where they are, so the line works as long as they do.

### Progress

A host that wants progress adds an MCP progress token to its request. The engine then sends progress notifications with a progress value, an optional total and an optional message, such as "Placing and routing".

Every operation that can take more than a second or two reports progress by phase:

- **Phase messages.** Each phase sends a message when it starts, such as "Downloading logix-core", "Synthesizing", "Placing and routing", "Packing bitstream", "Finding device", "Checking FPGA", "Writing flash" or "Verifying".
- **Percentages.** Within a phase, the engine also reports a percentage wherever the underlying tool or download prints one, such as package downloads and openFPGALoader writing and verifying flash. Phases without one send only their start message, and the host shows a spinner.
- **One increasing scale.** MCP requires the progress value to only increase within one operation. So when the operation starts, the engine decides its phases, including any package downloads it needs. It sets total to 100 times the number of phases and reports progress as 100 times the phase index plus the phase's percentage. A host can show an overall bar from progress and total, and the current phase's percentage as progress modulo 100.
- **Unparsed output.** When the engine can't read a tool's output, such as with a custom `programmer_command`, that phase sends only its start message.

### Packages

**Which releases.** The Logix Engine reads which release of each package to use from its remote config, the version folder `remote-config/logix-engine/<NNN>/` in its repo (`NNN` is the config version this engine reads, `001` today), from GitHub: `channels.jsonc` says which engine releases are on which channel, and the engine reads its channel's config file there, such as `prod.jsonc` or `dev.jsonc` (`prod.jsonc` when its channel has none of its own). Its channel is `prod` when `channels.jsonc` lists this release under it, and `dev` otherwise; a locally built engine is always on `dev`. It reads the folder before an operation that needs packages when it last read it more than 24 hours ago, and when a host calls `check_updates`, and keeps a copy in the engine folder, which it uses offline. When the config names a newer release, the next operation downloads it and the old one is deleted. So a newer catalog or set of tools reaches an engine without a new engine release. A config that can't be read leaves the one in use; with none to use yet (a first start offline), a tool that needs packages returns a setup error. Two diagnostic settings change this on one computer only: `LOGIX_ENGINE_CHANNEL` picks the channel, and `LOGIX_ENGINE_DEV_REMOTE_CONFIG` names another version folder to read instead, a URL or a local folder, such as a pull request's version (see [Command line](command-line.md)).

Before running, each tool checks that the packages it needs are downloaded and intact. It downloads missing packages and repairs broken ones itself. A package is broken when its receipt is missing or doesn't match the release the remote config names (its URL, platform and SHA-256), or when one of its tools fails to start. Download progress is sent as the operation's progress notifications, and cancelling the operation cancels the download. Two Logix Engine instances never download the same package at the same time.

The `--no-auto-download` command-line argument, described in [Command line](command-line.md), turns this off for offline or managed setups. A missing or broken package is then a setup error with `download_packages`, and so is a download that fails, such as when there's no network. To download, the host calls the download_packages tool, which works whether automatic download is on or off.

Download progress is one phase per package, such as "Downloading logix-core 2026.11.10". Its percentage covers the download (the first 80%) and then unpacking.

**Engine folder layout.** Hosts don't need to know it, but it helps when troubleshooting:

| Path in the engine folder | Contents |
| --- | --- |
| `remote-config/config.json` | The remote config in use: where it was read, when, and the release and SHA-256 of each package |
| `packages/<name>/<release tag>/` | A downloaded package, by its release tag (`yyyy-mm-dd`), such as `packages/logix-core/2026-11-10/`, with one folder per tool in the core (`oss-cad-suite/`, `openxc7/`, `verible/`). Its receipt, `logix-download.json`, is written last; once a newer release is in place, the old one is deleted |
| `downloads/` | Partial downloads, deleted when a download ends |
| `locks/` | One lock file per package, held while downloading it |
| `logs/` | Log files, described in [Command line](command-line.md) |
| `tmp/` | Small files the engine generates for the tools, such as the Verilator config that turns off warnings in the FPGA models. Safe to delete |

An interrupted download never leaves a package that looks downloaded, so the next call downloads it again.

## lint tool

Checks the design and, optionally, its testbenches with Verilator in lint-only mode. It is much faster than a build and finds likely mistakes that synthesis accepts, such as width mismatches. It writes nothing and needs no build folder.

The Logix Engine adds the FPGA's primitive models, so designs that use primitives such as `SB_IO`, `EHXPLLL`, `rPLL` or `PLLE2_BASE` lint cleanly, and it turns off warnings inside those models, since they aren't the user's to fix. Verilator treats warnings as failures, so `passed` is false when there are warnings too. `SYNTHESIZE` is defined (unless `synthesize` is false) and `LOGIX_SIM` is `0`. When `top_module` is omitted, every module that no other module uses is checked as a top module. The exact Verilator command is returned in `commands`.

**Problems are the result.** Lint's job is to find problems, so problems in the design are returned in `diagnostics` with `passed` set to false, not as a design error. The other errors in the list below still apply.

**Table 23.** lint tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board ID | Yes | The target board. It selects the FPGA's primitive models |
| `board_variant` | board variant | No | The board's variant: the first with these values. Omitted means its first |
| `custom_boards_folder` | custom boards folder | No | The host's own boards (see Board specification). Omitted means none |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them (see Display paths) |
| `top_module` | text | No | Name of the top-level module. When omitted, Verilator checks every module that no other module uses |
| `source_folder`, `source_files` | See folder fields in Table 2 | Yes | The design files |
| `testbench_files` | list of relative paths | No | Testbenches to check too, relative to the source folder. Defaults to none. A file may be listed only once in `source_files` and `testbench_files` together |
| `defines` | map of text | No | Macro definitions, as in the build tool. `SYNTHESIZE` is defined unless `synthesize` is false, and `LOGIX_SIM` is defined as `0` |
| `synthesize` | boolean | No | Define `SYNTHESIZE`, so `` `ifdef SYNTHESIZE `` picks the code for synthesis, as in a build. False leaves it undefined, to check the simulation code instead. Defaults to true |
| `verbose` | boolean | No | Run Verilator without `--quiet`. Defaults to false |

**Table 24.** lint tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `passed` | boolean | True when Verilator found no errors and no warnings |
| `diagnostics` | list of diagnostics | The problems found, at file, line and column where known. A Verilator warning code, such as `WIDTHTRUNC`, ends the text in parentheses, since a `/* verilator lint_off WIDTHTRUNC */` comment names it. Problems in the primitive models keep their location in the text |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the Logix Engine. See Commands |

**Errors:** setup errors with `download_packages`. Request errors, such as an unknown board, a missing file or an included file outside the source folder. Tool failures, when Verilator fails without reporting a problem. Engine errors and cancellation.

**Includes.** The source file rules in Table 8 (build tool) apply. Verilator and Icarus Verilog don't look for an included file in the including file's folder as yosys does, so the engine passes the folder of every listed and included file as an include folder. An include is then found in the source folder first, then in those folders. Hosts don't deal with the toolchain's own files, such as the primitive models and black boxes for each FPGA family: the Logix Engine adds the right ones for the board's FPGA in lint, test, simulate and build, and doesn't expose toolchain files to hosts, except where a tool says so, such as `bin_folder` in get_engine_info for running Verible's language server. Source files and includes are only the user's own files.

**Example**

Input:

```json
{
  "board": "icesugar-1-5",
  "top_module": "main",
  "source_folder": "/home/user/proj",
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

Runs testbenches with Icarus Verilog. For each testbench in order, iverilog compiles the design files, the FPGA's primitive models and the testbench into the build folder, then vvp runs it and writes its waveform there. Every testbench runs, even after one fails. By default the Logix Engine stops each testbench at its first failure, so a failing run ends fast; see Test and simulate modes.

Testbenches run in the source folder, so they read data files, such as with `$readmemh`, relative to it. `LOGIX_SIM` is `0` and `SYNTHESIZE` isn't defined, so designs can tell simulation from synthesis. The Logix Engine sees a failure as soon as the testbench prints it, so it can stop the run right away. The exact iverilog and vvp commands are returned in `commands`.

**Testbench rules:** see Writing testbenches. A testbench fails by calling `$error` or `$fatal`, or by printing an assertion failure line (see `diagnostics` in Table 27). It must not call `$dumpfile`, which is a design error at that line.

**Failures are the result.** A failing testbench is returned in `testbenches` with `passed` set to false, not as an error. Compile errors are a design error, since nothing could run.

**Table 25.** test tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board ID | Yes | The target board. It selects the FPGA's primitive models |
| `board_variant` | board variant | No | The board's variant: the first with these values. Omitted means its first |
| `custom_boards_folder` | custom boards folder | No | The host's own boards (see Board specification). Omitted means none |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them (see Display paths) |
| `source_folder`, `source_files`, `build_folder` | See folder fields in Table 2 | Yes | The design files, without testbenches |
| `testbench_files` | list of relative paths | Yes | The testbenches to run, in order, relative to the source folder. At least one. A file may be listed only once in `source_files` and `testbench_files` together |
| `defines` | map of text | No | Macro definitions, as in the build tool. `LOGIX_SIM` is always defined as `0` |
| `stop_at_first_failure` | boolean | No | Stop each testbench at its first failure, so a failing run ends fast; its output and waveform then end there. False runs each testbench to the end, to see every failure. Defaults to true |
| `max_sim_time_ns` | integer | No | Stop each testbench when simulated time reaches this many nanoseconds. Defaults to no limit. See Runs that don't end |
| `timeout_seconds` | integer | No | Stop each testbench after this many seconds of real time. Defaults to 600; 0 means no limit |
| `verbose` | boolean | No | Run iverilog with `-v`. Defaults to false |

**Table 26.** test tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `passed` | boolean | True when every testbench passed |
| `testbenches` | list of testbench results | Each testbench's result, in the order given. See Table 27 |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the engine. See Commands |

**Table 27.** Testbench result fields

| Field | Type | Description |
| --- | --- | --- |
| `file` | relative path | The testbench, relative to the source folder |
| `passed` | boolean | True when it ended itself with `$finish` (`ending` is `finished`) without `$fatal`, `$error` or an assertion failure line |
| `diagnostics` | list of diagnostics | Compile warnings, and the testbench's `$fatal`, `$error` and `$warning` messages at file and line. Assertion failure lines, as common testbench assertion macros print them, are errors too, at file and line when they give one, such as `ERROR at main_tb.v:52: ...` and `*** ASSERTION FAILED at main_tb.v:52: ...`; `*** ASSERTION FAILED in main_tb ...` has no location |
| `output` | multiline text | What the testbench printed, such as with `$display`. When it printed more than 5,000 lines, the first and last 2,500, with a line saying how many were left out between them |
| `waveform_file` | relative path | The waveform (`.vcd`), relative to the build folder, when the testbench wrote one with `$dumpvars` |
| `ending` | enum | How the run ended: `finished` (the testbench ended itself, with `$finish` or `$fatal`), `first_failure`, `sim_time_limit` or `timeout` (the engine stopped it; its output and waveform end there), or `no_finish` (it ran out of things to simulate without calling `$finish`, which usually means it stopped early by mistake) |
| `failure_count` | integer | How many failures the testbench reported. `diagnostics` lists the first 100, each with its time; a check that fails again at another time is another failure |

**Errors:** design errors for compile errors and `$dumpfile`. Setup errors with `download_packages`. Request errors, such as an unknown board, a missing file or a build folder that breaks the keyword rules. Tool failures, engine errors and cancellation.

**Build folder files.** For a testbench `rtl/uart_tb.v`, the build folder gets `rtl/uart_tb.out` (the compiled simulation) and `rtl/uart_tb.vcd` (the waveform): the testbench's path without its extension. Each run replaces them. Like a build, the first run writes the `.logix-build` marker, so `clean_build` can delete the folder.

**Example**

Input:

```json
{
  "board": "icesugar-1-5",
  "source_folder": "/home/user/proj",
  "source_files": ["main.v"],
  "testbench_files": ["main_tb.v"],
  "build_folder": "/home/user/proj/_build"
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

Simulates one testbench and opens its waveform in GTKWave. It compiles and runs the testbench as the test tool does, with the same commands, testbench rules and build folder files, except that `LOGIX_SIM` is defined as `1` and the testbench always runs to the end, so the whole waveform is there to look at. Then it makes sure there is a layout file and starts GTKWave fully detached with the waveform and the layout, and returns without waiting for it. For a layout the user saved, it adds `--rcvar=do_initial_zoom_fit 1` first; a default layout sets its own view, which that would replace. GTKWave keeps running after the operation and after the Logix Engine exits.

**Layout files.** A GTKWave layout (`.gtkw`) holds the signals shown and how. It is a project file the user changes by saving from GTKWave, which reads and saves the same path, so the host names it in `layout_file`, usually next to the testbench, such as `main_tb.gtkw`.

- **Missing or default:** when the file doesn't exist, or is a default one the engine generated (it has the engine's marker line; layouts any other tool made are the user's), the engine writes a default layout there with the testbench's top-level signals, so GTKWave doesn't open empty. The default layout also sets the view: the whole run when the testbench passed, or, when it failed, a window around the first failure that has a time, with GTKWave's marker on the failure about two thirds of the way across, so what led up to it shows too. The window is about 80 times the waveform's typical step between changes, roughly 40 cycles of a clock, and never wider than the run. A default is replaced on each simulation, so it follows design changes.
- **Saved by the user:** a layout without the marker is the user's and is never changed. GTKWave opens it, and saving from GTKWave updates it.
- **Unsaved defaults are cleaned up:** when GTKWave exits, the engine deletes the layout if it still has the marker, so default layouts the user never saved don't pile up in the project or end up in version control. This is best-effort: if the engine exits before GTKWave, the default stays until a later simulation replaces or deletes it.
- **No layout file:** when `layout_file` is omitted, the layout goes into the build folder as `<testbench>.gtkw`, where a saved layout is lost when the folder is cleaned.

This is the one case where the engine creates or deletes a file in the source folder. See [Safe file changes](requirements.md).

**A failing testbench** is still simulated and shown, since its waveform is how the user finds the problem. `passed` is false. A testbench that writes no waveform, because it doesn't call `$dumpvars`, is a design error.

**Viewer.** `open_viewer` set to false only writes the waveform and the layout, such as for an AI agent or when no one is at the screen. Opening a viewer needs a local frontend: a network adapter turns it off or refuses it for remote frontends, which view waveforms on their own.

**Table 28.** simulate tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board ID | Yes | The target board. It selects the FPGA's primitive models |
| `board_variant` | board variant | No | The board's variant: the first with these values. Omitted means its first |
| `custom_boards_folder` | custom boards folder | No | The host's own boards (see Board specification). Omitted means none |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them (see Display paths) |
| `source_folder`, `source_files`, `build_folder` | See folder fields in Table 2 | Yes | The design files, without testbenches |
| `testbench_file` | relative path | Yes | The testbench to simulate, relative to the source folder |
| `layout_file` | relative path | No | The GTKWave layout file, relative to the source folder, ending in `.gtkw`. Its folder must exist; the file may not. When omitted, the layout goes into the build folder |
| `open_viewer` | boolean | No | Start GTKWave on the waveform. Defaults to true |
| `max_sim_time_ns` | integer | No | Stop the testbench when simulated time reaches this many nanoseconds. Defaults to no limit |
| `timeout_seconds` | integer | No | Stop the testbench after this many seconds of real time. Defaults to no limit |
| `defines` | map of text | No | Macro definitions, as in the build tool. `LOGIX_SIM` is always defined as `1` |
| `verbose` | boolean | No | Run iverilog with `-v`. Defaults to false |

**Table 29.** simulate tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `passed` | boolean | As in the testbench results of the test tool: true when the testbench reached `$finish` without a failure |
| `diagnostics` | list of diagnostics | As in the testbench results of the test tool, in Table 27 |
| `output` | multiline text | What the testbench printed |
| `failure_count` | integer | How many failures the testbench reported, as in the test tool |
| `ending` | enum | How the run ended, as in the test tool. Never `first_failure` |
| `waveform_file` | relative path | The waveform (`.vcd`), relative to the build folder |
| `layout_file` | absolute path | The layout file GTKWave opens |
| `layout_created` | boolean | True if the engine wrote a default layout, false if it kept the user's |
| `viewer_opened` | boolean | True if GTKWave was started |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the engine. See Commands |

**Errors:** design errors for compile errors, `$dumpfile` and a testbench without `$dumpvars`. Setup errors with `download_packages`, or when the layout file can't be written. Request errors, such as a missing file, a layout file that isn't `.gtkw` or whose folder doesn't exist, or a build folder that breaks the keyword rules. Tool failures, such as when GTKWave can't start, engine errors and cancellation.

**Example**

Input:

```json
{
  "board": "icesugar-1-5",
  "source_folder": "/home/user/proj",
  "source_files": ["main.v"],
  "testbench_file": "main_tb.v",
  "build_folder": "/home/user/proj/_build",
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
  "layout_file": "/home/user/proj/main_tb.gtkw",
  "layout_created": true,
  "viewer_opened": true
}
```

## Test and simulate modes

The same testbench serves two purposes: `test` is a batch check that should fail fast, and `simulate` produces a waveform the user can look at to find the problem. The testbench only states what is correct; the Logix Engine applies each mode's policy:

|  | `test` | `simulate` |
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

These are the Logix Engine's guidelines for testbenches. The engine defines the macro `LOGIX_SIM`: 0 in `test` and `lint`, 1 in `simulate`. Assertion failure lines count as failures.

1. **Report a failure with `$error`, then keep going.** Put the expected and actual values in the message. The engine reads `$error` with its file, line and simulation time, and decides whether to stop.
2. **Use `$fatal` only when going on makes no sense,** such as a missing data file. It stops the run in both modes.
3. **Don't check the mode.** No ``if (!`LOGIX_SIM) $fatal``: the engine stops `test` at the first failure and lets `simulate` run to the end.
4. **End with `$finish`.** A testbench that runs out of things to do without it fails, and one that never ends is stopped by a limit (Runs that don't end) and fails.
5. **Call `$dumpvars(0, <testbench module>)` at the start** to write a waveform, and **never call `$dumpfile`**: the engine decides where the waveform goes.
6. **Read data files relative to the source folder,** such as `$readmemh("data/rom.hex", rom)`. Testbenches run there.
7. **`SYNTHESIZE` isn't defined in `test` and `simulate`,** so `` `ifdef SYNTHESIZE `` / `` `else `` selects the simulation code.
8. **Name testbenches `<name>_tb.v` or `<name>_tb.sv`** and give them a `` `timescale ``. The engine doesn't need the names, since requests list the testbenches, but hosts that follow this naming find them that way.

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

## format tool

Formats Verilog and SystemVerilog files in place with Verible's formatter. Formatting changes only layout, such as indentation, spacing and line breaks, never meaning. The host lists the files; the Logix Engine doesn't search for them. To format text that isn't saved yet, such as an editor's buffer, a host writes it to a file in a scratch folder and formats it there, since requests never carry source text.

Each file is formatted on its own, in order, with the request's `verible_options`; the exact commands are returned in `commands`. A file is rewritten only when its layout changes. A file reached through a symbolic link is formatted where it really is, which must be inside the source folder, and the link stays. Code between `// verilog_format: off` and `// verilog_format: on` comments is left as is.

**Problems are the result.** A file Verible can't format, such as one with a syntax error, is left unchanged and listed as `failed`, with the problems in `diagnostics` and `passed` set to false, as in lint. The other files are still formatted. A statement too complex for Verible to lay out within its search limit fails the same way; its diagnostic suggests the option `--max_search_states=2000000` or `verilog_format: off` comments.

**Table 31.** format tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `source_folder` | absolute path | Yes | The folder with the files |
| `source_files` | list of relative paths | Yes | The files to format, relative to the source folder: `.v`, `.sv`, `.vh` or `.svh`. Each is rewritten in place, and may be listed only once |
| `verible_options` | list of text | No | Extra options for the formatter, such as `["--column_limit=80", "--indentation_spaces=4"]`. Each must be one option, `--name` or `--name=value`, so it can't name a file. `--inplace`, which the engine always passes, and the options that read options from elsewhere (`--flagfile`, `--fromenv`, `--tryfromenv`) are refused. Defaults to none, which gives Verible's default style |
| `verbose` | boolean | No | Add Verible's `--verbose`. Defaults to false |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them (see Display paths) |

**Table 32.** format tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `passed` | boolean | True when every file is formatted now, whether it was rewritten or already formatted |
| `files` | list of objects | Each file, in the request's order, with its `file` as given and its `status`: `formatted` (rewritten), `unchanged` (already formatted) or `failed` (left unchanged; see `diagnostics`) |
| `diagnostics` | list of diagnostics | Why files failed, such as syntax errors at file, line and column |
| `commands` | list of commands | The programs this call ran, one per file. See Commands |

**Errors:** setup errors with `download_packages`, for the `verible` package. Request errors, such as a missing file, a file that isn't Verilog or SystemVerilog, a file outside the source folder or an option that isn't allowed. Tool failures, when Verible fails without reporting a problem, such as for an option it doesn't know. Engine errors and cancellation.

**Example**

Input:

```json
{
  "source_folder": "/home/user/proj",
  "source_files": ["main.v", "main_tb.v", "uart.v"]
}
```

Output:

```json
{
  "passed": false,
  "files": [
    { "file": "main.v", "status": "formatted" },
    { "file": "main_tb.v", "status": "unchanged" },
    { "file": "uart.v", "status": "failed" }
  ],
  "diagnostics": [
    {
      "file": "uart.v", "line": 12, "column": 6, "severity": "error",
      "text": "syntax error at token \"end\"", "tool": "verible"
    }
  ]
}
```

## diagram tool

Draws the design as schematic diagrams, one per module: registers, adders, comparators, multiplexers, memories, the FPGA's own blocks and other modules as symbols, with the wires between them. Yosys reads the design as written, without synthesis, with `SYNTHESIZE` defined, and the engine draws every module itself: it lays them out with ELK (the Eclipse Layout Kernel, logix-core's `elkjs`, run in a JavaScript engine built into Logix Engine) and writes the SVGs. Nothing else is needed: no browser plugin, runtime or other program.

The diagram starts at `top_module`: the design's top module, or any module of the design files, drawn with its parameters' default values. The modules it uses are drawn too, each once per set of parameters, down to `levels` levels below it (3 by default); deeper ones are in the model with a `note`, and a diagram that starts at one of them shows it. The drawing follows a few rules, so it reads like a hand-drawn schematic:

- **Labels instead of long wires.** A clock or a reset that goes to two or more places is drawn as labels with its name at each place, not as wires across the drawing. A fixed input, such as an adder's `1`, is a short wire from a label with its value (decimal, or hex from 10 up), while an input left unconnected, whose value doesn't matter (yosys's `x`), is drawn with no wire at all: its port's `fixed` value in the model is "don't care", and its hover says so.
- **Plain names.** Blocks are named in plain words in the model and on hover (Register, Adder, PLL, I/O pin), FPGA blocks by their family's names. Above a block, only a name worth reading: a module's, a register's (the signal it holds) or a memory's, which its read and write ports share (hovering one lights up the others); names over 18 characters are cut with "…", and the layout makes room for them. Other boxes show a short sign inside, such as RD and WR for memory ports, & for an AND of all bits, × or <<.
- **What isn't logic isn't drawn.** A memory's initialization isn't a block: the memory says where its initial contents are loaded.
- **Large modules.** A module with more than 1,500 blocks isn't drawn: its diagram would be unreadable. Modules over 150 blocks get a faster, simpler layout. A layout that fails or takes more than 60 seconds leaves only its module undrawn, with a `note`; the engine runs each layout in a process of its own, so a failure never stops the engine. The progress phase is "Preparing the diagram", or "Preparing the diagram (large)" when the modules drawn have more than 300 blocks in all.

Each call writes the build folder's `diagram/` folder again, deleting the old one first, so a failed call never leaves an old diagram looking current:

- **`diagram.html`**, the page: every module, self-contained in one file, which any web browser opens, with no network. Double-click a module to open it (the bar at the top goes back), drag to pan, scroll to zoom, and hover a block, port, wire, pin or label for its details. Hosts can embed it (see Showing the diagram).
- **`<module key>.svg`**, each module's drawing: no script, with a default style scoped to it (`svg.logix-diagram`) whose colors are CSS variables, such as `--logix-diagram-register` and `--logix-diagram-wire`, so a host can theme it without changing the file. Its elements carry data attributes for hosts that add their own interaction: `data-block` (a cell, by its yosys name), `data-port`, `data-pin`, `data-label`, `data-bits` (a wire's net bits) and `data-sj` (a bus split or join).
- **`diagram.json`**, the model: `top`, the top module's key, and `modules`, by key, each with `name`, `top`, `source` (its file and line), `svg_file`, `note`, `blocks` (each cell, by its yosys name, with `kind` (`logic`, `module` or `fpga`), `category`, `title`, `type`, `width`, `name`, `instance`, `module` (the key of the module it instantiates), `source`, `memory_init`, `fixed` inputs and `ports`, each with `direction`, `width`, `signal` and `by_label`), `labels`, `pins`, `signals` (by net bit, each with its `name` and `width`) and `clocks`. Source files are named as diagnostics name them.
- **`netlist.json`**, yosys's netlist, which the diagram is drawn from.

**Table 33.** diagram tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board ID | Yes | The target board. Its FPGA family names the FPGA's own blocks and gives their ports' directions |
| `board_variant` | board variant | No | The board's variant. It doesn't change the diagram |
| `custom_boards_folder` | custom boards folder | No | The host's own boards (see Board specification). Omitted means none |
| `top_module` | text | Yes | The module the diagram starts at: the design's top module, such as `main`, or any module of the design files, such as `uart` |
| `levels` | number | No | How many levels of modules below `top_module` are drawn, such as 1 for it and the modules it uses. Defaults to 3 |
| `source_folder`, `source_files`, `build_folder` | See folder fields in Table 2 | Yes | The design files, without testbenches or included files. Paths with a double quote (`"`) are refused, since yosys reads them from a script |
| `defines` | map of text | No | Macro definitions, as in the build tool. `SYNTHESIZE` is always defined |
| `open` | boolean | No | Open the page in the user's default web browser when it's written. Defaults to false |
| `verbose` | boolean | No | Run yosys without `-q`. Defaults to false |
| `display_paths` | display paths | No | How to name paths in what people read, including the diagram's source places. Omitted means as the request gave them (see Display paths) |

**Table 34.** diagram tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `page_file` | relative path | The page, relative to the build folder: `diagram/diagram.html` |
| `model_file` | relative path | The model, relative to the build folder: `diagram/diagram.json` |
| `modules` | list of objects | Every module, the top module first, each with `key` (its key in the model and the page, usually its name; a module used with different parameters gets one per version, such as `genrom` and `genrom-2`), `name`, `top`, `svg_file` (relative to the build folder; omitted when it wasn't drawn), `blocks` (how many its diagram has) and `note` (why it wasn't drawn) |
| `opened` | boolean | Whether the page was opened in the web browser |
| `diagnostics` | list of diagnostics | Yosys warnings while reading the design, at file and line where known. Often empty |
| `commands` | list of commands | The program this call ran: yosys. See Commands |

**Errors:** design errors, with diagnostics, for errors in the design, such as a syntax error, or a `top_module` that isn't in the files. Setup errors with `download_packages`, for the toolchain package. Request errors, such as a missing file or an unknown board. Tool failures, when yosys fails without reporting a problem. Engine errors and cancellation.

**Showing the diagram.** An AI agent shows the user the diagram with `open: true`, or gives them `page_file`'s path to open; nothing needs to be generated or installed. A frontend shows the page in a web view, or embeds it in an iframe and talks to it with messages: `?theme=light` or `?theme=dark` in its address sets the theme (otherwise it follows the system), `#main/uart` opens a module, it posts `{"type": "logix-diagram:open-source", "file", "line"}` when the user double-clicks a block other than a module (to show its source; a double-click on a module opens it; `file` is named as in diagnostics, so check it's a project file before opening it) and `{"type": "logix-diagram:module", "path"}` when the module shown changes, and it takes `{"type": "logix-diagram:theme", "theme"}`. A host that loads the page directly can define `window.logixDiagramOpenSource(file, line)` instead. The page shows "Go to source" for the module and "Double-click to go to its source" on blocks only when it's hosted this way.

**Example**

Input:

```json
{
  "board": "icesugar-1-5",
  "top_module": "main",
  "source_folder": "/home/user/proj",
  "source_files": ["main.v", "uart.v"],
  "build_folder": "/home/user/proj/_build",
  "open": true
}
```

Output:

```json
{
  "page_file": "diagram/diagram.html",
  "model_file": "diagram/diagram.json",
  "modules": [
    { "key": "main", "name": "main", "top": true, "svg_file": "diagram/main.svg", "blocks": 14 },
    { "key": "uart", "name": "uart", "top": false, "svg_file": "diagram/uart.svg", "blocks": 31 }
  ],
  "opened": true,
  "diagnostics": []
}
```

## build tool

Synthesizes the design, places and routes it, and generates the bitstream in the build folder. A build with no changes since the last one returns quickly with `up_to_date` set.

Each FPGA family is built by its toolchain's open source flow: yosys for synthesis, nextpnr for place and route, then the toolchain's bitstream tools (`ice40`: nextpnr-ice40 and icepack; `ecp5`: nextpnr-ecp5 and ecppack; `gowin`, for GW1N, GW2A and GW5A: nextpnr-himbaechel and gowin_pack; `openxc7`, for Artix-7, Spartan-7 and Zynq-7000: nextpnr-xilinx and the openXC7 tools). The exact commands are returned in `commands`.

The constraint file must be in the family's format and have its extension (any case): `.pcf` for iCE40, `.lpf` for ECP5, `.cst` for Gowin, `.xdc` for Xilinx 7-series (`get_catalog` `families` gives each family's). Any other extension is a request error, since a misnamed file would otherwise build wrongly or fail with a puzzling parse error.

Xilinx builds use two of the core's tools, `oss-cad-suite` and `openxc7`. Two Xilinx builds of the same design differ only in the bitstream header, which holds the build date.

**Table 6.** build tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `board` | board ID | Yes | The target board |
| `board_variant` | board variant | No | The board's variant: the first with these values. Omitted means its first |
| `custom_boards_folder` | custom boards folder | No | The host's own boards (see Board specification). Omitted means none |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them (see Display paths) |
| `top_module` | text | Yes | Name of the top-level module |
| `source_folder`, `source_files`, `constraint_file`, `build_folder` | See folder fields in Table 2 | Yes | The constraint file is required |
| `defines` | map of text | No | Verilog and SystemVerilog macro definitions, from name to value, such as `{ "USE_PLL": "", "WIDTH": "8" }`. Names follow Verilog identifier rules, and a bad name is a request error. An empty value defines the macro with no text, for use with `` `ifdef ``. The engine always passes each value explicitly, so every tool sees the same text, and passes them in name order, so the same request always gives the same tool commands. `SYNTHESIZE` is always defined |
| `verbose` | boolean | No | Let yosys and nextpnr print more detail, shown in the live output and the logs. Defaults to false. See Verbose under Live tool output |

**Table 7.** build tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `bitstream` | relative path | Path of the bitstream, relative to the build folder |
| `board_variant` | selector | The variant the build is for, in full: the one `board_variant` picked. `program_board` programs it by default |
| `up_to_date` | boolean | True if nothing had to be rebuilt |
| `resources` | list of objects | Every resource the toolchain reports, sorted by name ignoring case, each with `name` (the toolchain's own, as nextpnr reports it, such as `ICESTORM_LC` or `SLICE_LUTX`: what people search for and quote), `description` (what it is in a few plain words, such as "Logic cell: a 4-input LUT, a flip-flop and carry logic"; left out for a name the engine doesn't describe yet), `used` and `available` (integers, as nextpnr reports them) `percent` (the share used, rounded to 0.1; `null` when it can't be computed or isn't valid: nothing available, or more used than available) and `display_percent` (the share for showing to people, the preferred way to show it: in whole percent, rounded up; "0%" is guaranteed only when none is used and "100%" only when all are used, "99%" at most before that; `null` when `percent` is). Many are unused, on Xilinx FPGAs most: show the used ones first |
| `resources_summary` | list of objects | The main resources in plain words, so frontends don't need each family's resource names: the family's main capacity measure first (`logic_cells` for iCE40, `luts` elsewhere), then `flip_flops`, `ram_blocks`, `multipliers`, `plls` and `io_pins` where the FPGA has them, each with `kind`, `label` (plain words, plural, such as "logic cells"), `used`, `available`, `percent` and `display_percent` (as in `resources`; `null` also when any of its resources' numbers isn't valid). Xilinx `ram_blocks` counts 36 Kbit blocks, two 18 Kbit RAMs or FIFOs as one. More kinds may be added |
| `clocks_summary` | list of objects | The design's clocks, by clock net: each with `name` (for showing to people: usually the design's clock signal, such as `CLK` for `CLK$SB_IO_IN_$glb_clk`), `net` (the clock net nextpnr reports) and `max_mhz` (the maximum frequency, rounded to 0.01 MHz). A clock that misses its target isn't an error: compare these with the design's needs |
| `report` | multiline text | The build's report for people, the preferred way to show its resource use to people, a line each: the main resource always, the others in `resources_summary` when used, then each clock, such as "The design uses 27 of 7680 logic cells (1%)." and "Maximum clock speed: 194.4 MHz (sys_clk).", with each `display_percent`. A share that isn't valid is left out. Show it as is (a console, a CLI's output); its wording can change in any version, so programs read the fields above |
| `report_summary` | text | The report in one line, for a status bar: the main resource's share and the slowest clock, such as "Uses 1% of logic cells, max clock 194.4 MHz." Empty when there's nothing to report |
| `warnings` | list of diagnostics | Warnings from the tools. See diagnostic fields in Table 4 |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the Logix Engine. See Commands |

**Errors:** design errors, such as syntax errors or unknown pin names. Setup errors with `download_packages`. Request errors, such as an unknown board, a constraint file whose extension isn't the board's FPGA family's format, or a build folder that breaks the keyword rules. Tool failures, engine errors and cancellation. A clock that misses its intended frequency is not an error: the host compares the frequencies in `clocks_summary` with its own targets.

**Table 8.** build tool source file rules

| Topic | Rule |
| --- | --- |
| Include lookup | A Verilog `` `include `` is looked up first in the folder of the including file, then in `source_folder`. Included files don't need to be in `source_files` |
| Include location | Included files must be under `source_folder` and pass the same path checks as other source files. A file outside it is a request error |
| Unlisted modules | The engine doesn't search for modules. A module defined in a file that isn't in `source_files` is a design error that names the missing module |
| `up_to_date` | A build is up to date when the engine version, the toolchain packages, the board, the request's fields and the contents of every source, included and constraint file are the same as in the last successful build, and its bitstream is still there. Changing any of them makes the next build run again |
| Testbenches | The same rules apply to testbench files |

**Build record.** When a build succeeds, the engine writes a build record into the build folder. It holds the full board spec the build used and its checksum. The `program_board` tool reads it to get the board. A build that fails removes the old record, so `program_board` can't program a stale bitstream.

**Build folder files.** The engine creates the build folder if needed, and first writes a marker file, `.logix-build`, which tells the clean_build tool the folder is a build folder. A build makes `hardware.json` (synthesized netlist), the placed and routed design (`hardware.asc` for iCE40, `hardware.config` for ECP5, `hardware.pnr.json` for Gowin, `hardware.fasm` for Xilinx), `hardware.pnr` (nextpnr's report), for Xilinx `hardware.frames` (the configuration frames), the bitstream (`hardware.bin` for iCE40, `hardware.bit` for ECP5 and Xilinx, `hardware.fs` for Gowin), the tool logs `synth.log` and `pnr.log`, and the build record `build-record.json`, written last. These names are a fixed engine convention, the same for every build and host. Each build replaces the files the previous build made, such as the bitstream, intermediate files and reports. When a build has to run again, it deletes the previous build's outputs first, so a failed build never leaves an old bitstream that looks current. An up-to-date build keeps them. The engine never deletes files in the build folder that it didn't make, such as files put there by the host or the user.

**Example**

Input:

```json
{
  "board": "icesugar-1-5",
  "top_module": "main",
  "source_folder": "/home/user/proj",
  "source_files": ["main.v", "uart.v"],
  "constraint_file": "pinout.pcf",
  "build_folder": "/home/user/proj/_build"
}
```

Output (`resources` shortened to the used ones):

```json
{
  "bitstream": "hardware.bin",
  "board_variant": { "connection": "usb", "memory": "flash" },
  "up_to_date": false,
  "resources": [
    { "name": "ICESTORM_LC", "description": "Logic cell: a 4-input LUT, a flip-flop and carry logic", "used": 50, "available": 5280, "percent": 0.9, "display_percent": "1%" },
    { "name": "SB_GB", "description": "Global buffer, for clocks and other high-fanout signals", "used": 1, "available": 8, "percent": 12.5, "display_percent": "13%" },
    { "name": "SB_IO", "description": "I/O pin", "used": 3, "available": 39, "percent": 7.7, "display_percent": "8%" }
  ],
  "resources_summary": [
    { "kind": "logic_cells", "label": "logic cells", "used": 50, "available": 5280, "percent": 0.9, "display_percent": "1%" },
    { "kind": "ram_blocks", "label": "RAM blocks", "used": 0, "available": 30, "percent": 0.0, "display_percent": "0%" },
    { "kind": "multipliers", "label": "multipliers", "used": 0, "available": 8, "percent": 0.0, "display_percent": "0%" },
    { "kind": "plls", "label": "PLLs", "used": 0, "available": 1, "percent": 0.0, "display_percent": "0%" },
    { "kind": "io_pins", "label": "I/O pins", "used": 3, "available": 39, "percent": 7.7, "display_percent": "8%" }
  ],
  "clocks_summary": [{ "name": "sys_clk", "net": "sys_clk$SB_IO_IN_$glb_clk", "max_mhz": 89.77 }],
  "report": "The design uses 50 of 5280 logic cells (1%).\nThe design uses 3 of 39 I/O pins (8%).\nMaximum clock speed: 89.8 MHz (sys_clk).",
  "report_summary": "Uses 1% of logic cells, max clock 89.8 MHz.",
  "warnings": [
    { "file": "uart.v", "line": 13, "severity": "warning", "text": "Identifier `\\rx_done' is implicitly declared.", "tool": "yosys" }
  ]
}
```

## program_board tool

Programs a bitstream into the board it was built for, on the device the host chose. The `program_board` tool takes no board input. It reads the board spec from the build record in the build folder, so the board always matches the build. The host names the device with an `id` from `list_devices`, usually one whose `boards` include the build's board, asking the user when several do; the Logix Engine checks it, as described in the [device selection](requirements.md#device-selection) requirements, fills in the argument template of the build's variant and runs it. It never chooses a device itself.

**Note:** the `program_board` tool doesn't build. It programs the bitstream that is already in the build folder. A host that wants to build and program the board in one step calls `build` first, and calls `program_board` only if the build succeeded.

**How it runs:** the engine reads the board from the build record, finds the chosen device and checks it (still connected on the same USB port with the same USB IDs and serial number; matching the build variant's USB filter; and, when the command doesn't name the device by bus and device number or by a serial number only it has, no other connected device with its USB IDs, since the programmer would open the first one), checks the FPGA when it can (see Wrong FPGA below), fills in the variant's argument template, splitting it into words first so paths with spaces stay whole, and runs the programmer in the build folder. Progress has the phases "Finding device", "Checking FPGA" (when the FPGA is checked), "Writing flash" (or "Writing RAM") and "Verifying", with percentages read from the programmer's output. With `programmer_command` the phases are "Finding device" and "Programming", without percentages. A programmer that fails because the device is missing or can't be opened gives a setup error with `connect_board` or `fix_device_permissions`; other failures are tool failures with the programmer's output in `details`. The build record's board checksum is checked first, so a damaged record is a request error.

**Wrong FPGA:** before writing over JTAG with openFPGALoader (ECP5, Gowin and Xilinx boards), the engine runs openFPGALoader with `--detect` and only the variant's connection options, such as `--board`, `-c` and `--busdev-num`, so that run can't write anything. It compares the FPGA's ID code with the build FPGA's `jtag_idcode`. A different FPGA, such as an Arty A7-100T connected for an Arty A7-35T build, is a setup error with `connect_board` that names both FPGAs and the catalog boards with the connected FPGA that the device most likely is (those at its strongest match level, as `list_devices` rates them), and nothing is written. The detect run is in `commands`. There is no check for iCE40 boards (programmed through their flash, so the programmer never reads the FPGA's ID), other programmers, a `programmer_command`, or an FPGA without a `jtag_idcode`. The ID code identifies the chip, so a board that uses the same chip as another size, such as an iCE40 HX4K used as an 8k, can't be told apart by it.

**Flash or RAM:** the build's variant decides whether the bitstream goes to flash, where it stays after power-off, or to RAM, where it is lost at power-off (its `memory` value; boards that offer both have a choice, made when building). The write phase is "Writing flash" or "Writing RAM".

**Messages:** with live output, `program_board` says how it programs the board before it starts, naming the board (its description, without the variant), such as "Programming the RAM of the butterstick r1.0 through the FT232H adapter." ("through its USB port" for `usb`, "through its USB bootloader", "through USB" for `unspecified`; an untested variant adds " (untested)" before the period; the "of" form reads well with any name, including ones ending in parentheses), then the variant's `user_notes` as "Note: …", and after success where the design went: "Programmed the flash of the Alhambra II: the design stays when the board is powered off." or "… is lost …" for RAM. These are `message` lines; a `programmer_command` gets none, since the engine can't know what it does.

**Table 9.** program_board tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `build_folder` | build folder path | Yes | The build folder of a successful build. It holds the bitstream and the build record |
| `device` | text | Yes | The device to program: the `id` of a device from `list_devices`. It is opaque and short-lived: it stays valid while the device stays on the same USB port |
| `programmer_command` | text | No | A custom command template that replaces the variant's whole command. Its first word is a program from the toolchain package, such as `openFPGALoader`, or an absolute path. It uses the same variables as argument templates and must use `${BITSTREAM}` |
| `verbose` | boolean | No | Let the programmer print more detail. Not applied to a `programmer_command`. Defaults to false |
| `custom_boards_folder` | custom boards folder | No | The host's own boards, as sent to `build`. The build's board must still be the same under its ID (from this folder, else the catalog); otherwise the upload is a request error and nothing runs: build again. Omitted means none |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them (see Display paths) |

**Table 10.** program_board tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `device` | device | The device that was programmed. See device fields in Table 13 |
| `programmer` | catalog ID | The programmer that was used, such as `openfpgaloader`, or `custom` when `programmer_command` was given |
| `commands` | list of commands | The programs this call ran, in order, so they can be run again without the engine. See Commands |

**Errors:** setup errors with `connect_board` when the chosen device isn't connected anymore or another device is on its port (list the devices again), when the connected FPGA isn't the build's (see Wrong FPGA), or when the programmer doesn't find it (for a dfu-util board, also when it isn't in its bootloader mode); `disconnect_devices` when the command can't name the device and other connected devices have its USB IDs (icesprog, openFPGALoader with only `--board` or `-c`, such as for USB-Blaster and CMSIS-DAP adapters), or when the programmer itself can't tell them apart; `fix_device_permissions` or `install_drivers` when the device can't be opened. Request errors when `device` isn't a device ID, when the device doesn't match the board's USB filter, when the build folder has no build record, for an invalid custom board, or when the build's board isn't the same anymore. Other request errors, tool failures, engine errors and cancellation.

## list_devices tool

Lists the connected USB devices, each with an `id` for `program_board` and the boards it may be. The Logix Engine reads USB devices with libusb, built into the engine, the same library the programmers use, so the bus and device numbers mean the same to both. Devices are sorted by vendor ID, product ID, bus and device number.

**Matching.** A device matches a variant's USB filter when its USB IDs are equal, its release is equal when the filter has `release`, and, when the filter has `product_regex` or `serial_regex`, its product name or serial number contains a match for it. An unreachable device's names are unknown, so only its USB IDs and release are compared. The engine matches every device against every board on each call: the catalog's and those of the request's `custom_boards_file` (a custom board replacing the catalog board with its ID), so a host's own boards count as soon as it sends them. Each device lists the boards it matches, strongest first: by their best board variant's match level, then by board ID; custom boards have `custom: true`.

**Match levels.** Each board variant a device fits has a `match` level, since board variants can have different USB filters. Hosts use the level of the board variant they mean, such as the build's or the one the user selected. The levels say what the match means for the user, not why it matched:

- `strong`: the device is very likely this board.
- `possible`: it could be this board, but a few other boards look the same.
- `weak`: it fits, but so do many other boards, such as boards with a common USB chip.

Hosts show the level, in their own words, and compare levels (weak < possible < strong), such as to name the strongest matches and only count the weak ones. How the Logix Engine decides a level is internal: it changes between engine versions without notice, so hosts must never infer the cause of a match or rely on a level staying the same. Several boards can have the same level for one device, such as the two OrangeCrab sizes, and no level proves which board is connected: boards the catalog doesn't know can look the same.

**For a build.** Upload is the common case: with `build_folder`, a host doesn't need the build's board ID or board variant. It programs the only device listed, or asks the user when there are several, and when none is listed it says the board isn't connected, mentioning any `unreachable` device as a possible cause.

**Unreachable devices.** A device the engine sees but can't open, because of missing permissions or a missing driver, can't be programmed and its names can't be read, so it isn't in `devices`. When its USB IDs fit a board, it is listed in `unreachable` with those boards (every board variant `weak`, since its names are unknown) and the action that makes it usable: `install_drivers` on Windows, `fix_device_permissions` elsewhere. Other devices the engine can't open are left out.

**Drivers on Windows.** The programmers reach most boards through libusb, which on Windows can use only a USB interface that has WinUSB (or libusbK or libusb0), not the driver Windows picks by itself, such as FTDI's. Which interface needs it follows from the board variant's programmer: interface 0 for FTDI chips (iceprog, openFPGALoader's FTDI cables and the boards it knows by name, such as Digilent's, Alchitry's, the Tang Nanos and the ULX3S), the DFU interface for `dfu-util` (a board's bootloader), and none for `icesprog` (the iCESugar's iCELink) and CMSIS-DAP adapters, which Windows' own HID driver serves. On Windows, `list_devices` gives each such device its `driver`, from the drivers Windows has on the present devices with its USB IDs and interface (Windows gives drivers by USB IDs and interface, not by serial number, so such devices normally share it; `ready` only when every one is). Elsewhere nothing is reported. A device that isn't `ready` needs WinUSB first: `install_driver` installs it, and `uninstall_driver` undoes it.

**Device IDs.** An `id` names the device on this computer for a short time: it is made from where the device is plugged in and what it is, so it stays valid while the device stays on the same USB port, even when it is plugged in again, and survives an engine restart. IDs differ between platforms and computers. Hosts must treat them as opaque, never parse or build them, and list the devices again before asking the user to choose.

**Table 11.** list_devices tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `custom_boards_folder` | custom boards folder | No | The host's own boards (see Board specification), matched as well as the catalog's. Omitted means none |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them (see Display paths) |
| `build_folder` | build folder path | No | A successful build's folder. When given, only the devices that fit the build's board in its board variant are listed, in `devices` and `unreachable`, each with `build_match`, strongest first: the devices `program_board` can program that build into. The board comes from the build record, so it works for custom boards too |

**Table 12.** list_devices tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `devices` | list of devices | The connected devices the engine can open. See device fields in Table 13 |
| `unreachable` | list of objects | Optional. Devices that can't be opened and whose USB IDs fit a board: `id` (as in Table 13, for `install_driver`; `program_board` refuses it), `kind`, `label` (such as "USB device (0403:6010)", since its names can't be read), `vid`, `pid`, `bus`, `device`, `boards` (as in Table 13), `action`, the fix (see Table 5), and `build_match` with `build_folder` On Windows, `driver` as in Table 13 |

**Table 13.** Device fields

| Device field | Type | Description |
| --- | --- | --- |
| `id` | text | The device's ID, for `program_board`'s `device`. Opaque and short-lived |
| `kind` | enum | Always `usb` for now. Other kinds may be added later |
| `label` | text | The device for people: its product name (else its manufacturer's name, else "USB device"), then its manufacturer and USB IDs, such as "Alhambra II v1.0A (AlhambraBits, 0403:6010)" |
| `vid`, `pid` | USB ID | USB vendor and product IDs |
| `manufacturer`, `product` | text | Names reported by the device, when available |
| `serial_number` | text | Serial number, when available |
| `bus`, `device` | integer | USB bus and device number, for USB devices. The device number changes when the device is plugged in again |
| `build_match` | enum | Only with `build_folder`. How well the device matches the build's board in the build's board variant: `strong`, `possible` or `weak`, as for `boards` (`weak` for an unreachable device) |
| `boards` | list of objects | Optional, omitted when empty. The boards the device may be, best first, each with `board` (its ID), `custom` (true for a board from the custom boards folder, omitted otherwise) and `variants` (the board's variants this device fits, in the board's order, each with its `selector` and its own `match` level) |
| `likely_boards` | list of board IDs | Optional, omitted when empty. The boards the device most likely is, best first: those at its strongest match level, unless that is `weak` (many boards fit, so none is likely). Use it to name what a device probably is; `boards` has every candidate |
| `driver` | object | Optional, on Windows only, for a device whose board's programmer needs a driver (see Drivers on Windows): the board variant is the build's with `build_folder`, else the first variant of its best match. `needed` (`winusb`), `interface` (the USB interface that needs it, such as 0, an FTDI chip's first channel; its other channels, such as a serial port, keep their driver), `current` (the driver Windows has on that interface now, by its service name in lowercase, such as `ftdibus`, `usbser` or `winusb`; omitted when it has none) and `ready` (whether the programmer can use it as it is: true with WinUSB, and with libusbK or libusb0, which older guides installed) |

**Errors:** setup errors with `install_drivers` or `fix_device_permissions` when devices cannot be scanned. Request errors for an invalid custom board, or a build folder without a build record from a successful build. Engine errors.

**Example**

```json
{
  "devices": [
    {
      "id": "usb:1-1:0403:6010:FTATANKC",
      "kind": "usb", "vid": "0403", "pid": "6010",
      "manufacturer": "AlhambraBits", "product": "Alhambra II v1.0A",
      "serial_number": "FTATANKC", "bus": 1, "device": 7,
      "boards": [
        { "board": "alhambra-ii",
          "variants": [{ "selector": { "connection": "usb", "memory": "flash" }, "match": "strong" }] },
        { "board": "alchitry-au",
          "variants": [{ "selector": { "connection": "usb", "memory": "flash" }, "match": "weak" },
                       { "selector": { "connection": "usb", "memory": "ram" }, "match": "weak" }] }
      ]
    }
  ]
}
```

## install_driver tool

Windows only. Installs WinUSB on the USB interface a board's programmer needs (see Drivers on Windows, under list_devices), for a device whose `driver` isn't `ready`. Elsewhere it's a request error: no driver is needed.

**Ask the user first.** Installing a driver changes Windows for every program, so a host asks before calling it, saying which board gets WinUSB. Windows then asks whether Logix Engine may make changes (its permission prompt, UAC), and the user must answer Yes; a refusal is a setup error with `install_drivers`.

**How it runs:** the engine runs logix-core's `wdi-simple` (libwdi's command-line installer, built by logix-core from libwdi's source) to install WinUSB, the same way Zadig does. It runs it as an administrator: it starts itself again that way for this one step, so Windows' prompt names Logix Engine. libwdi signs the driver package it writes with a certificate of its own, which it adds to Windows' trusted stores (`CN=<hardware ID> (libwdi autogenerated)`); `uninstall_driver` removes it. Afterwards the engine lists the devices again, until Windows gives the device WinUSB. **Progress and output:** the phases are "Waiting for permission" (until the user answers Windows' prompt), "Installing driver" (which can take a minute or two) and "Checking driver", after any package download; the live output has the engine's messages, then, once the helper is done, each step's command line and output (`wdi-simple`'s), since the helper runs hidden, as an administrator.

**Other devices with the same USB IDs.** Windows gives drivers by USB IDs and interface, not per board, and libwdi installs on every connected device with those IDs (such as another FTDI board with the same chip). So while other such devices are connected, the tool refuses with a request error naming them and `disconnect_devices`: unplug them, or retry with `allow_same_ids` when the user agrees that they get WinUSB too. A device with the same IDs plugged in later may keep its driver: Windows chooses then. Only the interface the programmer needs changes, so an FTDI chip's other channel, such as the board's serial port, keeps its driver. Vendor tools that use that interface (such as Vivado for a Digilent board) can't until `uninstall_driver`.

**Table 39.** install_driver tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `device` | text | Yes | The device: the `id` of a device from `list_devices`, in `devices` or `unreachable` |
| `custom_boards_folder` | custom boards folder | No | The host's own boards, as for `list_devices`. Omitted means none |
| `build_folder` | build folder path | No | A successful build's folder: the driver is the one the build's board and board variant need, as `list_devices`' `driver` with the same `build_folder`. Omitted means the device's best match |
| `allow_same_ids` | boolean | No | Install even when other connected devices have the same USB IDs, which then get WinUSB too. Default false |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them |

**Table 40.** install_driver tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `device` | text | The device's `id` now: it can change once the driver lets the engine read its serial number |
| `driver` | object | The device's `driver` now, as in Table 13 (`ready` true) |
| `installed` | boolean | False when the driver was `ready` already, so nothing was done |
| `same_ids` | list of text | Optional, omitted when empty. Other connected devices with the same USB IDs, which got WinUSB too (only with `allow_same_ids`) |

**Errors:** request errors on other systems, for a device that is no known board (or not the build's), for one whose programmer needs no driver, and while other devices have the same USB IDs (`disconnect_devices`). Setup errors with `install_drivers` when the user didn't give permission, when the installation failed (with libwdi's reason, and every step's output in `details`), or when Windows still doesn't give the device WinUSB; with `connect_board` when the device isn't connected anymore; with `download_packages` when the FPGA tools package has no `wdi-simple`. Engine errors.

## uninstall_driver tool

Windows only. Undoes `install_driver` for a device: removes every driver package libwdi installed for the board's USB interface, then asks Windows to look for drivers again, so the device gets Windows' own choice back (such as FTDI's, for vendor tools). Elsewhere it's a request error. Ask the user first: Windows asks whether Logix Engine may make changes, as for `install_driver`.

**How it runs:** the engine finds the packages in Windows' INF folder (`oem<N>.inf`): those libwdi wrote (provider `libwdi`, or `libusbK` or `libusb-win32` for the libusbK and libusb0 drivers older guides installed with Zadig, and libwdi's `DeviceID` naming exactly the interface's hardware ID). Packages from Windows or a vendor never match. Then, as an administrator, it runs `pnputil /delete-driver <package> /uninstall /force` for each, removes libwdi's certificate for that hardware ID from the machine's Root and TrustedPublisher stores, and runs `pnputil /scan-devices`. Like the install, it applies to every device with the same USB IDs. Its progress and output are as `install_driver`'s, with "Removing driver" in place of "Installing driver" (the steps' output is `pnputil`'s and the certificates'). With no such package, it does nothing.

**Table 41.** uninstall_driver tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `device` | text | Yes | The device: the `id` of a device from `list_devices`, in `devices` or `unreachable` |
| `custom_boards_folder` | custom boards folder | No | The host's own boards, as for `list_devices`. Omitted means none |
| `build_folder` | build folder path | No | A successful build's folder, as for `install_driver`. Omitted means the device's best match |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them |

**Table 42.** uninstall_driver tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `device` | text | The device's `id` now (it can change with its driver) |
| `driver` | object | The device's `driver` now, as in Table 13, after Windows looked again; it may take a moment more to settle |
| `removed` | list of text | The driver packages removed, by their names in Windows (`oem<N>.inf`). Empty when there were none, so nothing was done |

**Errors:** request errors on other systems, for a device that is no known board, and for one whose programmer needs no driver. Setup errors with `install_drivers` when the user didn't give permission or a package couldn't be removed (every step's output in `details`); with `connect_board` when the device isn't connected anymore. Engine errors.

## get_catalog tool

Returns the Logix Engine's built-in catalog, fixed for each engine version, with the boards of the request's `custom_boards_folder` among them, so a board picker can list them together.

**Table 14.** get_catalog tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `boards` | catalog query | No | Which boards to return. Omitted means none |
| `fpgas` | catalog query | No | Which FPGAs to return. Omitted means none |
| `families` | catalog query | No | Which FPGA families to return, by ID, such as `ice40` or `gw1n`. Omitted means none |
| `custom_boards_folder` | custom boards folder | No | The host's own boards (see Board specification). When `boards` are fetched, they are returned with the catalog's, with `custom: true`; one with a catalog board's ID replaces that board. `boards.ids` can name them. Omitted means none |
| `display_paths` | display paths | No | How to name paths in what people read. Omitted means as the request gave them (see Display paths) |

**Table 15.** get_catalog tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `boards` | map of board definitions | Board definitions by ID, the custom boards folder's with `custom: true`, without their variants' `usb` filters: which devices are a board is the engine's own matter, and `list_devices` gives each device the boards it may be. Each variant has a `label` for people, its connection and memory labels, such as "USB · Flash" or "FT232H adapter · RAM". Boards have their `resources` when known (see resource) |
| `fpgas` | map of FPGA definitions | FPGA definitions by catalog ID |
| `families` | map of objects | FPGA families by ID (`ice40`, `ecp5`, `gw1n`, `gw2a`, `gw5a`, `artix7`, `spartan7`, `zynq7000`, as FPGA definitions name them), each with `label` (its name for people, with its vendor, such as "Lattice iCE40" or "Gowin GW2A"), `short_label` (such as "iCE40"), `constraint_extension` (its pin constraint files' extension, without the dot, such as `pcf`: the build takes only this format for its boards), `order` (its place in the engine's order, 0 first) and `toolchain` (the toolchain that builds it: `ice40`, `ecp5`, `gowin` or `openxc7`; for logs and diagnostics, while people see the family), so frontends don't keep their own lists |

**Errors:** request errors for unknown sections or IDs. Engine errors.

**Example**

Input:

```json
{
  "boards": { "fetch": true, "ids": ["icesugar-1-5"] }
}
```

Output (`resources` shortened):

```json
{
  "boards": {
    "icesugar-1-5": {
      "name": "iCESugar v1.5",
      "fpga_id": "ice40up5k-sg48",
      "dimensions": [
        { "id": "connection", "label": "Connection",
          "values": [ { "id": "usb", "label": "USB", "help": "The board's own USB port, through the programmer chip on the board." } ] },
        { "id": "memory", "label": "Memory",
          "values": [ { "id": "flash", "label": "Flash", "help": "Kept when the board is powered off." } ] }
      ],
      "variants": [
        {
          "selector": { "connection": "usb", "memory": "flash" },
          "label": "USB · Flash",
          "programmer": "icesprog",
          "args": "${BITSTREAM}",
          "tested": true,
          "user_notes": "Connect only this iCESugar when programming: its programmer can't tell iCESugar boards apart, so the engine refuses when another board with the same USB IDs, such as an iCESugar-Pro, is connected."
        }
      ],
      "resources": {
        "sys_clk": { "type": "clock", "pin": "35", "mhz": 12,
          "source": { "kind": "schematic", "ref": "github.com/wuxx/icesugar schematic/iCESugar-v1.5.pdf" },
          "user_notes": "From the on-board iCELink's 12 MHz clock output." },
        "switches": { "type": "switches", "pins": ["18", "19", "20", "21"], "active": "low",
          "source": { "kind": "schematic", "ref": "github.com/wuxx/icesugar schematic/iCESugar-v1.5.pdf" },
          "user_notes": "A 4-way DIP switch with pull-ups, on the same pins as PMOD4." }
      }
    }
  },
  "fpgas": {}
}
```

A board with a choice, such as the iCESugar-Pro, which can be programmed to flash or to RAM (dimensions' help and arguments shortened):

```json
{
  "name": "Icesugar Pro",
  "fpga_id": "lfe5u-25f-6bg256c",
  "dimensions": [
    { "id": "connection", "label": "Connection", "values": [ { "id": "usb", "label": "USB", "help": "..." } ] },
    { "id": "memory", "label": "Memory",
      "values": [ { "id": "flash", "label": "Flash", "help": "..." }, { "id": "ram", "label": "RAM", "help": "..." } ] }
  ],
  "variants": [
    { "selector": { "connection": "usb", "memory": "flash" }, "programmer": "openfpgaloader", "args": "...", "tested": true },
    { "selector": { "connection": "usb", "memory": "ram" }, "programmer": "openfpgaloader", "args": "...", "tested": true }
  ]
}
```

**Catalog contents.** The catalog is [Logix Catalog](https://github.com/openlogix/logix-catalog), a board file per board (see Board specification) and the FPGAs, one file per family, downloaded as the `logix-catalog` package, a release each engine release pins (a tool that reads boards downloads it first; it's small); `catalog` in `get_engine_info` gives its version. It has every iCE40, ECP5, Gowin (GW1N, GW2A, GW5A) and Xilinx 7-series (Artix-7, Spartan-7, Zynq-7000) FPGA the toolchain packages support, and 81 boards: 27 iCE40, 22 ECP5, 9 Gowin and 23 Xilinx, with 128 variants in all, programmed with `openfpgaloader`, `iceprog`, `dfu-util` or `icesprog`. 32 boards have a choice: memory (iCESugar-Pro, ColorLight i5, and most Xilinx and ECP5 boards, which have a RAM and a flash variant; flash is the default wherever a board has it), connection and memory (ColorLight 5A-75E V7.1 and i9 v7.2, FleaFPGA Ohm), or both with fewer combinations (ButterStick r1.0: its USB bootloader writes flash, its adapters RAM). The others have one value of each, which still tells where an upload goes: 50 variants write RAM, and the Zynq boards, the QMTECH XC7A100T and the ULX3S upload only to RAM. Every variant has a USB filter, whatever its family. Boards programmed through a cable named with openFPGALoader's `-c` option, such as `-c ft2232`, use that cable's default USB IDs. Every board has `resources`: 79 a clock with its frequency, 80 their LEDs (with `active` for 79 of them; RGB LEDs as `rgb_led`). They come from the boards' example pin files, most of them tried on hardware, and from the boards' schematics and makers; 25 boards name the source in `source`. 2 boards list PMODs, and one, the iCESugar v1.5, lists every pin it wires: its switches, its UART, its PMODs and an `unclassified` resource for the rest. Clocks that need a jumper have a `setup` requirement (the UPduino v3.0 and v3.1, the ECP5 Evaluation Board), and 5 boards list pins every design must drive: the ULX3S's ESP32 GPIO0, held high, and Fomu's USB pins, held low. Boards without an oscillator of their own (the pico-ice, whose clock comes from its RP2040) or whose clock isn't documented yet list no clock; the Tang Primer 20K is two boards, one per carrier board (Dock, with six LEDs, and Lite, without LEDs but with four PMODs); the ButterStick's seven RGB LEDs are a `led_matrix`; the Soan Papdi Mini's clock and RGB LED come from its maker.

**Trust and notes.** Each catalog variant has `tested`: true when its programming is known to work, because it was tried on the board or is used unchanged by a tool that people rely on; false when it hasn't been tried on hardware, such as a board moved to a different programmer. A change rated low risk after checking the programmer's source keeps `tested` true: every openFPGALoader command through an FTDI chip names the chosen device with `--busdev-num ${BUS}:${DEV}` (openFPGALoader's FTDI drivers then open that device), a change made without hardware tests. Optional `user_notes` give warnings and missing information for its users, such as a USB filter that also matches other boards, or a step needed before programming. The catalog file also keeps `maintainer_notes`, where each variant's settings come from and why they changed, which `get_catalog` doesn't return. A USB filter can't always tell one board model from another: boards that use the same USB chip and report the same names look alike to the engine. Hosts can show both fields where users pick a board.

## compare_boards tool

Compares two boards, such as a project's board and one it may move to. It returns facts, not advice: what they mean for a project is the host's decision. A host that keeps a project's sources as they are may warn about each difference; one that adapts projects may convert the pin file or regenerate FPGA-specific parts.

It compares boards, not variants: a board's variants differ only in how it's programmed, so they never change the result.

Every aspect is always listed, in a fixed order, each the same (`yes`), different (`no`) or `unknown`, when the catalog doesn't describe it yet, such as a board's pins: a list of only the differences couldn't tell an aspect that matches from one nobody checked. A host collects what it needs with a filter, such as the aspects whose `same` is `no`. As the catalog learns more about boards, such as their buttons or all their pins, later versions add aspects or settle unknown ones, with the same shape: a host skips aspects it doesn't know, or shows their `text`.

**Table 35.** compare_boards tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `from_board` | board ID | Yes | The first board, such as the project's board now |
| `to_board` | board ID | Yes | The second board, such as the board the project may move to |
| `custom_boards_folder` | custom boards folder | No | The host's own boards (see Board specification). Omitted means none |

**Table 36.** compare_boards tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `same_build` | boolean | True when a build for one board is valid for the other: their FPGAs have the same build settings (see What counts as the same board), so the same bitstream fits both |
| `aspects` | list of objects | Every aspect, in this order, each with `aspect` (its name), `same` (`yes`, `no` or `unknown`), `from` and `to` (the two values, when known; any JSON type, given by the aspect) and `text` (the comparison for people, such as "FPGA: ICE40HX4K-TQ144 → ICE40UP5K-SG48."). The aspects: `board` (the boards' names), `family` (the FPGA family, such as "Lattice iCE40" or "Gowin GW2A"), `fpga` (the part number), `size` (such as "8k"), `constraint_format` (the pin file's extension, such as ".pcf"), `pins` (`yes` for the same board, otherwise `unknown` for now, without values), `clock` (the frequencies in MHz of the clocks in the boards' `resources`, a list of numbers, such as `[12]`) and `leds` (how many LEDs the boards' `resources` list, a number). `clock` and `leds` are `unknown`, without values, when a board lists none, except for the same board |

**Errors:** request errors for an unknown board or variant, or a custom boards folder with problems. Engine errors.

**Example**

Input:

```json
{ "from_board": "icesugar-1-5", "to_board": "alhambra-ii" }
```

Output:

```json
{
  "same_build": false,
  "aspects": [
    {"aspect": "board", "same": "no", "from": "iCESugar v1.5", "to": "Alhambra II", "text": "Board: iCESugar v1.5 → Alhambra II."},
    {"aspect": "family", "same": "yes", "from": "Lattice iCE40", "to": "Lattice iCE40", "text": "Same FPGA family: Lattice iCE40."},
    {"aspect": "fpga", "same": "no", "from": "ICE40UP5K-SG48", "to": "ICE40HX4K-TQ144", "text": "FPGA: ICE40UP5K-SG48 → ICE40HX4K-TQ144."},
    {"aspect": "size", "same": "no", "from": "5k", "to": "8k", "text": "Size: 5k → 8k."},
    {"aspect": "constraint_format", "same": "yes", "from": ".pcf", "to": ".pcf", "text": "Same pin file format: .pcf."},
    {"aspect": "pins", "same": "unknown", "text": "Pins: not known whether they match (the catalog doesn't describe all of a board's pins)."},
    {"aspect": "clock", "same": "yes", "from": [12], "to": [12], "text": "Same clock: 12 MHz."},
    {"aspect": "leds", "same": "no", "from": 1, "to": 8, "text": "LEDs: 1 → 8."}
  ]
}
```

## get_engine_info tool

Returns the Logix Engine's version, platform and package status. A host calls it when it starts, to check that the engine is new enough and to show what is downloaded. It is fast and local: it never uses the network and doesn't verify checksums. Until the engine has read its remote config, which it does before its first operation that needs packages, `packages` is empty.

**Table 16.** get_engine_info tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| (none) |  |  | The tool takes no inputs |

**Table 17.** get_engine_info tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `engine_version` | version | The engine version, its release date as `yyyy.mm.dd` |
| `protocol_versions` | list of text | The MCP protocol versions the engine accepts, such as 2026-07-28 and 2025-11-25 |
| `platform` | text | The platform the engine runs on, such as `darwin-arm64` |
| `engine_folder` | absolute path | The engine folder, set by the host with the `--engine-folder` command-line argument when it launches the engine |
| `auto_download` | boolean | Whether automatic package download and repair is on. It is on by default, and the host turns it off with the `--no-auto-download` command-line argument when it launches the engine |
| `command_line` | list of text | The command-line arguments the engine was launched with, as given, for diagnostics |
| `log_file` | absolute path | The log file this engine process writes, in the `logs` subfolder of the engine folder |
| `packages` | list of packages | Every package the engine uses, from its remote config (empty until it has read one): `logix-catalog` (the board catalog) and `logix-core` (the FPGA tools). Each has a text `name`, a `version` (its release date as `yyyy.mm.dd`), a `status`: `ready`, `missing` or `broken`, and, for a ready `logix-core`, `components`: its tools, each with a `name` (`oss-cad-suite`, `openxc7`, `verible` and `elkjs`), a `description` and `bin_folder`, the absolute path of the folder with its programs, for hosts that run one directly, such as Verible's language server `verible-verilog-ls` from the `verible` component |
| `boards_schema` | text | The address of the board schema this engine reads, Logix Catalog's JSON Schema of a board file, in the catalog and in custom boards folders, such as `https://www.openlogix.org/schemas/logix-board-v1.json` (see Board specification) |
| `remote_config` | object | The remote config: `config_version` (such as `001`), `source` (the version folder's URL, or a local folder), `is_override` (true only for `LOGIX_ENGINE_DEV_REMOTE_CONFIG`), `channel` (such as `prod` or `dev`), `channel_reason` (`listed`: `channels.jsonc` lists this release; `default`: it doesn't, so `dev`; `local build`: always `dev`; or `LOGIX_ENGINE_CHANNEL`), `config_file` (the file read, such as `dev.jsonc`, or `prod.jsonc` when the channel has none) and `read_at`, when the one in use was read (UTC, `yyyy-mm-ddThh:mm:ssZ`). The channel fields and `read_at` are omitted before the first read |
| `catalog` | object | The board catalog: `version`, the date of the `logix-catalog` release in use, as `yyyy.mm.dd`, and `local`, true only when the engine reads a local catalog folder (`LOGIX_ENGINE_DEV_CATALOG_FOLDER`, a diagnostic setting), whose `version` is then `local` |
| `report` | multiline text | The facts above as lines for people, the engine's part of a frontend's system info (see Show system info under Recipes): the version, platform, engine folder, log file, automatic download, command line, each package with its version and status, the board catalog's version and the board schema. Show it as is; its wording can change in any version, so programs read the fields above |

**Errors:** engine errors only.

**Example**

Output:

```json
{
  "engine_version": "2026.12.01",
  "protocol_versions": ["2026-07-28", "2025-11-25"],
  "platform": "darwin-arm64",
  "engine_folder": "/Users/user/.myhost/engine",
  "auto_download": true,
  "command_line": ["--engine-folder", "/Users/user/.myhost/engine"],
  "log_file": "/Users/user/.myhost/engine/logs/logix-engine-20261028-140509-4123.log",
  "packages": [
    { "name": "logix-catalog", "version": "2026.11.10", "status": "ready" },
    { "name": "logix-core", "version": "2026.11.10", "status": "ready", "components": [
      { "name": "verible", "description": "CHIPS Alliance Verible: the Verilog formatter and language server", "bin_folder": "/Users/user/.myhost/engine/packages/logix-core/2026-11-10/verible/bin" }
    ] }
  ],
  "remote_config": {
    "config_version": "001",
    "source": "https://raw.githubusercontent.com/openlogix/logix-engine/main/remote-config/logix-engine/001/",
    "channel": "prod",
    "channel_reason": "listed",
    "config_file": "prod.jsonc",
    "read_at": "2026-12-01T09:30:00Z"
  },
  "boards_schema": "https://www.openlogix.org/schemas/logix-board-v1.json",
  "catalog": { "version": "2026.11.10" },
  "report": "Logix Engine 2026.12.01\nPlatform: darwin-arm64\nEngine folder: /Users/user/.myhost/engine\n..."
}
```

## download_packages tool

Downloads or repairs the Logix Engine's packages, reading its remote config first when that's due. Packages that are already ready are left alone, unless `force` is set. Other tools download the packages they need on their own, so a host calls this only to download ahead of time, such as before going offline, or to act on the `download_packages` action, such as when automatic download is off. It downloads whether or not automatic download is on, since the host asked. It reports progress, one phase per package, and can be cancelled.

**Table 18.** download_packages tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `packages` | list of text | No | Names of the packages to download or repair, as `get_engine_info` lists them. Omitted means every package the engine uses |
| `force` | boolean | No | Download the packages again even if they are ready, replacing the current copies. It fixes files that were damaged or deleted after the download, which the status check doesn't see. The current copy stays until the new one is complete. Defaults to false |

**Table 19.** download_packages tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `downloaded` | list of text | Names of the packages this call downloaded, repaired or downloaded again. Empty when they were all downloaded already and `force` was off |
| `packages` | list of packages | Every package the engine uses, with its status afterwards, as in `get_engine_info` |

**Errors:** request errors for unknown package names. Setup errors with `download_packages` when a download fails. Engine errors and cancellation.

**Example**

Input:

```json
{}
```

Output:

```json
{
  "downloaded": ["logix-core"],
  "packages": [
    { "name": "logix-catalog", "version": "2026.11.10", "status": "ready" },
    { "name": "logix-core", "version": "2026.11.10", "status": "ready", "components": [ ... ] }
  ]
}
```

## check_updates tool

Checks now for newer releases of the Logix Engine's packages: reads its remote config, downloads what it names that isn't downloaded yet, and uses it from the next call. The engine also checks on its own before an operation that needs packages, when it last checked more than 24 hours ago, so a host calls this only to check now, such as when the user asks for updates (Logix IDE's Help > Check for Updates). It works whether or not automatic download is on, since the host asked. It reports progress, one phase per package downloaded, and can be cancelled.

**Table 37.** check_updates tool inputs

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| (none) |  |  | The tool takes no inputs |

**Table 38.** check_updates tool outputs

| Field | Type | Description |
| --- | --- | --- |
| `updates` | list of objects | The packages whose release changed, each with its `name`, `from` (the release used before, as `yyyy.mm.dd`; omitted for a new package) and `to` (the release used now). Empty when nothing changed |
| `downloaded` | list of text | Names of the packages this call downloaded or repaired |
| `packages` | list of packages | Every package the engine uses, with its status afterwards, as in `get_engine_info` |
| `remote_config` | object | The remote config just read, as in `get_engine_info` |

**Errors:** setup errors with `download_packages` when the remote config can't be read and there's none to use, or a download fails. Engine errors and cancellation.

**Example**

Output:

```json
{
  "updates": [{ "name": "logix-catalog", "from": "2026.11.24", "to": "2026.11.30" }],
  "downloaded": ["logix-catalog"],
  "packages": [
    { "name": "logix-catalog", "version": "2026.11.30", "status": "ready" },
    { "name": "logix-core", "version": "2026.11.10", "status": "ready", "components": [ ... ] }
  ],
  "remote_config": {
    "config_version": "001",
    "source": "https://raw.githubusercontent.com/openlogix/logix-engine/main/remote-config/logix-engine/001/",
    "channel": "prod",
    "channel_reason": "listed",
    "config_file": "prod.jsonc",
    "read_at": "2026-12-01T09:30:00Z"
  }
}
```

## clean_build tool

Deletes build folders. It is given a folder and deletes only build folders, which the Logix Engine marks with a `.logix-build` file when it first builds in them:

- **A build folder,** such as `/home/user/proj/_build`: the tool deletes it with everything in it.
- **A folder of build folders,** such as `/home/user/proj/builds` with one build folder per board or board variant: the tool deletes each build folder directly inside it, then deletes the folder itself if nothing else is left in it.

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
{ "folder": "/home/user/proj/builds" }
```

Output:

```json
{
  "deleted": [
    "/home/user/proj/builds/icesugar-1-5",
    "/home/user/proj/builds/icezum",
    "/home/user/proj/builds"
  ]
}
```

## Default project model

The Logix Engine doesn't impose a project model: each frontend organizes designs its own way, and requests name every folder and file. The engine doesn't need a project file, in JSON or any other format, and never reads one.

This model is only a suggestion for AI sessions that have no other requirements, such as an AI agent starting a design from scratch with only the engine. It isn't a standard or a recommendation for frontends and hosts, which keep their own models. Use it only for a new project, or a folder without a layout of its own.

- **One folder** holds the project, with every file at its top level.
- **`logix.json`** holds the request values that would otherwise have to be remembered between sessions, under their request field names:

  ```json
  { "board": "icesugar-1-5", "board_variant": { "connection": "usb", "memory": "flash" }, "top_module": "main" }
  ```

  `board` is a board ID. For a board that isn't in the catalog, write its definition as a board file in a custom boards folder, such as `boards/my-board.json` in the project folder (see Board specification), and send the folder as `custom_boards_folder` with every request. `board_variant` is optional: without it, requests use the board's first variant. Copy the values into each request. The engine never reads this file: it is only the AI session's notes.
- **`main.v`** holds the top module, `main`, or **`main.sv`** for SystemVerilog. Other source files sit next to it.
- **One constraint file,** `pinout` with the extension of the FPGA's format: `pinout.pcf` (iCE40), `pinout.lpf` (ECP5), `pinout.cst` (Gowin) or `pinout.xdc` (Xilinx). `get_catalog` gives the board's FPGA.
- **Testbenches** are named `<name>_tb.v` or `<name>_tb.sv`, next to the code they test (see Writing testbenches).
- **GTKWave layouts** are named `<testbench>.gtkw`, next to the testbench, such as `main_tb.gtkw`, and passed as `layout_file`, so a layout the user saves is kept.
- **`_build`** inside the project folder is the build folder. It is disposable (see Folders), so add it to `.gitignore` in a git repository.

## Recipes for frontend developers

The Logix Engine was designed together with the Logix IDE, and its tools map to what a frontend shows: buttons, panels, a console and a device picker. These recipes give the intended flow for common tasks; the tool sections above have the fields. The Logix IDE is only an example here. The Logix Engine doesn't depend on it or on any of its traits: a frontend can be an editor, a graphical or block-based design tool, a command-line tool, a CI job or anything else.

**Reference frontend.** The Logix IDE ([github.com/openlogix/logix-ide](https://github.com/openlogix/logix-ide), MIT or Apache 2.0) implements every recipe below, and its code is meant as a worked example for any frontend, from Open Logix or not. Read it with its AI spec, its `CLAUDE.md`, which says what each part does and why:

| Recipe | In the Logix IDE |
| --- | --- |
| Start up, calls, progress and live output | [engine/client.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/engine/client.rs) (the MCP session), [setup.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/engine/setup.rs) (download, start, version check) |
| Show whether the board is connected | [engine/devices.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/engine/devices.rs), [ui/src/lib/boards.ts](https://github.com/openlogix/logix-ide/blob/main/ui/src/lib/boards.ts) (device groups), [ui/src/lib/devices.ts](https://github.com/openlogix/logix-ide/blob/main/ui/src/lib/devices.ts) (match levels in words) |
| Upload: build, pick a device, program | [engine/task.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/engine/task.rs) (`pick_device`), [ui/src/lib/DevicePicker.svelte](https://github.com/openlogix/logix-ide/blob/main/ui/src/lib/DevicePicker.svelte) |
| Show problems, build results and testbench results | [engine/task.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/engine/task.rs), [ui/src/lib/buildReport.ts](https://github.com/openlogix/logix-ide/blob/main/ui/src/lib/buildReport.ts), [ui/src/lib/tasks.ts](https://github.com/openlogix/logix-ide/blob/main/ui/src/lib/tasks.ts) |
| Pick a board and its board variant | [engine/catalog.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/engine/catalog.rs), [ui/src/lib/BoardPicker.svelte](https://github.com/openlogix/logix-ide/blob/main/ui/src/lib/BoardPicker.svelte), [ui/src/lib/variants.ts](https://github.com/openlogix/logix-ide/blob/main/ui/src/lib/variants.ts) |
| A project's own boards | [commands.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/commands.rs) (`boards_file`, `custom_boards_stamp`), [engine/catalog.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/engine/catalog.rs) (`project_boards`, `BoardsProblem`, `BoardsFile` for `display_paths`), [ui/src/lib/boards.ts](https://github.com/openlogix/logix-ide/blob/main/ui/src/lib/boards.ts) (`boardsProblemLines`) |
| Format an unsaved editor buffer | [engine/format.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/engine/format.rs) |
| Show system info | [system_info.rs](https://github.com/openlogix/logix-ide/blob/main/src-tauri/src/system_info.rs), [ui/src/lib/SystemInfoDialog.svelte](https://github.com/openlogix/logix-ide/blob/main/ui/src/lib/SystemInfoDialog.svelte) |

Logic that every frontend would need and that doesn't depend on a frontend's own model belongs in the Logix Engine, not in each frontend: when a frontend finds itself repeating engine knowledge, such as FPGA family names or the toolchain's resource names, that is a gap to fill in the engine.

### Start up

1. Launch the Logix Engine with `--engine-folder`, do the MCP handshake, then call `get_engine_info`.
2. Check that `engine_version` is one your frontend supports, by its own rule, such as a version range or a list of tested releases. If it isn't, call no other tools: tell the user which Logix Engine versions the frontend needs, then stop or fall back as the frontend decides.
3. Call `download_packages` and show its progress. It reads the engine's remote config when that's due and downloads only what's missing, so it returns at once when everything is ready, and it's how a first start downloads the packages with the host's own progress screen. (Tools would download what they need on first use anyway.) Then show package status from its `packages`. To check for newer packages now, call `check_updates`.

### Show whether the board is connected

1. Call `list_devices` every few seconds while the status is on screen, and pause while a task uses the board. The Logix Engine has no device-change notification, so the frontend polls.
2. Look for the selected board and board variant in each device's `boards`. Show "connected" when a device fits, and "not connected" when none does; Upload then checks. A frontend can use the `match` levels to order the boards a device may be, naming the strongest and counting the weak ones, but no USB match proves which board is plugged in.
3. When no device fits but `unreachable` has one, show its `action` instead, such as fixing device permissions. A device seen but not usable is a common reason a board "isn't found".
4. Device IDs stay the same while a device stays on its USB port, so a selection survives polling and replugging into the same port.

### Upload: build, pick a device, program

1. Call `build`. If it fails, show its diagnostics and stop; `program_board` never builds.
2. Call `list_devices` with the build folder. It lists only devices that fit the build's board and board variant, so the frontend never needs the board ID here.
3. One device: use it. Several: ask the user, with a `strong` `build_match` device pre-selected when there is exactly one. None: say the board isn't connected, and mention any `unreachable` device and its action.
4. Call `program_board` with the build folder and the chosen device `id`.
5. On `connect_board` or `disconnect_devices`, list the devices again and let the user retry. Never build or guess a device ID.

On Windows, a device whose `driver` isn't `ready` (in `devices` or `unreachable`) needs WinUSB first: before step 4, explain it to the user (which board, that Windows will ask for permission, and that other devices with the same USB IDs get it too) and, if they agree, call `install_driver` with the device and the build folder, then list the devices again. Offer `uninstall_driver` somewhere the user can find it later, such as next to the device, for when vendor tools need the board back.

### Show progress and a console

- Add a progress token to every long call. Show an overall bar from `progress / total`, the phase message as its label, and the phase's own percentage as `progress mod 100`. A phase without percentages shows a spinner.
- For a console, add `"logix/output": true` to the call's `_meta` and append each `logix/output` line, styled by `kind` and `level`. Hold a line with `redraw` true and drop it when the same tool's next line arrives, so a progress bar stays on one line; show it if anything else comes first or the call ends. Don't parse these lines; the result has the diagnostics.
- A "copy command" action can use each `commands` entry's `shell` line, so users can rerun a step or report a tool bug without the engine.
- Cancel with an MCP cancel notification for the call's request ID.

### Show problems in the editor

- Every tool reports problems as diagnostics with `file` relative to the source folder, `line`, `column` and `severity`, so one routine can underline them for lint, test, build and format alike.
- `lint` is fast and writes nothing, so it suits running on save. Build warnings come in `warnings`.
- Show a build's result with the engine's text: `report_summary` in a status bar ("Uses 1% of logic cells, max clock 194.4 MHz.") and `report` in a console, so every frontend words it the same way. For a full report, show `resources` as a table: the toolchain's name, its description, used, available and `display_percent` ("n/a" when it's `null`), the used ones by default, with a way to show the rest (on Xilinx FPGAs most are unused), and `clocks_summary` with each clock's name, maximum speed and net. Logix IDE's Board Utilization window does this (`ui/src/lib/BuildReport.svelte`).
- Turn an error's `action` code into a button: `download_packages` calls the download_packages tool, and `connect_board`, `fix_device_permissions`, `install_drivers` and `disconnect_devices` show short instructions.

### Run and debug testbenches

1. "Run tests" calls `test` with every testbench. Each stops at its first failure, so a failing run ends fast, and each result says where and when it failed.
2. "Debug" on a failing testbench calls `simulate` with a `layout_file` next to the testbench, such as `main_tb.gtkw`. GTKWave opens zoomed in around the first failure.
3. If the user saves the layout in GTKWave, the Logix Engine keeps it from then on; an unsaved default is deleted when GTKWave closes.

### Pick a board

- Fill a board picker from `get_catalog` with `boards.fetch`, `fpgas.fetch` and `families.fetch` true: group boards by their FPGA's `family`, with the family's `label` and `order`, and use its `constraint_extension` for the board's pin file.
- Show each board's `dimensions` labels and help as the board variant choices, each variant's `label` as the current choice, and its `tested` and `user_notes` where the user picks. Keep only the values the user chose, such as `{"memory": "ram"}`, and send them as `board_variant`: the engine uses the board's first variant with them, the same rule a host uses to show which variant that is.
- A user's own boards are custom boards: keep them in a folder in the project, such as `boards/`, a file per board named by its ID, so they work on every platform and anyone who opens the project gets them. Send the folder's path as `custom_boards_folder` with every request, whatever board is selected, and list its boards with the catalog's by sending it to `get_catalog`. The frontend needn't edit, parse or copy them: users edit it with any editor (with a `"$schema"` line naming `boards_schema` from `get_engine_info`, editors that read JSON Schemas check and complete it), the Logix Engine reads it, and a frontend only notices changes on disk, shows the problems the engine reports at line and column, and marks its boards where the user picks so they aren't mistaken for catalog boards.
- When the user changes a project's board, call `compare_boards` with the old and new boards and decide from its aspects what the change means for your project model: warn about each difference, or adapt the project, such as converting the pin file when `constraint_format` differs. `same_build` tells whether an existing build still fits.

### Format an unsaved editor buffer

Requests never carry source text. Write the buffer to a file in a scratch folder, call `format` on it, and read the result back into the editor.

### Show system info

Give users one place, such as Help > System Info, with a plain text report and a Copy button, for bug reports. Every frontend shows the same report, so a report reads the same whichever frontend it came from:

1. The frontend's own lines: its name and version, the operating system and its version, its own log file, and which Logix Engine it runs: the release it pins, or another binary (such as a local build).
2. A blank line, then `report` from `get_engine_info`, as is. Call `get_engine_info` when the report is opened, so package status is current. If the engine isn't running, say so in its place.

## Open questions

These are not decided and none of them is in the API today. They are listed so API users can see what may change and comment.

- **Skipping build steps whose inputs didn't change.** Today a build is up to date as a whole, by a checksum of all its inputs, and otherwise reruns synthesis, place and route, and packing. Incremental build tools such as SCons skip steps whose own inputs are unchanged, so a change to only the constraint file skips synthesis. Per-step checksums in the build record would do the same. Each step's checksum must also cover the versions of the tools and packages it runs, and the Logix Engine version, since a new version can produce different output from the same inputs. Most edits change the sources, which rerun every step anyway, so this waits until large designs make pin-only rebuilds slow.
- **What clean_build may delete.** Today it deletes only folders the Logix Engine marked with `.logix-build`, so it leaves old build folders made by other tools alone until the engine builds in them. The alternative is to delete any folder that passes the build folder path checks. That cleans other tools' old folders too, but a wrong path such as `/home/user/builds/myproject` passes those checks, since one of its last two folder names contains "build". A middle way would be to also refuse folders that directly contain design files, such as `.v`, `.sv` or `.pcf` files.
- **Extra tool arguments for build.** Other FPGA tools let users pass extra options to the synthesis and place-and-route tools, so the build tool likely needs something similar. One option is to define abstract build phases, such as synthesis, place and route, and bitstream, offer portable options such as a target frequency and a seed, and add extra arguments per phase as an explicit escape hatch, passed unchanged to whatever tool runs that phase. The build output would then report which tool and version ran each phase.
- **Include folders for build.** Some projects keep header files in a separate folder. An optional `include_folders` input, a list of relative paths under `source_folder`, would let the host name extra places to look up included files.
- **Separating simulation from viewing.** Today `simulate` writes the waveform and also opens GTKWave. Longer term, `simulate` could only write the waveform and return the top-level signals, with opening a viewer as an optional step or its own tool. GTKWave layouts would then be one viewer's concern, and other viewers, such as Surfer or a waveform view inside an IDE, would fit the same way. It only adds fields and tools, so it breaks nothing.
