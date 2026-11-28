# In-design debugger: requirements and design

This is the spec for Logix Engine's in-design debugger, the Logix debugger. It
was designed and first built in a separate prototype and moved into Logix
Engine on 2026-11-27 (Q12); this file is now the source of truth. Its tools are
in the [Tool API](tool-api.md) (Debugging a design on the board).

> **Reading the older sections.** Sections written before the move say
> "soft-agent" for the prototype that held the debugger and `debug_build` for
> what is now `build` with `debug_file`; read them as Logix Engine and `build`.
> The prototype used the engine unchanged, through its public API; the
> debugger is now part of the engine itself (`src/debug/`).

## Summary and goals

A debug build adds a small debugger to the user's design, so a host can read
and write chosen nets while the design runs on the board. A debug
configuration file names the nets; the build inserts the debugger and its link
to the host (a UART over the board's USB cable first). Everything is
deterministic tooling: no AI at build time or run time. AI agents are only one
possible client of the host tools, like any other.

Goals:

- **Observe:** read the current value of named nets (state machines,
  counters, flags, bus registers) without adding ports or editing the design.
- **Control:** write named nets or registers, such as forcing an enable,
  injecting a value or holding a reset.
- **No design changes needed:** a design builds the same with or without
  debugging; the debug build only adds.
- **Movable:** built so the feature can later move into Logix Engine without
  changing the file format or the tools.

**Next phases, planned** (2026-11-28): triggers and counters first, then
capture on the same triggers. A trigger watches chosen signals for a
condition in the design's own clock; a counter counts a trigger's events,
so it sees one-clock events that the host's polling misses; capture
records chosen signals around a trigger on the board and sends them up to
the host, to view as a waveform (like ILA or SignalTap). Each is optional:
a debug file without them builds the debugger as today, with none of their
logic. The first version leaves room for them in the file format, the
debugger and the protocol, as each section notes.

Out of scope for both: on-chip CPUs, JTAG uplinks and multi-board setups.

## Background: how designs build today

The engine's `build` tool runs one yosys call, then the toolchain's place and
route and packer. The debug build has to fit around that flow, since
soft-agent uses the engine unchanged.

- **Synthesis:** one yosys run reads the listed `.v`/`.sv` files with `-D`
  defines (`SYNTHESIZE` always) and runs `hierarchy -top <top>; setattr …;
  synth_ice40 -top <top> -json hardware.json` (`synth_ecp5`, `synth_gowin`,
  `synth_xilinx` for the other families). The script is fixed by
  `build::synth_script`; a request can't add passes to it.
- **Place and route, packing:** `nextpnr-ice40` with the constraint file, then
  `icepack` (iCE40); the other families have their own pair.
- **Inputs are explicit:** every request names its source folder, source
  files, top module, constraint file and build folder. The engine searches for
  nothing and keeps no project file.
- **Board facts come from the catalog:** `get_catalog` gives each board's
  clock (pin, MHz) and UART pins. The iCESugar v1.5 has a 12 MHz clock on pin
  35 and its USB UART on TX 6, RX 4.
- **Serial is already there:** `open_serial`, `read_serial`, `write_serial`
  and `close_serial` keep a session on a board's UART (8N1, 115200 by
  default), pause it while `program_board` writes that board, and reopen it
  after.
- **Extension point:** soft-agent can add tools and call the engine's public
  library code, but can't change the engine's `build`. A debug build therefore
  prepares sources that the engine's build then synthesizes (Build
  integration, below).

## Requirements

The first version is the Must rows; Should rows are wanted if cheap; the Next
phase row (capture) is planned and shapes today's choices; Later rows only
constrain them.

| ID | Requirement | Priority |
| --- | --- | --- |
| R1 | A debug configuration file names the nets to read and to write, by hierarchical name from the top module (such as `main.u_ctrl.state`); widths come from the design. | Must |
| R2 | Nets can also be marked in the design itself (an attribute or a probe module), and the build finds them. | Later |
| R3 | A debug build of an unchanged design adds the debugger; a normal build of the same sources has no trace of it. | Must |
| R4 | The host reads any listed net, and writes any net listed as writable, by name. | Must |
| R5 | Reads of several nets in one request are taken in the same clock cycle (a coherent snapshot). | Must |
| R6 | The uplink is a UART: the board's from the catalog when it has one, else any two pins with a USB serial adapter, at a baud rate the clock supports. | Must |
| R7 | The build writes a debug map (names, widths, directions, addresses, clock) into the build folder, and the host tools take names from it, never from the sources. | Must |
| R8 | The debugger reports an ID of the build it is in, and the host refuses a map that doesn't match the bitstream on the board. | Must |
| R9 | A name that doesn't exist or was optimized away is a design error at build time. | Must |
| R10 | The debugger's resource use shows in the build report, apart from the design's. | Should |
| R11 | Nets in other clock domains than the debugger's are read safely (synchronized), and the map says which are. | Later |
| R12 | When the design already uses the UART pins, the build refuses with a clear error, or the configuration picks other pins. | Should |
| R13 | Works for every family the engine builds (iCE40, ECP5, Gowin, Xilinx 7), with no per-family code in the debugger. | Should |
| R14 | Capture (the planned next phase): record listed signals on the board, such as into block RAM from a trigger condition, and send them up to the host as a waveform. The first version's file, debugger and protocol leave room for it. | Next phase |
| R15 | Other uplinks: JTAG, SPI, USB through a soft USB core. | Later |
| R16 | Triggers (the next phase, before capture): conditions on listed signals, checked every clock of the design: a value under a mask, a rising or falling edge or any change, and AND or OR of a few of them. Which signals a trigger looks at is fixed by the build; its condition is set from the host while the design runs, with no rebuild. Sequences (A then B within N clocks) and a trigger output pin come later. | Next phase |
| R17 | Counters (with triggers): each counts one trigger's events, every clock. A counter saturates at its limit and never wraps: binary all-ones by default, or a decimal limit such as 99999, so a counter at its limit means that many or more. The host reads counters as read-only values in the same snapshot as the watched nets, and clears them. | Next phase |
| R18 | Triggers, counters and capture are each opt-in: a debug file without them builds no logic for them, and a normal build has none of the debugger. | Next phase |

Non-functional: the debugger stays small (a target under 300 iCE40 logic
cells for 8 probes; to be measured), it must not lower the design's maximum
clock much, and the protocol and file format are documented and versioned, so
other hosts can use them without soft-agent.

## Languages

Verilog and SystemVerilog, the same as the engine. The debugger works on the
netlist yosys builds from the source, so net names are the same instance
paths in either language. It covers what the engine's yosys reader
(`read_verilog -sv`) accepts. Version 1 takes plain nets and registers;
SystemVerilog struct fields, interface signals and packed-array slices need
their own naming rules and come later. No VHDL yet, since the engine doesn't
support it.

## The debug configuration file

One TOML file per debug build, named in the build request like the
constraint file (such as `debug.toml`), with a schema generated from Rust
types as the engine does for its tools. It says which nets
the debugger sees, how it reaches the host, and which clock it runs on.

Format options considered:

| Option | For | Against |
| --- | --- | --- |
| JSON with a generated schema | Same as the engine's tools and catalog; editors validate it; easy for any host to write | No comments; verbose for long lists |
| JSONC (JSON with comments), the first choice | Comments for people; the engine already reads JSONC remote configs | Fewer editors validate it out of the box; Verilog snippets would be one-line strings |
| TOML, chosen | Comments; the same format as Logix IDE's project file (`logix-ide.toml`); multi-line literal strings (`'''…'''`) hold Verilog snippets as written; good parsers in Rust (`toml`), Python (`tomllib`, built in) and JavaScript | Deep nesting is awkward, which this file doesn't need |
| YAML | Short; block strings for snippets | Guesses types (`tx: 6` a number, `no` false, `08` octal in YAML 1.1); indentation changes meaning silently; Rust's `serde_yaml` is archived |
| A constraint-style text file (like `.pcf`, one line per net) | Very short; familiar to FPGA users | A new grammar to parse and document; hard to extend |

An example, for the iCESugar v1.5:

```toml
#:schema https://www.openlogix.org/schemas/logix-debug-v1.json
clock = "sys_clk"

# The board's UART, by its resource name in the catalog (TX 6, RX 4).
uplink = { kind = "uart", resource = "uart", baud = 115200 }

[[drive]]
name = "blink_force"
net = "main.blink"

[[watch]]
name = "counter"
net = "main.count"
format = "dec"

[[watch]]
name = "blink"
net = "main.blink"

[[watch]]
net = "main.u_ctrl.state"
```

- `#:schema`: an optional first line naming the schema, for editors
  (Taplo, the "Even Better TOML" extension), which then check and complete
  the file. It is a comment to TOML, so the build doesn't read it. The
  version lives in the address (`-v1`; a breaking change gets `-v2`), so
  there is no separate `version` field. The schema lives in this repo,
  under `schemas/`.
- Entries are written as blocks, `[[drive]]` and `[[watch]]`, one key per
  line, in the order they apply. TOML also accepts one-line entries
  (`watch = [{ net = "main.blink" }]`), but a one-line entry can't hold a
  multi-line Verilog snippet, so files and examples use blocks.
- `clock`: the one clock the debugger and its UART run on, a top-level port of
  the design; every watched and driven net must be on it. Its frequency comes
  from the board's clock resource when it is that pin, else from
  `clock_mhz` in the file.
- `uplink`: required, so every file says where the debugger talks to the
  host. `kind` is `uart`, the only kind in version 1. Then one of:
  - `resource`: the board's UART by its resource name in the catalog
    (`get_catalog` lists a board's resources), such as `uart` on the
    iCESugar v1.5, whose pins are TX 6 and RX 4. A name that isn't a UART
    of the board is an error that lists the board's UARTs.
  - `tx` and `rx`: FPGA pins, for a USB serial adapter on a PMOD or a board
    with no UART in the catalog.

  `baud` is optional and defaults to 115200. The debug map records the
  resource and the pins it resolved to.
- `net`, in each watch and drive entry: the net, of any width, by its
  path (`main.u_counter.count`, 23 bits, or `main.blink`, one); its width
  comes from the design. Until 2026-11-25 one-bit nets used `net` and
  wider ones `bus`; the two are now one key.
- A `net` can also be a list that combines one-bit nets and pins into one
  bus, for watch and drive alike: `net = ["pin.21", "pin.20", "pin.19",
  "pin.18"]` with `name = "switches_n"`. The first member is the
  most significant bit, as in a Verilog concatenation, so `pin.18` is bit 0
  here. Each member must be one bit, and a list needs at least two members
  and a `name`. A drive list's nets share one override enable, so they
  are taken over and released together, and pins can't be in a drive
  list. A net can be in only one drive entry.
- `drive` and `watch`, from the host's side: `drive` lists the nets the
  host can drive (AMD's VIO calls them probe\_out, Intel sources), and
  `watch` the nets it reads (probe\_in, probes). They were `output` and
  `input` in earlier drafts, which read backwards from the design's side.
  `name` is the host's name for each (defaults to the net's path); widths
  come from the design, not the file. Names are unique within `watch` and
  within `drive`; a watch and a drive entry share one only when they name
  the same net, as both do by default (since 2026-11-27; such a pair was
  refused, so a net both watched and driven needed a name of its own). Watching a net that is also driven
  reads what the design sees of it: the host's value while driven, the
  design's own while released (since 2026-11-25; it read the design's own
  value only, so a driven constant never changed in Watch).
- `format` of a watch or drive entry: how hosts show its value at first,
  `hex`, `dec` or `bin`, and for a drive entry, how a typed value is read
  (a value with its own `0x` or `0b` prefix means what it says). Optional:
  omitted means `dec` for one bit, so a bit shows as `1`, and `hex` for
  more. In the panel, each row's Format menu changes it; a choice made
  there is remembered by name in that browser and wins over the file's.
- A drive entry has no mode: the debugger always overrides the net,
  taking it over from the design and releasing it back (see the next
  section). Until 2026-11-25 a `mode` key chose between `override` and
  `source`; it is gone, and a file that still has it gets an error. How a
  value is applied over time is a separate question from this coupling:
  a write holds a steady value until release, and a one-bit entry can
  also pulse (Pulses, in the next section). Neither needs a key.

For the next phases, the file gains `trigger` (the signals each trigger
looks at), `count` (each counter's trigger and limit) and `capture`
(signals, depth, trigger, sample clock) alongside `watch` and `drive`, each
optional (R18). Version 1 reserves the three keys and refuses them with
"not supported yet"; any other unknown key is refused too, so no file
valid now can clash with them later. What a trigger looks for is not in
the file: the host sets it while the design runs (R16).

Decided: one debug file per build request; merging several files can come
later.

## Connecting the debugger to design nets

Decided: the first version takes nets from the debug file only (option A).
Marking in the design (B, C) comes later, merged into the same list. Reads
are easy either way; writes are the hard part, since the debugger must take
over a net that the design already drives.

| Option | How it looks | For | Against |
| --- | --- | --- | --- |
| A. Names in the file (version 1) | `net = "main.u_ctrl.state"` | No design edits at all (R3); one place to see what is debugged | Names break when code is renamed; synthesis renames and removes nets, so the build must keep them alive and say when it can't (R9) |
| B. Attributes in the design (later) | `(* logix_debug = "watch" *) reg [3:0] state;` | Travels with the code, survives renames; a normal build ignores unknown attributes | Edits the design; attributes alone don't say the host name or write mode unless their value grows a small syntax |
| C. Probe modules in the design (later) | `logix_probe #(.NAME("state")) p (.value(state));` and `logix_source #(.NAME("en")) s (.design(en_logic), .value(en));` | Explicit and simulatable: a testbench sees the same pass-through; a write is a clean mux the designer placed | A normal build needs the probe modules' pass-through file too; most design edits of the three |

B and C differ in kind: an attribute is a note that adds no hardware, so the
build must find the net and insert any override mux itself; a probe module is
real hardware in the design, so simulation sees it and the designer places the
override mux.

Writes:

- **Override**, the one way the debugger couples to a net: it can take over a net the design drives, through a
  mux at that net's driver, released by default. With option A the build cuts
  the net and inserts the mux itself, a netlist edit (Build integration).
  Every load of the net sees the driven value, the design's own logic
  included: overriding a register's output also feeds the register's next
  value, through its logic and its hold path, so after a release the
  register carries on from the driven value (seen on the Alhambra II
  example's running light, 2026-11-25).

  A spare control input, a net the design only reads, is a net set to a
  default and overridden: `wire white = 1'b0;`. Released, it is the
  default, and a normal build, which has no debugger, gets the same.

  Until 2026-11-25 there was also a `source` mode, for a net with no driver
  in the debug build, driven only by the debugger. A design had to leave
  the net undriven under `ifdef LOGIX_DEBUG` and give it a default
  elsewhere, and one that left it undriven in normal builds too had an
  undefined net there. Once the pre-pass kept constant nets (Implementation
  notes), overriding covered it safely, so `source` was removed, and with
  it the `mode` key. A driven net with no driver of its own is now
  refused.

- **Pulses** (added 2026-11-25): every one-bit drive entry can
  pulse, which flips the net's value for exactly one clock, whatever it is
  then: the host's 0 or 1 while driven, the design's own value while
  released. Afterwards the net is as it was. The debugger times it, since
  the UART can't time one clock: a PULSE command loads a mask into a
  register that is high for one clock. The net's load side is then
  `pulse ^ (en ? value : design)`, one XOR and one flip-flop per one-bit
  entry; wider entries and lists get neither, and `write_debug` refuses to
  pulse them. Watch samples far slower than a clock, so a pulse doesn't
  show there; its effects do. A pulse on a register's output also feeds
  that register's next value, as an override does, so pulses suit control
  inputs best (start, step, clear). In the Alhambra II example, a pulse on
  `enable_step`, held at 0, lets one step through.
  Tested on the Alhambra II on 2026-11-25, with that example's earlier
  `freeze` and `step_now` nets: with the light frozen, each
  pulse on `step_now` moved it exactly one place, released or driven to 0;
  a pulse on `freeze` held at 1 left it frozen and still driven at 1; and
  pulsing the 8-bit `leds` was refused.

### Naming rules (version 1)

- A name is a full instance path, dot-separated, starting with the top
  module's name, then the net: `main.u_core.u_fifo.count`. A net in the top
  module itself is `main.count`. Starting with the top makes a path read the
  same as in a testbench or a waveform, and the build checks it against
  `top_module`.
- Each instance is its own net: a module instantiated twice gives
  `main.u_lane0.err` and `main.u_lane1.err`, and the file lists each one it wants.
- Generate blocks are written as yosys names them: `main.gen_lane[2].u_lane.err`.
- A package pin is `pin.<pin>`, such as `pin.18`: the build reads the
  top-level port bit the constraint file places on that pin (`set_io s1_n
  18`, or a bus bit such as `sw[0]`). Pins are one bit, so they use `net`,
  and version 1 reads them but doesn't drive them. A pin the constraint file
  doesn't use is an error, and a top module named `pin` is refused, since
  its paths would be ambiguous. A pin wired to a dedicated pad, such as the
  iCE40 UltraPlus's RGB LED driver pins, can't reach the fabric and so can't
  be read.
- Whole nets and buses only: no bit selects (`bus[7:4]`) and no wildcards in version 1.
- A name that matches no net, or more than one, is a build error (R9).

Capture phase: the same selection marks capture signals (`"capture"` in the
file), so nothing here changes.

## Debugger hardware

A small register-map debugger with a UART command engine, in plain
Verilog-2005, with no CPU and no family primitives. The host sends short
frames; the debugger answers from registers that tap or drive the design's
nets.

```
            FPGA                                     board            host
 +---------------------------------------------+
 |  user design (instrumented copy)            |
 |    nets to read ---------+                  |
 |    nets to override <-+  |                  |
 |                       |  v                  |
 |  +--------------------------------------+   |
 |  | debug hub                            |   |
 |  |  snapshot regs | write regs + enables|   |
 |  |  build ID ROM  | [capture: later]    |   |
 |  +--------------------------------------+   |
 |            ^ register bus                   |
 |  +--------------------------------------+   |   UART   +--------+  USB  +-----------+
 |  | command engine + UART                |<--+--------->| bridge |<----->| soft-agent|
 |  +--------------------------------------+   |          +--------+       | MCP server|
 +---------------------------------------------+                           +-----------+
```

- **Debug hub:** one byte shift register, loaded in one cycle with the
  read nets, the write values and the enables on a READ (R5), or with the
  protocol version and build ID on an INFO (R8), and shifted out a byte at
  a time as the reply goes; payloads come in through it too. One register
  per write net, plus an override enable per write net, loaded together
  from it once a WRITE frame's CRC checks out.
- **Command engine + UART:** parses frames and answers them. At 12 MHz and
  115200 baud the divisor is 104 (0.16% off), well within UART tolerance.
  A 64-byte receive buffer, in block RAM, sits between the UART and the
  engine: the engine takes input only while it waits for a frame, so bytes
  that arrive while it executes or replies wait there. The host can
  therefore send several frames at once; it keeps at most 48 bytes on the
  way, and the buffer has no full check.
- **Built for size and speed** (2026-11-26): no variable shifts or byte
  multiplexers (the shift register replaces them), one CRC for both ways
  taking two clocks a byte, the byte compares the engine needs stored with
  each byte in the receive buffer's spare RAM bits, one-hot states,
  registered strobes, and timers whose sign bit is their tick. No path in
  it is more than a few LUTs long. See the cost to-do for the numbers.
- **Clock domains:** version 1 has one clock, shared by the UART and the
  debugger, and every watched and driven net must be on it. Nets from other
  domains come later, with synchronizers: two flip-flops for single bits, a
  handshake for buses (R11).

### Protocol (version 1)

Versions start again at 1 on 2026-11-26 (the user: no compatibility while in
early development); the protocol before it was called version 3.

The protocol works on 8-bit bytes. A space is a little-endian bit vector,
packed with no gaps and padded with zeros to whole bytes: the snapshot is
the read nets in map order, then the write values in map order, then the
override enables (one per driven entry); a WRITE payload is the write
values, then the enables. The map gives each net's bit offset and width in
its own part. Each part is at least one bit: a design with nothing driven
still has one write bit and one enable.

Frames from the host: `0xA5`, command, payload length (one byte), payload,
CRC-8. Replies: `0x5A`, the command, status, payload length, payload,
CRC-8. CRC-8 is polynomial 0x07, initial value 0, over every byte after the
sync byte. A frame with a bad CRC or an unknown command gets a status error,
not silence, so the host never waits for a timeout on a well-formed frame.
A frame that stops arriving is dropped after 256 to 512 bit times (25 to 50
bytes).

| Command | Code | Request payload | Reply payload |
| --- | --- | --- | --- |
| INFO | 0x01 | none | protocol version (1 byte), build ID (4 bytes, little-endian) |
| READ | 0x02 | none | the snapshot, taken in one clock: read nets, write values, enables |
| WRITE | 0x03 | the write values and the enables, all of them | none; all take effect in the same clock, once the CRC checks out |
| PULSE | 0x04 | a mask of the enables' width | none; each set bit's pulse is high for exactly one clock, all of them in the same clock. Only one-bit write nets use theirs |
| CAPTURE\_\* (next phase) | 0x10 and up | arm, trigger, status, read | capture data in blocks |

Status: 0 OK, 1 bad CRC, 2 unknown command, 3 bad length. A reply to an
unknown command has command 0x00. A READ reply is at most 255 bytes, so a
build refuses more than 2040 bits of read nets, write values and enables
together.

There are no offsets: the host reads everything in one READ and writes
everything in one WRITE, from its copy of the write values and enables,
which each READ refreshes. The INFO reply has no sizes, since the build ID
covers the map.

At 115200 baud the link carries about 11.5 KB/s: reads feel instant, but an
8 KB capture would take about 0.7 s, which is why faster rates are measured
before the capture phase.

## Build integration

Decided: a yosys pre-pass in soft-agent that writes an instrumented copy of
the design, then the engine's `build` on a generated top. It needs no engine
change, and keeps the normal build untouched (R3).

```
 1. pre-pass (yosys): read user sources -> hierarchy, proc, flatten
                      -> expose read nets, expose -cut override nets
                      -> write_verilog  _build/debug/design_debug.v
 2. generate:         _build/debug/debug_top.v  (user top + debugger + muxes)
                      _build/debug/debug.pcf    (user constraints + UART pins;
                                                  debug.lpf on ECP5,
                                                  debug.cst on Gowin,
                                                  debug.xdc on Xilinx)
                      _build/debug_map.json     (names, offsets, build ID)
 3. add:              the debugger's own Verilog, embedded in soft-agent
 4. engine build:     the engine's build tool on the files above
```

Steps 1 to 3 are new and live in soft-agent; step 4 is the engine's `build`
code, called with the generated files.

| Option | How | For | Against |
| --- | --- | --- | --- |
| I1. Wrapper only | Generate a top that instantiates the user's top and the debugger | Simplest; no netlist edits | Reaches only top-level ports, not internal nets |
| I2. Yosys pre-pass (chosen) | Run yosys to flatten and expose the nets, write Verilog, then build that with a generated top | Internal nets, reads and overrides; engine unchanged | Two yosys runs; errors point at generated files unless mapped back |
| I3. soft-agent's own build | Copy the engine's build steps with an extended synthesis script | One yosys run | Duplicates engine code that changes often; drifts like `main.rs` would |
| I4. Engine hook (later) | `build` accepts extra synthesis passes or a pre-synthesis step | Cleanest; one yosys run | An engine change: needs the user's go-ahead, and fits the move into the engine |

Tried on the blinky design (yosys 0.70, from the engine's package):
`flatten; expose w:count; expose -cut w:blink; write_verilog` gave
`output [22:0] count`, `output blink` and `input \blink.i`. So `-cut` gives
the design's value out and the loads' input in, and the generated top puts
the override mux between them.

- **When to expose:** right after `proc` and `flatten`, before optimization,
  so the nets still exist; a net missing then is error R9.
- **Errors in user terms:** the pre-pass runs on the user's own files first,
  so design errors keep their file and line.
- **Constraints:** the debug build writes its own copy of the user's
  constraint file with the debugger's TX and RX pins added. The user's file
  stays untouched.
- **Build ID:** a hash of the generated design, the debug map and the
  debugger's Verilog, in the debugger's ROM and in `debug_map.json`.
- **Capture phase:** the same pre-pass exposes capture signals; the debugger
  adds a RAM block, so `debug_top.v` grows but the flow stays as drawn.

## Host side: MCP tools

A session model like the engine's serial tools, with names from the build's
debug map. All are soft-agent extension tools in the engine's style
(self-contained requests, absolute paths, errors as data):

| Tool | Does | Notes |
| --- | --- | --- |
| `build` with `debug_file` | Builds with a debug file: inserts the debugger, then the usual synthesis, place and route, packing | `build`'s fields, plus `debug_file`; writes `debug_map.json` and reports the debugger's resources apart (`debug`) |
| `list_nets` | Lists the design's nets a debug file can name, for a host's net picker (Q15) | The design's fields, as for `build`; returns each net's path, width and kind |
| `open_debug` | Opens a session on a board's debugger, from a build folder and a serial port, and optionally the board's `device` (its ID from `list_devices`, as for `open_serial`) | Reads the debugger's build ID first and refuses a mismatch (R8); with `device`, finds the port again after an unplug, even under another name |
| `read_debug` | Reads named nets, in one snapshot | Values as numbers and as bit strings, with widths |
| `write_debug` | Writes a named net, releases an override back to the design, or pulses a one-bit net | Refuses names that aren't in drive, and pulses on nets wider than one bit |
| `close_debug` | Ends the session | |
| `open_debug_panel` | Opens an interactive panel for a session in the browser | Drive first, then Watch, which refreshes live and can be paused, then Plot, a live strip chart of the watched values; each watched row shows its value in a format chosen per row (hex, decimal or binary); each driven net has value and release controls (0, 1 and Pulse buttons for one bit; a pulse the board took flashes the row's state) |

- **The port is shared.** The debugger owns the UART while a session is open,
  so the engine's `open_serial` on the same port is refused in version 1. A
  console channel in the protocol, so text the design prints still reaches a
  Serial Monitor, can come later.
- **Programming:** a debug session pauses while `program_board` writes
  its board, in the same program (built 2026-11-28): its port is closed,
  and reads and writes are refused with a setup error saying the board is
  being programmed. After, the next read or write reopens the port and
  checks the build ID again, as after an unplug: the same build goes on
  with every override released (the board restarted), and another build
  is refused. Tested on the iCESugar v1.5.
- **Results:** in milestone 1 they show in Claude Code or any MCP client as
  structured tool results; request and reply, not a stream.
- **Triggers and counters:** the host sets each trigger's condition and
  clears counters, with a tool of its own or as part of `write_debug`; to
  be decided with their protocol commands. Counters are read with
  `read_debug`, in the same snapshot as the watched nets, so the panel's
  Watch table and Plot show them as they are.
- **Capture phase:** a tool of its own, such as `capture_debug`, arms a capture with a trigger, and its
  result is a VCD file in the build folder, so the engine's `open_waveform`
  shows it with no new viewer.

## Boards

What a board needs for the debugger, and where the Logix Catalog's boards
stand (audited 2026-11-26, catalog main at that date, 81 boards).

### What a board needs

None of the debugger's board data is required from the catalog. The catalog
only saves the user from typing it.

- **A clock** of known frequency for the debugger and its UART, the one
  `clock` of the debug file. The board's catalog clock (`type: "clock"`, with
  `pin` and `mhz`) gives the frequency when the clock port is on its pin;
  any other clock pin works with `clock_mhz` in the file.
  - **Not an internal oscillator** (decided 2026-11-26): the UART needs
    the clock within about 2%, and the iCE40 UltraPlus's 48 MHz `SB_HFOSC`
    is specified at ±10% (0 to 85 °C) and ±20% (−40 to 100 °C), in its
    datasheet's Table 4.11. The user dropped it: too inaccurate. `clock`
    stays a top-level input port on a pin.
- **A UART link to the host:** a TX and an RX pin, at a 3.3 V (or the
  bank's) I/O standard.
  - The board's own USB serial bridge, as a catalog `uart` resource (`tx`,
    `rx`, `usb_interface` when the bridge's chip has several interfaces,
    `io_standard` when the family needs one), so `uplink = { kind = "uart",
    resource = "uart" }` finds it.
  - Or any two free pins wired to a USB serial adapter (3.3 V), with
    `uplink = { kind = "uart", tx = "...", rx = "..." }`. Every board can use
    the debugger this way.
- **A baud rate the clock can make.** `baud` defaults to 115200. The build
  accepts any rate the clock divides with at least 8 clocks a bit and at
  most 2% error. 1,000,000 is the fastest tested (five boards, see Batches)
  and makes reads up to four times faster, but nothing needs it: a bridge
  that only does a slower rate works at that rate, only slower. A board's
  `ai_hints` should say so when its bridge is limited (the Tang Nano 9K's
  says 9600). Rates below 115200 are untested: at 9600 a full 128-byte
  read reply takes about 135 ms, within the host's 1 s reply timeout.

### Catalog boards

Whether a board's bridge reaches the FPGA comes from its USB IDs (an FTDI
FT2232H has a second channel, an FT232H has only one) and its makers'
documents; the rows marked as such are not checked against schematics or on
the boards. The catalog has no field for a bridge without a `uart` resource.

- **Catalog `uart`, tested with the debugger** (5): Alhambra II (FT2232H,
  12 MHz), Arty A7-35T (FT2232H, 100 MHz), iCESugar v1.5 (iCELink, 12 MHz),
  iCESugar Pro (iCELink, 25 MHz), Tang Nano 20K (BL616, 27 MHz). All at
  1,000,000 baud, the Alhambra II and both iCESugars at 115200 too.
- **Catalog `uart`, slow** (1): Tang Nano 9K (BL702): only 9600 baud on
  macOS. It should work at 9600, not tested.
- **A USB serial bridge to the FPGA, no catalog `uart` yet** (likely, from
  makers' documents): iCEBreaker, iCEstick, iCE40-HX8K breakout, Go Board,
  IceZUM Alhambra, EDU-CIAA-FPGA, Alchitry Cu, Au and Au+, Arty A7-100T,
  Arty S7-25 and S7-50, Basys 3, Cmod A7-35T, Nexys A7-50T and A7-100T,
  ULX3S (all four, FT231X), Icepi Zero (FT231X), iCESugar nano (iCELink),
  Tang Nano 1K and 4K (BL702, maybe as slow as the 9K's), Tang Primer 20K
  Dock (BL616), ECP5 Versa (FT2232H), pico-ice (its RP2040's USB serial).
  Adding their `uart` resources is catalog work, done as any catalog change.
- **Serial only to a processor, not the FPGA fabric** (5): PYNQ-Z1, PYNQ-Z2,
  Zybo Z7-10 and Z7-20, ZedBoard. Their USB UART goes to the Zynq's
  processor side; the debugger needs an adapter on a PMOD.
- **No USB serial bridge** (likely): native-USB boards programmed by DFU
  (Fomu, OrangeCrab 25F and 85F, iCEBreaker bitsy v0 and v1, ButterStick),
  single-channel FT232H boards (UPduino 3.0 and 3.1, VSDSquadron FM, FPGA101,
  ARICE1, ODT IcyBlue Feather), and Colorlight 5A-75B and 5A-75E (no USB).
  They need an adapter.
- **Unknown, check the board's schematic** (the rest, 26): ALINX AX7035,
  Azukar2, BRS-100-GW1NR9, Colorlight i5, i9 and i9+ (depends on the
  extension board), ECP5 Evaluation Board, ECP5 Mini 12 and 25, EPM11,
  FleaFPGA Ohm, iCE40UP5K breakout, iCEnode, MicroPhase A7-Lite, Mimas A7,
  Mimas ECP5 Mini, OK-iCE40Pro, QMTech XC7A100T, Soan Papdi and Soan Papdi
  Mini, Tang Nano (the first), Tang Primer 20K Lite (the core board alone),
  ThetaMachines ETH4K, ZXTRES, ZXTRES+ and ZXTRES++.

Clocks: 79 of the 81 boards have a catalog clock, from 12 to 125 MHz, and
every one of them can make 115200 baud within the build's 2%. The two with
none, the pico-ice and the ODT IcyBlue Feather, have no clock pin in the
catalog; they need a clock on a pin, with `clock_mhz` (not their internal
oscillator, above).

## Prior art

What each existing tool suggests for this design. From memory, not checked
against current docs.

| Tool | How it works | Take from it |
| --- | --- | --- |
| AMD Xilinx ILA and VIO | Nets marked with a `mark_debug` attribute or picked in the tool; cores inserted into the synthesized netlist; JTAG uplink; a probes file ties the bitstream to names | Attribute marking (option B), a separate map file tied to the bitstream (R7, R8), VIO as the model for read/write |
| Intel SignalTap and In-System Sources and Probes | Nets picked in a settings file; capture with triggers into RAM; sources and probes for read/write; JTAG | Read/write and capture as two cores sharing one link; the file names the nets (option A) |
| Lattice Reveal | Inserter and analyzer tools; signals picked from the design; JTAG | The two-step model: insert, then analyze |
| LiteX LiteScope and UARTBone | A bus bridge over UART (UARTBone) gives the host register access; LiteScope captures into RAM behind it; `litex_server` shares the link with several clients | A UART bridge to a small register map, capture as a later block on the same bus, and one host process sharing the link |

Difference here: open source tools only (yosys, nextpnr), and the uplink is
the board's own USB UART, not JTAG, so it works on boards with no JTAG
adapter.

## In Logix Engine (done 2026-11-27)

The feature moved (Q12): the file format, debug map, protocol and tool names
moved unchanged; `debug_build` became the `debug_file` field of `build`, and
`list_nets` was added for hosts' net pickers (Q15). The schema's address is
`https://www.openlogix.org/schemas/logix-debug-v1.json`, published by the
website from this repo's `schemas/` (Q9). What's left:

- **The debugger's Verilog in logix-core**, like the engine's other data,
  instead of embedded in the engine binary.
- **The panel as a Logix Viewer app** (Q13), Logix Debugger, beside Logix
  Waveforms and Logix Diagrams; until then `open_debug_panel` serves its page
  from the engine on 127.0.0.1.
- **Logix IDE:** a Debug command, `debug.toml` in the project, a net picker
  from `list_nets`.

### What the move planned

The feature moves (Q12, 2026-11-26): the file format, debug map, protocol
and tool names move unchanged; only where the code lives changes. To keep that easy:

- **Follow the engine's tool conventions now:** self-contained requests,
  absolute paths, errors as `ToolError` data with categories, schemas
  generated from Rust types, display paths honored.
- **Keep the debugger's Verilog generic:** plain Verilog-2005 with no family
  primitives, so it builds on every toolchain the engine supports (R13).
- **Ship the debugger's Verilog like other engine data:** in the engine it
  would live in a package, not in the binary; soft-agent embeds it for now.
- **Name it neutrally:** `debug_build` could become a `debug_file` field of
  the engine's `build`, and the other tools keep their names. Avoid
  `soft_agent` in any name a host sees.
- **The panel** (Q13): a Logix Viewer app of its own, a third beside
  Logix Waveforms and Logix Diagrams, with its own name (such as Logix
  Debugger) and dock icon, one window per session, titled as the others
  are. A host with a window of its own, such as Logix IDE, shows the
  panel's page itself instead (`open` false), as it does the waveform
  viewer and the diagram. Unlike those pages, the panel is live: it reads
  and drives the board while it is open, so it needs a live channel to the
  engine's session, which the page contract (`window.logixHost`, version
  1) doesn't have. Designing that channel, for Logix Viewer and for hosts,
  is part of the port: today `open_debug_panel` serves the page from a local
  web server instead. The app's icon and the Logix Viewer side are
  logix-core's.
- **Editing the debug file in a host** (2026-11-28): the file stays the
  source, and a host's editor, such as Logix IDE's, edits it: tables for
  Watch, Drive, Triggers, Counters and Capture, an add button with a
  searchable list of the design's nets, right-click on a signal in the
  code to add it, a mark beside the nets in the file, names that match no
  net flagged as typed, and the debugger's estimated cost as the file
  changes. The file stays editable by hand and by AI agents, and the
  editor keeps its comments where it can. What every host needs comes from
  the engine, not each host:
  - **The design's nets:** a tool (such as `list_nets`) that runs the
    debug build's first step, yosys reading and elaborating the design,
    and returns every net with its instance path, width and kind (port,
    register, wire), named as the debug file names them (generate names
    as yosys prints them). About a second for these designs, with no
    synthesis and no board. A design that doesn't parse gives an error;
    the editor keeps its last list and says it is out of date.
  - **Checking a debug file** against those nets, and **the cost
    estimate**, as `debug_build`'s report gives it now.

  Settings that cost no logic, such as trigger conditions, arming and
  clearing counters, stay in the panel, not the file.
- **What the engine would need:** a hook for extra synthesis passes in
  `build::synth_script`, or a pre-synthesis step; today soft-agent works
  around its absence (Build integration). Proposing that hook is an engine
  change, so it needs the user's go-ahead first.

### To do before porting (done)

Open as of 2026-11-25, to be gone over one at a time:

- [x] **Every family the engine supports** (done 2026-11-26). `debug_build`
  refused families other than iCE40; the pre-pass and the size estimate
  now use each family's own cells and synth command, and the UART pins
  each constraint format (see the build section). The debugger's Verilog
  is unchanged, generic. A board needs no catalog UART: the uplink's `tx`
  and `rx` can be any pins, wired to a USB serial adapter. Tested on one
  board of each toolchain:
  - [x] **IceStorm (iCE40).** Tested on the Alhambra II and the iCESugar
    v1.5.
  - [x] **Trellis (ECP5).** Tested on the iCESugar Pro
    (`examples/icesugar-pro`, 2026-11-25): reads, holds, pulses, drives
    and releases as on iCE40, at 115200 and 1,000,000 baud.
  - [x] **Apicula (Gowin GW1N, GW2A, GW5A).** Tested on the Tang Nano 20K
    (`examples/tang-nano-20k`, GW2AR-18, 2026-11-26): reads, holds,
    pulses, drives and releases as on iCE40, at 1,000,000 baud. The Tang
    Nano 9K builds too, but its BL702's serial port worked only at 9600
    baud, so it wasn't tested.
  - [x] **openXC7 (Artix-7, Spartan-7, Zynq-7000).** Tested on the Arty
    A7-35T (`examples/arty-a7-35t`, 2026-11-26): reads, holds, pulses,
    drives and releases as on iCE40, at 1,000,000 baud, programmed to
    flash. Right after programming, the first `open_debug` got no reply
    while the FPGA loaded from flash; a second one a little later
    worked.
- [x] **Board data in the catalog** (done 2026-11-26). The catalog's clock
  and `uart` are optional: any clock pin with `clock_mhz`, and any two pins
  with a USB serial adapter, work too, at any baud the clock can make, not
  only 1,000,000. What a board needs and where the 81 catalog boards stand
  are under Boards: 6 have a `uart`, about 27 more likely have a bridge
  with no `uart` entry yet, and 79 have a clock.
- [x] **Internal oscillators** (dropped 2026-11-26). The iCE40
  UltraPlus's `SB_HFOSC` is ±10% at best, against the UART's 2% (Boards).
- [x] **Cost** (done 2026-11-26). The debugger was rebuilt for size and
  speed (Debugger hardware), with protocol 1. Measured as the difference
  between a normal build and a debug build of each example (the Alhambra
  II's design, also built for the iCESugar v1.5 with its LEDs on PMOD 2;
  31 bits watched, 9 driven), placed and routed by the engine's build:

  | Board (FPGA) | Normal build | Debug build, before | Debug build, now |
  | --- | --- | --- | --- |
  | iCESugar v1.5 (iCE40 UP5K) | 43 cells, 73.7 MHz | +882 cells, 24.6 MHz | +330 cells, 69.9 MHz |
  | Alhambra II (iCE40 HX4K) | 43 cells, 188.4 MHz | +903 cells, 69.5 MHz | +330 cells, 173 to 185 MHz |
  | iCESugar Pro (ECP5) | 39 LUTs, 223.7 MHz | +915 LUTs, 85.1 MHz | +469 LUTs, 206.5 MHz |
  | Tang Nano 20K (Gowin) | 71 LUTs and FFs, 277.6 MHz | +1,210, 87.2 MHz | +664, 238.4 MHz |
  | Arty A7-35T (Xilinx) | 95 LUTs and FFs, 274.7 MHz | +1,091, 142.6 MHz | +608, 292.9 MHz |

  On iCE40 the debugger is now about 265 logic cells plus one per watched
  or driven bit, and one RAM block. Alone (31 read bits, 9 write bits, 2
  enables), placed with three nextpnr seeds, it reaches 80 to 85 MHz on
  the UP5K and 172 to 196 MHz on the HX8K, as fast as the example design
  itself: on the iCESugar v1.5 the debug build's slowest path is the
  design's own counter. What is left of the drop comes from placement and
  from the watched nets' extra loads, and varies with the seed. The
  Alhambra II's maximum clock was 173 to 185 MHz over the builds made while
  measuring. On ECP5 the receive buffer is in LUT RAM, so it costs more
  LUTs there. `debug_build`'s report still gives the debugger's own LUTs
  and flip-flops from synthesizing it alone (258 and 208 on iCE40), which
  pack into fewer cells than their sum. Tested on the iCESugar v1.5 at
  1,000,000 baud: reads, writes, releases, holds and one-clock pulses as
  before, 300 reads with no errors. The other families' examples were
  built, not tested on their boards.
- [x] **Where the panel goes** (decided 2026-11-26, Q13). Into the engine
  with the rest, as a third Logix Viewer app beside Logix Waveforms and
  Logix Diagrams, with its own name and dock icon (see In Logix Engine; formerly Path into Logix
  Engine later).
- [x] **Windows and Linux** (done 2026-11-28). Test sessions with the engine's serial port
  and driver handling on Windows and Linux.
  - [x] **Windows.** Tested on 2026-11-27 on Windows 11 Pro (x64), Python
    3.15, the iCESugar v1.5 and pre-release `pre-2026-11-27`: the
    automated check passed (1.06 ms a read, 2.07 ms a write), and so did
    the panel by hand. After a replug the panel resumed by itself in about
    2 s. With the port in use, a second session stopped at once with
    "access is denied". It needed the engine to match serial numbers in
    any case (Windows reports them in uppercase) and the scripts to take
    `--port` when `list_devices` gives no port.
  - [x] **Linux.** Tested on 2026-11-27 with the iCESugar v1.5 and
    pre-release `pre-2026-11-27`: everything passed but the second
    session. A second program's `program_board` reprogrammed the board
    under an open session (Linux, like macOS, locks the port only while it
    is open, and the session had closed it when the board restarted), and
    the panel then resumed on the other program's build. Sessions now hold
    a lock across programs (Sessions in other programs, below).
  - [x] **Retest** (2026-11-28, `pre-2026-11-27` and `pre-2026-11-28`),
    on Linux, Windows and macOS: a second program's `debug_check.py` is
    refused at once with the session's process number, and the panel and
    the board are unaffected; the plot leaves a gap while the board is
    unplugged; closing the session's window without closing the session
    frees the board. A pause no longer draws the last value on for up to
    5 s after it (`pre-2026-11-28`, checked on macOS).
- [ ] **Freeze the formats.** Once in the engine, `debug.toml`, the debug
  map, the protocol (version 1) and the tool names are public. Review them
  once more before porting. Reviewed 2026-11-28: the session tools now go
  by the engine's verb_noun names (`open_debug`, `read_debug`,
  `write_debug`, `close_debug`, `open_debug_panel`; they were
  `debug_open` and so on), and `open_debug` takes an optional `device`,
  as `open_serial` does. `debug_build` stays until the port, where it
  becomes `build` with a `debug_file`.
- [x] **Schema address** (done 2026-11-27). `debug.toml`'s `#:schema` line and
  `schemas/logix-debug-v1.json` point at this repo, under a personal
  account. Before porting, move the schema to an Open Logix address, and change it in `src/debug/config.rs`
  (the schema's `$id`), the schema file and the examples' `debug.toml`. Planned
  (2026-11-28): `https://www.openlogix.org/schemas/logix-debug-v1.json`,
  published by the website from the engine repo's copy, so the address
  doesn't depend on a repo's layout; confirm at port time.

### To do after porting

- [ ] **Contributor docs.** In Logix Catalog's contributor docs, say what a
  board needs for the debugger (Boards, above): a clock, and a `uart`
  resource for its USB serial bridge, with `usb_interface`, `io_standard`
  and an `ai_hints` line when the bridge is limited to a slower baud.
  Update the debugger's requirements, if any, in the docs that explain
  custom boards too. Waits until the debugger is in the engine.

## Decisions

| # | Question | Decided |
| --- | --- | --- |
| Q1 | File format: JSON, JSONC, TOML or a `.pcf`-like text file? | TOML with a generated schema, as Logix IDE's project file; it was JSONC until 2026-11-25 |
| Q2 | Where nets are chosen: the file, the design, or both? | The debug file only for now; marking in the design later |
| Q3 | How the debugger gets in: a yosys pre-pass, soft-agent's own build, or an engine hook? | Yosys pre-pass now (no engine change); an engine hook when moving it |
| Q4 | Writes: `source` only first, or `override` of driven nets too? | Both, if yosys `expose -cut` works as expected; else `source` first (`source`, as Intel's In-System Sources, was called `register` until 2026-11-25). `source` was removed on 2026-11-25, and `mode` with it: overriding a net with a default does the same, safely |
| Q5 | Debugger clock: always the board clock, or any net? | One clock for the UART and the debugger, the board's clock resource by default; every watched and driven net must be on it |
| Q6 | The UART when the design uses it too? | Refuse in version 1; console channel later |
| Q7 | Baud rate? | 115200 now; measure the iCESugar's bridge at 1 Mbaud and up later. Measured 2026-11-25: the Alhambra II, the iCESugar v1.5 and the iCESugar Pro reliable at 1,000,000 (see Batches); the Tang Nano 20K and the Arty A7-35T too, 2026-11-26 |
| Q8 | Several debug files per project, merged? | One file per build request |
| Q9 | Where the schema lives? | In Logix Engine's `schemas/`, published at www.openlogix.org/schemas/ by the website (it lived in the prototype's repo until the move) |
| Q10 | Naming rules? | Full instance paths starting with the top module (`main.u_ctrl.state`), or `pin.<pin>` for package pins, whole nets, generate names as yosys prints them, no wildcards (see Naming rules) |
| Q11 | Versions and compatibility before release? | None kept while in early development (the user, 2026-11-26): the protocol started again at version 1 then, and a change may break older builds, which are refused and built again |
| Q12 | Port the debugger to Logix Engine? | Yes (the user, 2026-11-26): the build, the session tools, the formats and the panel move into the engine |
| Q13 | Where the panel goes? | Into the engine, as a third Logix Viewer app beside Logix Waveforms and Logix Diagrams, with its own dock icon; hosts such as Logix IDE show its page themselves (the user, 2026-11-26) |
| Q14 | Triggers, counters and capture: in which order, and do they cost a build that doesn't use them? | Triggers and counters first, then capture on the same triggers; each opt-in, with no logic when the file leaves it out; counters saturate and never wrap (R16 to R18; the user, 2026-11-28) |
| Q15 | How a user edits the debug file in an IDE? | With a GUI over the same file, which stays the source; the engine lists a design's nets for it (In Logix Engine; the user, 2026-11-28) |

## First milestone

On the iCESugar v1.5 with the blinky design, its counter moved into a
submodule (`u_counter`), on 2026-11-24:

- [x] Read three nets (`main.u_counter.count`, `main.u_counter.tick`,
  `main.blink`) named in
  a `debug.jsonc` (now `debug.toml`). Two reads 0.26 s apart saw `count` advance 3.12 million
  cycles, as a 12 MHz clock does.
- [x] Write one net in `override` mode, then release it: the writes and the
  release succeed. Seeing the LED follow the host still needs a person
  watching the board.
- [x] The debug map and build ID check, with a stale-map refusal shown: a
  second debug build's folder was refused while the board ran the first.
- [x] Resource use of the debugger reported apart from the design's.
- [x] A normal `build` of the same sources still works, with no debugger in
  it.

Measured:

| Build | Logic cells (of 5280) | Max clock |
| --- | --- | --- |
| Blinky, normal build | 36 | 62.9 MHz |
| Blinky with the debugger (25 read bits, 1 override) | 706 | 31.5 MHz |
| The debugger alone (yosys, before placement) | 461 LUTs, 246 flip-flops | |

The debugger is above the 300-cell target and halves the blinky's maximum
clock. Both come mostly from its byte-wide muxes over the read and write
spaces (variable shifts) and the reply byte mux. Next steps for size and
speed: address the spaces a byte at a time through a shift register instead
of a variable shift, and register the reply byte. Done on 2026-11-26, with more:
see the cost to-do under To do before porting.

## Implementation notes (version 1)

- **Where it lives:** `src/debug/` in Logix Engine: `config.rs` (the debug
  file), `protocol.rs` (frames and the debug map), `build.rs`
  (the debug build), `session.rs` (the session tools), and `hdl/` (the
  debugger's Verilog and its testbench).
- **Defines:** the pre-pass reads the sources with `SYNTHESIZE`, as the
  engine's build does, and `LOGIX_DEBUG`, which only debug builds define.
- **Driven nets must be used and driven:** after the cut, a driven net's
  loads must still read it (a cell or a top-level port), or driving it
  would change nothing; and the net must have a driver of its own (a cell,
  a constant or an input), or released it would have no value. The build
  refuses either.
- **No optimization before the cut:** the pre-pass runs `proc -noopt` and
  no `opt_clean` until after the cut. `proc`'s own `opt_expr` and
  `opt_clean` put a constant net's value into its loads
  (`wire white = 1'b0;`), leaving the net nothing to drive; synthesis
  optimizes later.
- **Buffers on driven nets:** before the cut, `insbuf` on the driven nets
  turns a plain connection, to a constant or another net, into a buffer
  cell. `expose -cut` needs a driver cell to cut at: without one it moved
  a constant net's loads onto the constant, and connected an alias
  (`wire red_on = ~sw_n[0];`) to its own cut side, a combinational loop.
  The build also refuses any cut net whose driver and loads still share
  bits.
- **Families:** iCE40 and ECP5 (2026-11-25), Gowin and Xilinx 7-series
  (2026-11-26): every toolchain the engine has. The pre-pass loads the
  family's cell library from yosys (`+/ice40/cells_sim.v`,
  `+/ecp5/cells_sim.v`, `+/gowin/cells_sim.v`, `+/xilinx/cells_sim.v`) as
  black boxes, so
  `expose -cut` knows which ports of primitives such as `SB_RGBA_DRV` are
  inputs. Without it, a cut net kept feeding primitives from the design's
  side. The UART pins go into the family's constraint format (`set_io` in
  `.pcf`; `LOCATE COMP` in `.lpf`, `IO_LOC` in `.cst` and
  `set_property PACKAGE_PIN` in `.xdc`, with the catalog UART's I/O
  standard), and the size estimate runs the family's synth command, as the
  engine does (`synth_ice40`, `synth_ecp5`,
  `synth_gowin -family <yosys family>`, `synth_xilinx -arch xc7`), then
  `flatten`, since `synth_xilinx` keeps the hierarchy. On
  ECP5 it is reported against the FPGA's LUTs and flip-flops, which nextpnr
  counts apart (`TRELLIS_COMB`, `TRELLIS_FF`), and a carry cell (`CCU2C`)
  counts as two LUTs. On Gowin it is reported against `LUT4` and `DFF`,
  with each `LUT1`-`LUT4` and `ALU` cell one LUT and each `DFF*` cell one
  flip-flop. On Xilinx it is reported against `SLICE_LUTX` and
  `SLICE_FFX`, with each `LUT1`-`LUT6`, `INV` and `SRL*` cell one LUT, a
  `RAM64M` or `RAM32M` four, and each `FD*` cell one flip-flop.
- **Generated files:** `<build folder>/debug_src/` holds `design_debug.v`,
  `debug_top.v` (top module `logix_debug_top`, UART ports
  `logix_debug_tx`/`logix_debug_rx`), `debug.pcf` (`debug.lpf` on ECP5,
  `debug.cst` on Gowin, `debug.xdc` on Xilinx)
  and `logix_debugger.v`.
  The engine's `build` builds them into the build folder, next to
  `debug_map.json`.
- **Sessions:** `open_debug` checks the build ID, then releases every
  override and clears the write registers. `close_debug` releases them
  too. Every READ reply carries the write values and enables as the
  debugger holds them: `write_debug` reports the state it reads back after
  writing, and the panel reads the drive state back from the board at
  every refresh, so it shows what the debugger really drives. When the
  board doesn't answer, the panel shows the last state written, marked
  "unconfirmed". Tested on the iCESugar: reprogramming the board under an
  open session restarted it, and the panel's next read showed the override
  released. A build made before protocol 1 (2026-11-26) is refused: build
  it again.
  Every `read_debug` and `write_debug` asks the debugger for its build ID
  first (one INFO frame, about a millisecond), and refuses when the board
  was programmed with another build after `open_debug`, since the session's
  map would no longer fit.
- **Reconnecting:** a link error (no reply, or the port gone, such as with
  the board unplugged) closes the session's port, and the next read or
  write opens it again, so a session, and the panel polling it, resume by
  themselves when the board is plugged back in. A board plugged back in has
  restarted with nothing overridden, so after its build ID checks out, the
  session releases every override and clears its copy of the written
  values to match. Tested on the iCESugar v1.5 on 2026-11-25: the panel resumed by
  itself after the board was unplugged and plugged back in.
- **Sessions in other programs:** a session holds a lock file for its port
  (`logix-debug-sessions/<port>.lock` in the system's temporary folder,
  with its process ID in `<port>.pid` beside it, since on Windows other
  processes can't read a locked file) from `open_debug` until it closes or its program
  ends, also while the port itself is closed for a reconnect. Another
  soft-agent program then refuses `open_debug` on that port, and
  `program_board` for the board with that port, with a setup error naming
  the process, before it programs anything: programming would put another
  design, perhaps with another debug configuration, under the session.
  `program_board` in the session's own program goes ahead, pausing the
  session (Host side, Programming). The plain engine and
  other programs don't see the lock; in the engine, `program_board` would
  check it itself.
  `read_debug` reads watched nets only; to read a driven net, list it in `watch`
  as well.
- **Batches:** the host sends each operation's frames in one write, up to
  48 bytes (the receive buffer holds 64), and then reads the replies in
  order, so an operation pays the USB serial chip's round trip once: a
  `read_debug` is INFO and READ together; a `write_debug` is an INFO first
  (nothing is written to another build), then a WRITE (or PULSE) and a
  READ together; a panel refresh is two batches. A
  refused frame doesn't end a batch early: its other replies are still read,
  so none is left on the line. Measured 2026-11-25 (protocol 3); with
  protocol 1, on 2026-11-26, the iCESugar v1.5 at 1,000,000 baud measured
  1.03 ms a `read_debug` (970/s) and 2.0 ms a `write_debug`, the same:

  | Board (USB serial chip) | `read_debug` | `write_debug` | Panel refresh |
  | --- | --- | --- | --- |
  | Alhambra II (FTDI, 16 ms latency timer) | 16 ms, 62/s (48 ms before batches) | 32 ms (64 ms) | 34 ms (about 100 ms) |
  | iCESugar v1.5 (its DAPLink's USB serial) | 4 ms, 249/s | 6 ms | 9 ms |
  | Alhambra II at 1,000,000 baud | 16 ms, 63/s | 32 ms | 34 ms |
  | iCESugar v1.5 at 1,000,000 baud | 1.0 ms, about 1,000/s | 2 ms | 4 ms |
  | iCESugar Pro (ECP5, its iCELink) at 1,000,000 baud | 1.0 ms, about 1,000/s | | |
  | Tang Nano 20K (Gowin, its BL616) at 1,000,000 baud | 6 ms, 167/s | | |
  | Arty A7-35T (Xilinx, FTDI) at 1,000,000 baud | 16 ms, 63/s | | |

  At 115200 baud a read's bytes take about 4 ms on the wire. That is
  nearly all of the iCESugar's time, so 1 Mbaud makes it four times
  faster; on the Alhambra the FTDI timer dominates, so it doesn't change.
  Both bridges were reliable at 1 Mbaud: 300 reads on the Alhambra and
  3,300 on the iCESugar, with no errors and no implausible values; the
  iCESugar Pro too, with 500 reads and no errors (25 clocks a bit from its
  25 MHz clock), and the Tang Nano 20K (2026-11-26), with 500 reads and no
  errors (27 clocks a bit from its 27 MHz clock); its BL616's USB serial
  port, not the wire, sets its pace. The Arty A7-35T too (2026-11-26),
  with 500 reads and no errors (100 clocks a bit from its 100 MHz clock);
  its FTDI chip's latency timer sets its pace, as on the Alhambra. With a
  12 MHz clock the usable fast rates are 1,000,000 (12 clocks a bit,
  exact), 921,600 and 1,500,000 (8 clocks a bit, the fastest the build
  accepts).

  The FTDI chip holds received bytes up to its latency timer, 16 ms by
  default; lowering it would speed the Alhambra up further, but macOS gives
  no simple way to.
- **Build ID:** a hash of the generated design, the debug map and the
  debugger's own Verilog, so a board with an older debugger is refused.
- **Limits:** a READ reply is at most 255 bytes, so a build takes at most
  2040 bits of watched nets, driven values and enables together. A frame
  that stops arriving is dropped after 25 to 50 byte times. The port isn't
  locked against the engine's serial tools, so don't open both on one port.
- **Progress:** `debug_build` reports no progress while it runs yet.
- **The plot:** the panel's Plot section draws every read's watched values
  over the last 10 seconds to 5 minutes: one bit as a high/low trace, more
  as a stepped line scaled to the values in view, with a hover cursor for a
  moment's time and values. A value holds until the next read, a failed
  read, a pause, or the time the status stops being Live (three refresh
  periods, at least 5 s), so where reads failed, the board was unplugged or the panel
  was paused, the plot is empty, as before the first read. It samples at
  the refresh rate (up to 50 a second), so it is a strip chart, not a logic analyzer: changes faster
  than the refresh don't show, and fast counters look like sawtooths. The
  capture phase is for those.
- **The panel:** `src/debug/panel.rs` serves `panel.html` from the
  soft-agent process on 127.0.0.1, on a port the system picks, while
  soft-agent runs. Every request needs the server's random token, which is
  in the address `open_debug_panel` returns, and a `127.0.0.1` or `localhost`
  Host header, so other web pages can't reach the board through it. Its API
  is `GET /api/sessions`, `GET /api/state?session=` (the session's map, the
  host's drive values and a fresh snapshot) and `POST /api/write` (the
  `write_debug` input). It polls at slightly irregular periods (about 4 per
  second, 1 per second, or every 5 s), since a round period can match a
  design's own and show the same phase at every read.
