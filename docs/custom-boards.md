# Custom boards

Logix Engine knows the
[supported boards](https://www.openlogix.org/logix-catalog/boards/). When
yours isn't on the list, or you want to program a supported board another
way, describe it yourself in a small file: a **custom board**. Every tool
then builds for it, programs it and finds it among connected devices like
any other board.

A custom board can use any of the
[supported FPGAs](https://www.openlogix.org/logix-catalog/fpgas/), also
those no supported board has yet. Custom boards add boards, never FPGAs.

This page is for people using the engine with an AI agent, such as in the
[Claude Code tutorial](tutorial-claude-code.md), and for developers of
frontends. Using Logix IDE? Its
[Custom boards](https://www.openlogix.org/logix-ide/custom-boards/) page
says how it does it.

## How it works

- Custom boards are in a **custom boards folder**, one file per board. It
  can be any folder; a project's **boards** folder is the usual choice.
- Each file is named after the board's **ID**, such as `my-upduino.json`
  for the board `my-upduino`. An ID has only lowercase letters, digits and
  dashes. The engine reads every `*.json` file in the folder and ignores
  other files.
- The files are in the same format as the supported boards, Logix Catalog's
  [board format](https://www.openlogix.org/logix-catalog/model/). So you can
  start from a supported board's file, and a board you describe can be added
  to the supported boards for everyone, as it is.
- Every tool that deals with boards (`build`, `lint`, `test`, `simulate`,
  `diagram`, `program_board`, `list_devices`, `install_driver`,
  `uninstall_driver`, `get_catalog` and `compare_boards`) takes the folder's
  absolute path as `custom_boards_folder`. The engine keeps nothing between
  requests, so send the same folder with every request.
- Its boards are used like the supported boards: `board` names one by its
  ID, and `get_catalog` lists them with the others, marked `custom: true`.

## With an AI agent

An agent such as Claude Code can write the file, check it with the engine
and use it. Tell it what you know about the board: its name, its FPGA, how
it's programmed and, best of all, a link to its schematic or its maker's
docs. With the board plugged in, the agent finds its USB IDs with
`list_devices`.

**AI prompt:**

```text
My board, the <name>, isn't one of Logix Engine's supported boards. Write
a custom board for it in the folder boards here, starting from the most
similar supported board. Its FPGA is <part number>, it's programmed over
USB through <chip>, and it's plugged in now. Its schematic is at
<link>. Then check it with the engine and tell me what you couldn't find.
```

The agent finds the FPGA's ID and similar boards with `get_catalog`, writes
the file, and sends the folder as `custom_boards_folder` with every request
from then on. If the file has a mistake, the engine says where, at file,
line and column, and the agent fixes it.

> [!TIP]
> Check what the agent wrote before you program the board, above all the
> pins: an agent may guess where the schematic doesn't say. Ask it which
> facts come from where.

## Write one yourself, step by step

1. **Find your board's FPGA ID.** Ask `get_catalog` for the FPGAs of your
   board's family (`fpgas`), and find its part number: each FPGA's ID, such
   as `ice40up5k-sg48`, is the exact chip and package. The
   [supported FPGAs](https://www.openlogix.org/logix-catalog/fpgas/) list
   them by part number.

2. **Find a similar supported board.** A board with the same FPGA, or the
   same programming chip (most often an FTDI chip), is the best start. The
   [supported boards](https://www.openlogix.org/logix-catalog/boards/) list
   shows each board's ID under its name, and its FPGA.

3. **Copy its file.** Each supported board's file is in Logix Catalog's
   [boards folder](https://github.com/openlogix/logix-catalog/tree/main/boards)
   on GitHub, named after its ID. Copy it into your custom boards folder,
   named after your board's ID, such as `boards/my-upduino.json`. (A board
   from `get_catalog` lacks its variants' `usb` filters, which a board file
   needs: the file has them.)

4. **Describe your board.** Change the fields as the next sections say: its
   `name`, its FPGA (`fpga_id`), how to program it (`variants`) and,
   optionally, what's on it (`resources`). Leave out what you don't know.

5. **Check it.** Any board tool checks every file in the folder, such as
   `get_catalog` with `{"boards": {"fetch": true, "ids": ["my-upduino"]}}`
   and the folder as `custom_boards_folder`. A mistake comes back as an
   error with a diagnostic per problem (see When something's wrong).

6. **Build and program.** Write the design's pin file for your board's
   pins, from its schematic or its maker's docs, then `build` and
   `program_board` with `"board": "my-upduino"` and the same
   `custom_boards_folder`. When programming works, set `tested` to `true`.

## A board's file

A small but complete file, for an UPduino programmed over its own USB port:

```json
{
  "$schema": "https://www.openlogix.org/schemas/logix-board-v1.json",
  "name": "My UPduino",
  "fpga_id": "ice40up5k-sg48",
  "variants": [
    {
      "selector": {"connection": "usb", "memory": "flash"},
      "programmer": "iceprog",
      "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
      "usb": {"vid": "0403", "pid": "6014"},
      "tested": false
    }
  ],
  "resources": {
    "clk": {"type": "clock", "pin": "35", "mhz": 12},
    "rgb": {"type": "rgb_led", "red": "41", "green": "39", "blue": "40", "active": "low"}
  }
}
```

| Field | Required | What it holds |
| --- | --- | --- |
| `$schema` | no | The format's JSON Schema, always the address above. With it, editors such as VS Code, JetBrains IDEs, Zed or Neovim check the file and complete its fields as it's written. When it's there, it must be this address |
| `name` | yes | The board's name for people |
| `fpga_id` | yes | The board's FPGA, by its FPGA ID (`get_catalog`'s `fpgas`) |
| `variants` | yes | The ways to program the board, the first being the default (see Variants) |
| `requires` | no | What every design for the board must do, such as a pin to hold high (`drive`), or what a person must do first (`setup`) |
| `resources` | no | What the board wires to FPGA pins: clocks, LEDs, buttons, UARTs, PMODs and headers (see Resources) |
| `ai_hints` | no | Short lines for AI agents writing designs for the board, about what the other fields can't say. Always the last field |

Logix Catalog's [board format](https://www.openlogix.org/logix-catalog/model/)
describes every field in full, and the [Tool API](tool-api.md#board-specification)
says how the engine reads them.

### Variants

Each variant is one way to program the board, such as writing the flash
(kept at power-off) or the FPGA's RAM (lost at power-off). Requests choose
one with `board_variant`, some or all of its selector; without it, the
first is used.

| Field | What it holds |
| --- | --- |
| `selector` | What tells this variant apart: `connection`, how the computer reaches the board (`usb`, its own USB port; `bootloader`, its USB port in bootloader mode; `ft2232h`, `ft232h`, `cmsis-dap` or `usb-blaster`, an external adapter; or `unspecified`), and `memory`, `flash` or `ram`. No two variants share one |
| `programmer` | The program that programs the board: `openfpgaloader`, `iceprog`, `dfu-util` or `icesprog`. The engine downloads them with the FPGA tools |
| `args` | The programmer's arguments, with these variables: `${BITSTREAM}` (the file to write), `${BUS}` and `${DEV}` (the board's USB bus and device numbers), `${VID}` and `${PID}` (its USB IDs), `${SERIAL_NUM}` (its serial number) and `${USB_PATH}` (its USB port path, as `dfu-util`'s `-p` takes it) |
| `usb` | How the engine finds the board among connected USB devices: `vid` and `pid`, its USB IDs as four hex digits each, and optionally `product_regex` (a regular expression for its product name), `serial_regex` (for its serial number) and `release` (its chip's release number, USB `bcdDevice`), for boards that share USB IDs |
| `tested` | `true` once programming this way is known to work |
| `user_notes`, `maintainer_notes` | Optional notes, for people using the board and for whoever maintains the file |

The arguments depend on the programmer and the board's USB chip, so copy
them from a supported board that's programmed the same way. For
`openfpgaloader`, the board may also be one it knows by name (`-b` and the
name), as many supported boards are.

### Your board's USB IDs

The `usb` filter needs your board's USB IDs. `list_devices` lists every
connected USB device with its USB IDs (`vid` and `pid`), also those that fit
no board yet: call it with the board unplugged and again with it plugged
in, and the new device is your board. An agent can do it for you. Or use
your system's tools, with the board plugged in:

- **macOS:** open **System Information** (hold Option and choose **Apple
  menu > System Information**), then **Hardware > USB**, and select your
  board: **Vendor ID** and **Product ID** are its USB IDs, such as `0x0403`
  and `0x6014` (write them without `0x`).
- **Windows:** in **Device Manager**, open your board's **Properties**, then
  **Details**, and pick **Hardware Ids**: `USB\VID_0403&PID_6014` gives
  `0403` and `6014`.
- **Linux:** run `lsusb`. Your board's line has its USB IDs, such as
  `ID 0403:6014`.

Once the file has them, `list_devices` finds the board, and on Windows and
Linux says whether it needs a driver or a udev rule first, which
`install_driver` adds, as for supported boards (see the
[Tool API](tool-api.md#list_devices-tool)).

### Resources

`resources` lists what the board wires to FPGA pins, each by the name a
design would give its port, such as `clk` or `leds`. `get_catalog` returns
them, so AI agents and frontends know the board's clock, LEDs and pins
without guessing. They're optional, and builds don't read them: the
design's pin file says where each signal goes.

| Type | Fields | Example |
| --- | --- | --- |
| `clock` | `pin`, `mhz` | `"clk": {"type": "clock", "pin": "35", "mhz": 12}` |
| `leds` | `pins` (bit 0 first), `active` | `"leds": {"type": "leds", "pins": ["11", "37"], "active": "low"}` |
| `rgb_led` | `red`, `green`, `blue`, `active` | `"rgb": {"type": "rgb_led", "red": "41", "green": "39", "blue": "40", "active": "low"}` |
| `buttons`, `switches` | `pins`, `active` | `"btn": {"type": "buttons", "pins": ["10"], "active": "low"}` |
| `uart` | `tx` (the FPGA's output), `rx` | `"uart": {"type": "uart", "tx": "9", "rx": "6"}` |
| `pmod` | `pins`: the 8 signal pins, 1 to 4 then 7 to 10 | `"pmod1": {"type": "pmod", "pins": ["4", "2", "47", "45", "3", "48", "46", "44"]}` |
| `header` | `positions`: what each position is wired to | other connectors |

`active` is `high` or `low`: whether the LED lights, or the button reads,
at a high or a low level. Leave it out if you don't know. Pins are named as
the FPGA's pin file names them.

## When something's wrong

Every board tool checks every file in the folder, so a mistake in any file
stops every board tool, whichever board the request names, until it's
fixed. The error has a diagnostic per problem, at file, line and column, as
for source files, such as
`boards/my-upduino.json:4:14: error: "ice40up5k-sg" isn't an FPGA in the catalog. Use get_catalog to list them.`. They
all come at once, at most 10. Fix the file and send the request again.

The one exception is `list_devices`: it still lists the devices, matched
against the supported boards only, and returns the problem in
`custom_boards_problem`, so a frontend's device list keeps working while
the file is being fixed.

Common mistakes:

- The file name isn't the board's ID with `.json`, or the ID has capital
  letters, spaces or underscores.
- `fpga_id` isn't an FPGA ID (`get_catalog`'s `fpgas` has them).
- A missing comma or quote. With the `$schema` line, an editor marks it as
  you type.
- `program_board` doesn't find the board: check its USB IDs, and the
  variant's `connection`.

## Replace a supported board

A custom board with the same ID as a supported board replaces it, for the
requests that send its folder: for building, matching devices and listing.
Copy the supported board's file with its name unchanged, and edit it, such
as to program it another way.

## In a frontend

A frontend that lets its users add boards gives each project a custom
boards folder and sends it with every request. A few things to know:

- **Only what the bitstream depends on counts.** After a build, a change to
  a custom board needs a new build only when its FPGA changes; its
  programming, name and resources can change freely. See
  [What counts as the same board](tool-api.md#board-specification).
- **Show the diagnostics** where users edit the files, and name the files
  as they know them with `display_paths`.
- **Notice changes** to the folder (the engine keeps nothing between
  requests), and fetch the boards again with `get_catalog`.

Logix IDE does all of this, as a reference: its
[Custom boards](https://www.openlogix.org/logix-ide/custom-boards/) page,
and its code on [GitHub](https://github.com/openlogix/logix-ide).

## Share it with everyone

A custom board is already in the supported boards' format. To add it to
them, so every Open Logix tool knows it, send its file to Logix Catalog as
a pull request: see
[Adding or fixing a board](https://www.openlogix.org/logix-catalog/#adding-or-fixing-a-board).
