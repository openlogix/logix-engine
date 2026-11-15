//! Short descriptions of the resource names nextpnr reports, per toolchain
//! (docs/tool-api.md, build tool, `resources`), so every frontend explains
//! them the same way. The names stay the toolchain's own: they are what
//! people search for and quote. A name not listed here has no description.

use crate::catalog::Toolchain;

/// The description of a resource name, such as "Logic cell: a 4-input LUT,
/// a flip-flop and carry logic" for ICESTORM_LC, or None when unknown.
pub fn describe(arch: Toolchain, name: &str) -> Option<&'static str> {
    let table: &[(&str, &str)] = match arch {
        Toolchain::Ice40 => ICE40,
        Toolchain::Ecp5 => ECP5,
        Toolchain::Gowin => GOWIN,
        Toolchain::Openxc7 => OPENXC7,
    };
    table.iter().find(|(n, _)| *n == name).map(|(_, d)| *d)
}

const ICE40: &[(&str, &str)] = &[
    (
        "ICESTORM_LC",
        "Logic cell: a 4-input LUT, a flip-flop and carry logic",
    ),
    ("ICESTORM_RAM", "Block RAM, 4 Kbit"),
    ("ICESTORM_SPRAM", "Single-port RAM, 256 Kbit"),
    (
        "ICESTORM_DSP",
        "DSP block: 16x16 multiplier and accumulator",
    ),
    ("ICESTORM_PLL", "PLL (clock generator)"),
    ("ICESTORM_HFOSC", "Internal oscillator, 48 MHz"),
    ("ICESTORM_LFOSC", "Internal oscillator, 10 kHz"),
    ("SB_IO", "I/O pin"),
    (
        "SB_GB",
        "Global buffer, for clocks and other high-fanout signals",
    ),
    ("SB_GB_IO", "I/O pin with a global buffer"),
    (
        "SB_WARMBOOT",
        "Warm boot: loads another bitstream from flash",
    ),
    ("SB_I2C", "I2C controller"),
    ("SB_SPI", "SPI controller"),
    ("SB_LEDDA_IP", "LED PWM controller"),
    ("SB_RGBA_DRV", "RGB LED driver"),
    ("SB_FILTER_50NS", "Glitch filter, 50 ns, for I2C"),
    ("IO_I3C", "I/O pin with switchable pull-ups, for I3C"),
];

const ECP5: &[(&str, &str)] = &[
    ("TRELLIS_COMB", "LUT: a 4-input lookup table"),
    ("TRELLIS_FF", "Flip-flop"),
    ("TRELLIS_RAMW", "Distributed RAM write port"),
    ("TRELLIS_IO", "I/O pin"),
    ("TRELLIS_ECLKBUF", "Edge clock buffer"),
    ("DP16KD", "Block RAM, 18 Kbit"),
    ("MULT18X18D", "Multiplier, 18x18"),
    ("ALU54B", "ALU, 54-bit, for multiplier results"),
    ("EHXPLLL", "PLL (clock generator)"),
    ("DCCA", "Global clock buffer"),
    ("DCSC", "Glitch-free clock selector"),
    ("CLKDIVF", "Clock divider"),
    ("ECLKSYNCB", "Edge clock synchronizer"),
    ("ECLKBRIDGECS", "Edge clock bridge"),
    ("DCUA", "SERDES: high-speed transceivers"),
    ("EXTREFB", "SERDES reference clock input"),
    ("PCSCLKDIV", "SERDES clock divider"),
    ("DDRDLL", "DDR memory delay-locked loop"),
    ("DLLDELD", "Delay line, for DDR"),
    ("DQSBUFM", "DDR strobe (DQS) buffer"),
    ("IOLOGIC", "I/O register and DDR logic"),
    ("SIOLOGIC", "I/O register logic, single-ended pins"),
    ("DTR", "Die temperature sensor"),
    ("GSR", "Global set/reset"),
    ("JTAGG", "JTAG port access"),
    ("OSCG", "Internal oscillator"),
    ("SEDGA", "Soft error detection"),
    ("USRMCLK", "Configuration flash clock pin access"),
];

const GOWIN: &[(&str, &str)] = &[
    ("LUT4", "LUT: a 4-input lookup table"),
    ("DFF", "Flip-flop"),
    ("ALU", "Arithmetic logic: adder with carry"),
    (
        "MUX2_LUT5",
        "Wide-function multiplexer: two LUTs as a 5-input function",
    ),
    ("MUX2_LUT6", "Wide-function multiplexer: a 6-input function"),
    ("MUX2_LUT7", "Wide-function multiplexer: a 7-input function"),
    (
        "MUX2_LUT8",
        "Wide-function multiplexer: an 8-input function",
    ),
    ("RAM16SDP4", "Distributed RAM, 16x4, in LUTs"),
    ("BSRAM", "Block RAM, 18 Kbit"),
    ("MULT9X9", "Multiplier, 9x9"),
    ("MULT18X18", "Multiplier, 18x18"),
    ("MULT36X36", "Multiplier, 36x36"),
    ("MULTALU18X18", "Multiplier with accumulator, 18x18"),
    ("MULTALU36X18", "Multiplier with accumulator, 36x18"),
    (
        "MULTADDALU18X18",
        "Two multipliers with adder and accumulator, 18x18",
    ),
    ("ALU54D", "ALU, 54-bit, for multiplier results"),
    ("PADD9", "Pre-adder, 9-bit"),
    ("PADD18", "Pre-adder, 18-bit"),
    ("rPLL", "PLL (clock generator)"),
    ("PLLVR", "PLL (clock generator)"),
    ("OSC", "Internal oscillator"),
    ("OSCZ", "Internal oscillator"),
    ("OSCH", "Internal oscillator"),
    ("OSCF", "Internal oscillator"),
    ("BUFG", "Global clock buffer"),
    ("DQCE", "Global clock enable"),
    ("DCS", "Glitch-free clock selector"),
    ("DHCEN", "High-speed clock enable"),
    ("CLKDIV", "Clock divider"),
    ("CLKDIV2", "Clock divider by 2"),
    ("IOB", "I/O pin"),
    ("IOLOGICI", "Input register logic"),
    ("IOLOGICO", "Output register logic"),
    ("IDES16", "Input deserializer, 1:16"),
    ("OSER16", "Output serializer, 16:1"),
    ("MIPI_IBUF", "MIPI input buffer"),
    ("MIPI_OBUF", "MIPI output buffer"),
    ("FLASH608K", "User flash, 608 Kbit"),
    ("GSR", "Global set/reset"),
    ("GND", "Constant 0 source"),
    ("VCC", "Constant 1 source"),
];

const OPENXC7: &[(&str, &str)] = &[
    ("SLICE_LUTX", "LUT: a 6-input lookup table"),
    ("SLICE_FFX", "Flip-flop"),
    ("CARRY4", "Carry chain, 4-bit"),
    ("SELMUX2_1", "Wide-function multiplexer (F7 and F8 muxes)"),
    ("RAMB18E1", "Block RAM, 18 Kbit (half of a 36 Kbit block)"),
    ("RAMB36E1", "Block RAM, 36 Kbit"),
    ("RAMBFIFO36E1", "Block RAM, 36 Kbit, with FIFO logic"),
    ("FIFO18E1", "FIFO, 18 Kbit"),
    ("DSP48E1", "DSP slice: 25x18 multiplier and accumulator"),
    ("MMCME2_ADV", "Mixed-mode clock manager (clock generator)"),
    ("PLLE2_ADV", "PLL (clock generator)"),
    ("BUFG", "Global clock buffer"),
    ("BUFGCTRL", "Global clock buffer"),
    ("BUFHCE", "Horizontal clock buffer"),
    ("BUFIO", "I/O clock buffer"),
    ("BUFR", "Regional clock buffer"),
    ("PAD", "I/O pin"),
    ("IOB33_INBUF_EN", "Input buffer, 3.3 V bank"),
    (
        "IOB33M_INBUF_EN",
        "Input buffer, 3.3 V bank, differential pair's first pin",
    ),
    (
        "IOB33S_INBUF_EN",
        "Input buffer, 3.3 V bank, differential pair's second pin",
    ),
    ("IOB33_OUTBUF", "Output buffer, 3.3 V bank"),
    (
        "IOB33M_OUTBUF",
        "Output buffer, 3.3 V bank, differential pair's first pin",
    ),
    (
        "IOB33S_OUTBUF",
        "Output buffer, 3.3 V bank, differential pair's second pin",
    ),
    (
        "IOB33_TERM_OVERRIDE",
        "Input termination control, 3.3 V bank",
    ),
    (
        "IOB33M_TERM_OVERRIDE",
        "Input termination control, differential pair's first pin",
    ),
    (
        "IOB33S_TERM_OVERRIDE",
        "Input termination control, differential pair's second pin",
    ),
    ("ILOGICE2_IFF", "Input register"),
    ("ILOGICE3_IFF", "Input register"),
    ("ILOGICE3_ZHOLD_DELAY", "Input hold delay"),
    ("OLOGICE2_OUTFF", "Output register"),
    ("OLOGICE3_OUTFF", "Output register"),
    ("OLOGICE2_TFF", "Output enable register"),
    ("OLOGICE3_TFF", "Output enable register"),
    ("OLOGICE2_MISR", "Output signature register, for testing"),
    ("OLOGICE3_MISR", "Output signature register, for testing"),
    ("ISERDESE2", "Input deserializer"),
    ("OSERDESE2", "Output serializer"),
    ("IDELAYE2", "Input delay line"),
    ("IDELAYCTRL", "Delay line calibration"),
    ("DCIRESET", "Impedance control (DCI) reset"),
    ("PULL_OR_KEEP1", "Pin pull-up, pull-down or keeper"),
    ("INVERTER", "Signal inverter"),
    ("BUFFER", "Signal buffer"),
    ("HARD0", "Constant 0 source"),
    ("PSEUDO_GND", "Constant 0 source"),
    ("PSEUDO_VCC", "Constant 1 source"),
    ("GTPE2_CHANNEL", "High-speed transceiver (GTP)"),
    ("GTPE2_COMMON", "High-speed transceivers' shared PLL"),
    ("IBUFDS_GTE2", "Transceiver reference clock input"),
    ("PCIE_2_1", "PCI Express block"),
    ("XADC", "Analog-to-digital converter"),
    ("PS7", "Zynq processing system: the ARM cores"),
    ("BSCAN", "JTAG boundary-scan access"),
    ("DNA_PORT", "Device's unique ID"),
    ("EFUSE_USR", "User eFuse bits"),
    ("FRAME_ECC", "Configuration memory error check"),
    ("ICAP", "Internal configuration access"),
    ("STARTUP", "Startup control: configuration clock and pins"),
    (
        "USR_ACCESS",
        "User access register, such as a bitstream timestamp",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_known_names_only() {
        assert_eq!(
            describe(Toolchain::Ice40, "ICESTORM_LC"),
            Some("Logic cell: a 4-input LUT, a flip-flop and carry logic")
        );
        assert_eq!(
            describe(Toolchain::Openxc7, "SLICE_LUTX"),
            Some("LUT: a 6-input lookup table")
        );
        // Names are per toolchain: Gowin's GSR isn't Xilinx's.
        assert_eq!(describe(Toolchain::Openxc7, "GSR"), None);
        assert_eq!(describe(Toolchain::Gowin, "NOT_A_RESOURCE"), None);
    }

    #[test]
    fn descriptions_are_short_single_lines_and_names_unique() {
        for (arch, table) in [
            (Toolchain::Ice40, ICE40),
            (Toolchain::Ecp5, ECP5),
            (Toolchain::Gowin, GOWIN),
            (Toolchain::Openxc7, OPENXC7),
        ] {
            let mut names: Vec<_> = table.iter().map(|(n, _)| *n).collect();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), table.len(), "{arch:?}: a name twice");
            for (n, d) in table {
                assert!(
                    !d.is_empty() && d.len() <= 70 && !d.contains('\n'),
                    "{n}: {d}"
                );
                assert!(!d.ends_with('.'), "{n}: no final period");
            }
        }
    }
}
