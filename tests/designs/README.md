# Test designs

Small designs for the engine's tests and for trying the tools by hand. They aren't examples: the engine has no examples (see docs/requirements.md).

| Folder | What it is |
| --- | --- |
| `blinky` | An LED blinking from a counter, for the Alhambra II (`main.v`, `main.pcf`) |
| `blinky-ecp5` | The same blinky for the iCESugar-Pro, an ECP5 board (`main.v`, `main.lpf`) |
| `blinky-gowin` | The same blinky for the Sipeed Tang Nano 9K, a Gowin board (`main.v`, `main.cst`) |
| `blinky-xilinx` | The same blinky for the Digilent Arty A7-35T, a Xilinx board (`main.v`, `main.xdc`) |
| `counter` | A 4-bit counter (`counter.v`) with a passing testbench (`counter_tb.v`) and a failing one (`counter_fail_tb.v`) |
| `syntax-error` | A module with a syntax error on line 3 (`main.v`) |
| `pages` | A Morse code beacon (`morse.v`) blinking HELLO FPGA, and a testbench (`morse_tb.v`) that reads it back: the waveform pages of the browser tests (`tests/pages/`), with a clock, a bus and irregular 1-bit signals |

Tests copy a design into a temporary folder before using it, so these files are never changed.
