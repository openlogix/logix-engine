//! A build's resource use and clocks for people (docs/tool-api.md, build
//! tool), so hosts don't need to know each FPGA family's names: every
//! resource with a description and a share that's checked (`resources`),
//! the main kinds in plain words ("logic cells" for ICESTORM_LC), clocks by
//! plain names ("CLK" for the net CLK$SB_IO_IN_$glb_clk), and the report's
//! text, so every frontend words it the same way.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::parse::ResourceUsage;
use crate::catalog::Toolchain;

/// One resource the toolchain reports, by its own name, with a short
/// description and how much the design uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    /// The toolchain's name, as nextpnr reports it, such as ICESTORM_LC or
    /// SLICE_LUTX: what people search for and quote.
    pub name: String,
    /// What it is, in a few plain words, such as "Logic cell: a 4-input
    /// LUT, a flip-flop and carry logic". None for a name the engine
    /// doesn't describe yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// How many the design uses, as nextpnr reports it.
    pub used: u64,
    /// How many the FPGA has, as nextpnr reports it.
    pub available: u64,
    /// The share used, in percent, rounded to 0.1. None when it can't be
    /// computed or isn't valid: available is 0, or used is more than
    /// available (nextpnr data the engine doesn't trust, as Apio's report).
    pub percent: Option<f64>,
    /// The share for showing to people, in whole percent: the preferred
    /// way to show the share to people. "0%" is guaranteed only when none
    /// is used and "100%" only when all are used; anything else is rounded
    /// up ("1%" for 27 of 7680, "99%" at most before all are used). None
    /// when percent is.
    pub display_percent: Option<String>,
}

/// One kind of FPGA resource, in plain words, with how much the design uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceSummary {
    /// The kind: logic_cells, luts, flip_flops, ram_blocks, multipliers,
    /// plls or io_pins. More kinds may be added.
    pub kind: String,
    /// The kind in plain words, plural, such as "logic cells" or "LUTs", for
    /// showing to people.
    pub label: String,
    /// How many the design uses.
    pub used: u64,
    /// How many the FPGA has.
    pub available: u64,
    /// The share used, in percent, rounded to 0.1. None when it can't be
    /// computed or isn't valid, as for resources.
    pub percent: Option<f64>,
    /// The share for showing to people, as for resources.
    pub display_percent: Option<String>,
}

/// The share used, in percent, unrounded, or None when it isn't valid:
/// nothing available, or more used than available.
fn share(used: u64, available: u64) -> Option<f64> {
    (available > 0 && used <= available).then(|| 100.0 * used as f64 / available as f64)
}

fn round1(p: f64) -> f64 {
    (p * 10.0).round() / 10.0
}

/// Every resource the toolchain reports, by name (ignoring case), each with
/// its description and share.
pub fn resources(arch: Toolchain, usage: &BTreeMap<String, ResourceUsage>) -> Vec<Resource> {
    let mut list: Vec<Resource> = usage
        .iter()
        .map(|(name, u)| Resource {
            name: name.clone(),
            description: super::descriptions::describe(arch, name).map(String::from),
            used: u.used,
            available: u.available,
            percent: share(u.used, u.available).map(round1),
            display_percent: display_percent(u.used, u.available),
        })
        .collect();
    list.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.name.cmp(&b.name))
    });
    list
}

/// Per family, the kinds of resources, the family's main capacity measure
/// first, each with the toolchain names whose counts are added up.
fn kinds(arch: Toolchain) -> &'static [(&'static str, &'static str, &'static [&'static str])] {
    match arch {
        Toolchain::Ice40 => &[
            ("logic_cells", "logic cells", &["ICESTORM_LC"]),
            ("ram_blocks", "RAM blocks", &["ICESTORM_RAM"]),
            ("multipliers", "multipliers", &["ICESTORM_DSP"]),
            ("plls", "PLLs", &["ICESTORM_PLL"]),
            ("io_pins", "I/O pins", &["SB_IO"]),
        ],
        Toolchain::Ecp5 => &[
            ("luts", "LUTs", &["TRELLIS_COMB"]),
            ("flip_flops", "flip-flops", &["TRELLIS_FF"]),
            ("ram_blocks", "RAM blocks", &["DP16KD"]),
            ("multipliers", "multipliers", &["MULT18X18D"]),
            ("plls", "PLLs", &["EHXPLLL"]),
            ("io_pins", "I/O pins", &["TRELLIS_IO"]),
        ],
        Toolchain::Gowin => &[
            ("luts", "LUTs", &["LUT4"]),
            ("flip_flops", "flip-flops", &["DFF"]),
            ("ram_blocks", "RAM blocks", &["BSRAM"]),
            ("multipliers", "multipliers", &["MULT18X18"]),
            ("plls", "PLLs", &["rPLL", "PLLVR"]),
            ("io_pins", "I/O pins", &["IOB"]),
        ],
        Toolchain::Openxc7 => &[
            ("luts", "LUTs", &["SLICE_LUTX"]),
            ("flip_flops", "flip-flops", &["SLICE_FFX"]),
            // 36 Kbit blocks: an 18 Kbit RAM or FIFO is half of one (the
            // same sites, so their availability isn't added; see
            // xilinx_ram_blocks).
            (
                "ram_blocks",
                "RAM blocks",
                &["RAMB36E1", "RAMBFIFO36E1", "RAMB18E1", "FIFO18E1"],
            ),
            ("multipliers", "multipliers", &["DSP48E1"]),
            ("plls", "PLLs", &["PLLE2_ADV", "MMCME2_ADV"]),
            ("io_pins", "I/O pins", &["PAD"]),
        ],
    }
}

/// Xilinx RAM blocks in 36 Kbit blocks: 36 Kbit RAMs and FIFOs count one,
/// two 18 Kbit ones (rounded up) count one, and the 36 Kbit sites are what
/// the FPGA has (its 18 Kbit sites are halves of the same blocks). None
/// when a count is invalid.
fn xilinx_ram_blocks(usage: &BTreeMap<String, ResourceUsage>) -> Option<(u64, u64, bool)> {
    let get = |n: &str| usage.get(n);
    let r36 = get("RAMB36E1")?;
    let valid = ["RAMB36E1", "RAMBFIFO36E1", "RAMB18E1", "FIFO18E1"]
        .iter()
        .filter_map(|n| get(n))
        .all(|u| u.used <= u.available);
    let used = |n: &str| get(n).map_or(0, |u| u.used);
    let halves = used("RAMB18E1") + used("FIFO18E1");
    Some((
        r36.used + used("RAMBFIFO36E1") + halves.div_ceil(2),
        r36.available,
        valid,
    ))
}

/// The summary of a build's resource use: the family's main capacity
/// measure first, then the other kinds, each when the FPGA has any. Kinds
/// whose resources the report doesn't list are left out. A kind's share is
/// None when any of its counts is invalid (more used than available).
pub fn summarize(arch: Toolchain, usage: &BTreeMap<String, ResourceUsage>) -> Vec<ResourceSummary> {
    let mut out = Vec::new();
    for (i, (kind, label, names)) in kinds(arch).iter().enumerate() {
        let (used, available, valid) = if arch == Toolchain::Openxc7 && *kind == "ram_blocks" {
            match xilinx_ram_blocks(usage) {
                Some(t) => t,
                None => continue,
            }
        } else {
            let found: Vec<&ResourceUsage> = names.iter().filter_map(|n| usage.get(*n)).collect();
            if found.is_empty() {
                continue;
            }
            (
                found.iter().map(|u| u.used).sum(),
                found.iter().map(|u| u.available).sum(),
                found.iter().all(|u| u.used <= u.available),
            )
        };
        // The main measure always; others only when the FPGA has them.
        if i == 0 || available > 0 {
            out.push(ResourceSummary {
                kind: kind.to_string(),
                label: label.to_string(),
                used,
                available,
                percent: share(used, available).filter(|_| valid).map(round1),
                display_percent: display_percent(used, available).filter(|_| valid),
            });
        }
    }
    out
}

/// A share for people, in whole percent: "0%" when none is used, otherwise
/// rounded up, so something used is never "0%", and "100%" only when all
/// are used (at most "99%" before that). None when it isn't valid.
fn display_percent(used: u64, available: u64) -> Option<String> {
    share(used, available)?;
    let up = (100 * used).div_ceil(available);
    let p = if used < available { up.min(99) } else { 100 };
    Some(format!("{p}%"))
}

/// The build's report for people, a line each: the main resource always,
/// the others when used, then each clock's maximum speed, such as "The
/// design uses 27 of 7680 logic cells (1%)." and "Maximum clock speed:
/// 194.4 MHz (sys_clk)." A share that isn't valid is left out.
pub fn report(summary: &[ResourceSummary], clocks: &[ClockSummary]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (i, s) in summary.iter().enumerate() {
        if i > 0 && s.used == 0 {
            continue;
        }
        let pct = s
            .display_percent
            .as_ref()
            .map(|p| format!(" ({p})"))
            .unwrap_or_default();
        lines.push(format!(
            "The design uses {} of {} {}{pct}.",
            s.used, s.available, s.label
        ));
    }
    for c in clocks {
        lines.push(format!(
            "Maximum clock speed: {:.1} MHz ({}).",
            c.max_mhz, c.name
        ));
    }
    lines.join("\n")
}

/// The build's report in one line, for a status bar: the main resource's
/// share and the slowest clock, such as "Uses 1% of logic cells, max
/// clock 194.4 MHz." Empty when there's nothing to report.
pub fn report_summary(summary: &[ResourceSummary], clocks: &[ClockSummary]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(main) = summary.first() {
        parts.push(match &main.display_percent {
            Some(p) => format!("uses {p} of {}", main.label),
            None => format!("uses {} {}", main.used, main.label),
        });
    }
    if let Some(slowest) = clocks.iter().map(|c| c.max_mhz).reduce(f64::min) {
        parts.push(format!("max clock {slowest:.1} MHz"));
    }
    if parts.is_empty() {
        return String::new();
    }
    let text = parts.join(", ");
    let mut chars = text.chars();
    let first = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    format!("{first}{}.", chars.as_str())
}

/// One clock of the design, with a plain name and its maximum frequency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClockSummary {
    /// The clock's name for showing to people, usually the design's clock
    /// signal, such as "CLK".
    pub name: String,
    /// The clock net name nextpnr reports, the key in timing.
    pub net: String,
    /// The maximum frequency the clock can run at, in MHz.
    pub max_mhz: f64,
}

/// A plain name for a clock net, per family, as the toolchains name the
/// net after the signal (the same rules as Apio's build report):
///
/// - iCE40: the part before the first `$`, without trailing `_`
///   (`CLK$SB_IO_IN_$glb_clk` is CLK).
/// - ECP5: the third `$`-separated part (`$glbnet$CLK$TRELLIS_IO_IN` is CLK).
/// - Gowin: the part before the first `$`, without `_IBUF_I_O`
///   (`CLK_IBUF_I_O` is CLK).
/// - Xilinx: the last non-empty part. nextpnr-xilinx from openXC7
///   2026-11-09 names the net after the signal (`CLK`); older ones reported
///   a name synthesis made up (`$abc$21083$aiger$o100`, kept as o100).
///
/// A net that doesn't follow the pattern keeps its full name.
pub fn clock_name(arch: Toolchain, net: &str) -> String {
    let parts: Vec<&str> = net.split('$').collect();
    let name = match arch {
        Toolchain::Ice40 => parts[0].trim_end_matches('_'),
        Toolchain::Ecp5 => parts.get(2).copied().unwrap_or(""),
        Toolchain::Gowin => parts[0].trim_end_matches("_IBUF_I_O"),
        Toolchain::Openxc7 => parts
            .iter()
            .rev()
            .find(|p| !p.is_empty())
            .copied()
            .unwrap_or(""),
    };
    if name.is_empty() { net } else { name }.to_string()
}

/// The design's clocks with plain names, in the order of timing (by net).
pub fn summarize_clocks(arch: Toolchain, timing: &BTreeMap<String, f64>) -> Vec<ClockSummary> {
    timing
        .iter()
        .map(|(net, mhz)| ClockSummary {
            name: clock_name(arch, net),
            net: net.clone(),
            max_mhz: *mhz,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_clocks_as_the_design_does() {
        use Toolchain::*;
        assert_eq!(clock_name(Ice40, "CLK$SB_IO_IN_$glb_clk"), "CLK");
        assert_eq!(clock_name(Ice40, "sys_clk$SB_IO_IN"), "sys_clk");
        assert_eq!(clock_name(Ecp5, "$glbnet$CLK$TRELLIS_IO_IN"), "CLK");
        assert_eq!(clock_name(Gowin, "sys_clk_IBUF_I_O"), "sys_clk");
        assert_eq!(clock_name(Openxc7, "$abc$21083$aiger$o100"), "o100");
        assert_eq!(
            clock_name(Openxc7, "sys_clk_IBUF_BUFG"),
            "sys_clk_IBUF_BUFG"
        );
        // Nets that don't follow the pattern keep their name.
        assert_eq!(clock_name(Ecp5, "clk"), "clk");
        assert_eq!(clock_name(Ice40, "$glb$x"), "$glb$x");

        let timing = BTreeMap::from([("CLK_IBUF_I_O".to_string(), 199.5)]);
        assert_eq!(
            summarize_clocks(Gowin, &timing),
            [ClockSummary {
                name: "CLK".into(),
                net: "CLK_IBUF_I_O".into(),
                max_mhz: 199.5
            }]
        );
    }

    fn usage(pairs: &[(&str, u64, u64)]) -> BTreeMap<String, ResourceUsage> {
        pairs
            .iter()
            .map(|(n, used, available)| {
                (
                    n.to_string(),
                    ResourceUsage {
                        used: *used,
                        available: *available,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn names_resources_in_plain_words_main_first() {
        let ice40 = usage(&[
            ("ICESTORM_LC", 27, 7680),
            ("ICESTORM_RAM", 0, 32),
            ("SB_IO", 2, 107),
            ("SB_GB", 1, 8),
        ]);
        let s = summarize(Toolchain::Ice40, &ice40);
        let kinds: Vec<_> = s
            .iter()
            .map(|r| (r.kind.as_str(), r.used, r.available))
            .collect();
        assert_eq!(
            kinds,
            [
                ("logic_cells", 27, 7680),
                ("ram_blocks", 0, 32),
                ("io_pins", 2, 107)
            ]
        );
        assert_eq!(s[0].label, "logic cells");

        // Xilinx RAM blocks count 36 Kbit blocks (two 18 Kbit ones are one);
        // a kind the FPGA lacks (available 0) is left out.
        let xilinx = usage(&[
            ("SLICE_LUTX", 56, 20800),
            ("RAMB36E1", 1, 50),
            ("RAMB18E1", 2, 100),
            ("DSP48E1", 0, 0),
        ]);
        let s = summarize(Toolchain::Openxc7, &xilinx);
        let kinds: Vec<_> = s
            .iter()
            .map(|r| (r.kind.as_str(), r.used, r.available))
            .collect();
        assert_eq!(kinds, [("luts", 56, 20800), ("ram_blocks", 2, 50)]);
        // Two toolchain names for one kind are added up.
        let gowin = usage(&[("LUT4", 9, 8640), ("rPLL", 1, 2), ("PLLVR", 0, 0)]);
        let s = summarize(Toolchain::Gowin, &gowin);
        assert_eq!(
            (s[1].kind.as_str(), s[1].used, s[1].available),
            ("plls", 1, 2)
        );
    }

    #[test]
    fn shares_are_checked_like_apios() {
        let r = resources(
            Toolchain::Ice40,
            &usage(&[
                ("SB_IO", 3, 39),
                ("ICESTORM_LC", 27, 5280),
                ("odd", 5, 0),
                ("over", 9, 4),
                ("ICESTORM_PLL", 0, 1),
            ]),
        );
        let rows: Vec<_> = r.iter().map(|x| (x.name.as_str(), x.percent)).collect();
        // By name, ignoring case; no share when nothing is available or
        // more is used than available.
        assert_eq!(
            rows,
            [
                ("ICESTORM_LC", Some(0.5)),
                ("ICESTORM_PLL", Some(0.0)),
                ("odd", None),
                ("over", None),
                ("SB_IO", Some(7.7))
            ]
        );
        assert_eq!(
            r[0].description.as_deref(),
            Some("Logic cell: a 4-input LUT, a flip-flop and carry logic")
        );
        assert_eq!(r[2].description, None);
        // A kind with an invalid count has no share.
        let s = summarize(Toolchain::Ice40, &usage(&[("ICESTORM_LC", 9000, 7680)]));
        assert_eq!((s[0].used, s[0].percent), (9000, None));
    }

    #[test]
    fn the_report_for_people() {
        let summary = summarize(
            Toolchain::Ice40,
            &usage(&[
                ("ICESTORM_LC", 27, 7680),
                ("ICESTORM_RAM", 0, 32),
                ("SB_IO", 2, 107),
            ]),
        );
        let clocks = [
            ClockSummary {
                name: "sys_clk".into(),
                net: "n1".into(),
                max_mhz: 194.36,
            },
            ClockSummary {
                name: "fast".into(),
                net: "n2".into(),
                max_mhz: 88.0,
            },
        ];
        assert_eq!(
            report(&summary, &clocks),
            "The design uses 27 of 7680 logic cells (1%).\n\
             The design uses 2 of 107 I/O pins (2%).\n\
             Maximum clock speed: 194.4 MHz (sys_clk).\n\
             Maximum clock speed: 88.0 MHz (fast)."
        );
        assert_eq!(
            report_summary(&summary, &clocks),
            "Uses 1% of logic cells, max clock 88.0 MHz."
        );
        // Invalid numbers: no share; nothing at all: empty.
        let bad = summarize(Toolchain::Ice40, &usage(&[("ICESTORM_LC", 9000, 7680)]));
        assert_eq!(
            report(&bad, &[]),
            "The design uses 9000 of 7680 logic cells."
        );
        assert_eq!(report_summary(&bad, &[]), "Uses 9000 logic cells.");
        assert_eq!(report_summary(&[], &clocks[..1]), "Max clock 194.4 MHz.");
        assert_eq!(report_summary(&[], &[]), "");
        // Whole percent, rounded up: something used is never "0%", and
        // "100%" only when all are used.
        let d = |u, a| display_percent(u, a);
        assert_eq!(d(0, 65200).as_deref(), Some("0%"));
        assert_eq!(d(1, 65200).as_deref(), Some("1%"));
        assert_eq!(d(100, 1000).as_deref(), Some("10%"));
        assert_eq!(d(101, 1000).as_deref(), Some("11%"));
        assert_eq!(d(999, 1000).as_deref(), Some("99%"));
        assert_eq!(d(1000, 1000).as_deref(), Some("100%"));
        assert_eq!(d(5, 0), None);
        assert_eq!(d(9, 8), None);
    }
}
