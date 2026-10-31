//! Reads the output of yosys and nextpnr into diagnostics, and nextpnr's
//! report into resource use and clock frequencies.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::{Diagnostic, Severity};
use crate::text::to_text;

/// A tool's errors and warnings.
#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
}

fn yosys_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?:(?P<file>[^\s:][^:]*):(?P<line>\d+): )?(?P<kind>ERROR|Warning): (?P<text>.*)$",
        )
        .unwrap()
    })
}

fn nextpnr_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?P<kind>ERROR|Warning): (?P<text>.*?)(?: \(on line (?P<line>\d+)\))?$")
            .unwrap()
    })
}

/// Maps a file name from tool output to a path relative to the source
/// folder, or None when it's outside it (such as a yosys library file).
pub fn relative_file(file: &str, source_folder: &str) -> Option<String> {
    let f = file.strip_prefix("./").unwrap_or(file);
    if f.starts_with('/') {
        let prefix = format!("{}/", source_folder.trim_end_matches('/'));
        return f.strip_prefix(&prefix).map(str::to_string);
    }
    Some(f.to_string())
}

/// Parses yosys output. Files are reported as given on the command line,
/// relative to the source folder (`source_folder` is its real path).
pub fn parse_yosys(output: &str, source_folder: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for line in output.lines() {
        let Some(c) = yosys_line().captures(line.trim_end()) else {
            continue;
        };
        let severity = if &c["kind"] == "ERROR" {
            Severity::Error
        } else {
            Severity::Warning
        };
        let mut text = to_text(&c["text"]);
        let (mut file, mut line_no) = (None, None);
        if let (Some(f), Some(l)) = (c.name("file"), c.name("line")) {
            match relative_file(f.as_str(), source_folder) {
                Some(rel) => {
                    file = Some(rel);
                    line_no = l.as_str().parse().ok();
                }
                // Keep the location in the text when it's not a design file.
                None => text = format!("{}:{}: {text}", f.as_str(), l.as_str()),
            }
        }
        let d = Diagnostic {
            file,
            line: line_no,
            column: None,
            severity,
            text,
            tool: Some("yosys".into()),
            time_ns: None,
        };
        match severity {
            Severity::Error => parsed.errors.push(d),
            _ => parsed.warnings.push(d),
        }
    }
    parsed
}

/// Parses nextpnr output. Messages with "(on line N)" are about the
/// constraint file, `constraint_file` relative to the source folder.
pub fn parse_nextpnr(output: &str, constraint_file: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for line in output.lines() {
        let Some(c) = nextpnr_line().captures(line.trim_end()) else {
            continue;
        };
        let text = to_text(&c["text"]);
        // A summary of the errors above it, not an error of its own.
        if text == "Loading PCF failed." {
            continue;
        }
        let severity = if &c["kind"] == "ERROR" {
            Severity::Error
        } else {
            Severity::Warning
        };
        let line_no: Option<u32> = c.name("line").and_then(|l| l.as_str().parse().ok());
        let d = Diagnostic {
            file: line_no.map(|_| constraint_file.to_string()),
            line: line_no,
            column: None,
            severity,
            text,
            tool: Some("nextpnr".into()),
            time_ns: None,
        };
        match severity {
            Severity::Error => parsed.errors.push(d),
            _ => parsed.warnings.push(d),
        }
    }
    parsed
}

/// How much of one FPGA resource the design uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceUsage {
    pub used: u64,
    pub available: u64,
}

#[derive(Debug, Deserialize)]
struct Report {
    #[serde(default)]
    utilization: BTreeMap<String, ResourceUsage>,
    #[serde(default)]
    fmax: BTreeMap<String, Fmax>,
}

#[derive(Debug, Deserialize)]
struct Fmax {
    achieved: f64,
}

/// Resource use by resource name.
pub type Resources = BTreeMap<String, ResourceUsage>;
/// Achieved frequency in MHz by clock net.
pub type Timing = BTreeMap<String, f64>;

/// Reads nextpnr's JSON report: resource use by resource name, and the
/// achieved frequency in MHz by clock net, rounded to 0.01 MHz.
pub fn parse_report(json: &str) -> Result<(Resources, Timing), String> {
    let r: Report = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let timing = r
        .fmax
        .into_iter()
        .map(|(clock, f)| (clock, (f.achieved * 100.0).round() / 100.0))
        .collect();
    Ok((r.utilization, timing))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real output from yosys 0.70 (oss-cad-suite 2026-10-22).
    #[test]
    fn yosys_errors_and_warnings() {
        let out = "bad2.v:2: Warning: Identifier `\\undeclared_sig' is implicitly declared.\n\
                   ERROR: Module `\\foo' referenced in module `\\main' in cell `\\u' is not part of the design.";
        let p = parse_yosys(out, "/src");
        assert_eq!(p.warnings.len(), 1);
        assert_eq!(p.warnings[0].file.as_deref(), Some("bad2.v"));
        assert_eq!(p.warnings[0].line, Some(2));
        assert_eq!(p.errors.len(), 1);
        assert_eq!(p.errors[0].file, None);
        assert!(p.errors[0].text.starts_with("Module `\\foo'"));

        let p = parse_yosys(
            "rtl/bad.v:3: ERROR: syntax error, unexpected TOK_ASSIGN, expecting ',' or ';' or '=' or '['",
            "/src",
        );
        assert_eq!(p.errors[0].file.as_deref(), Some("rtl/bad.v"));
        assert_eq!(p.errors[0].line, Some(3));
        assert_eq!(p.errors[0].tool.as_deref(), Some("yosys"));

        let p = parse_yosys(
            "/opt/oss/share/yosys/ice40/cells_sim.v:10: Warning: something\n/src/a.v:4: Warning: other\nnoise line",
            "/src",
        );
        assert_eq!(p.warnings[0].file, None);
        assert!(
            p.warnings[0]
                .text
                .starts_with("/opt/oss/share/yosys/ice40/cells_sim.v:10: ")
        );
        assert_eq!(p.warnings[1].file.as_deref(), Some("a.v"));
    }

    // Real output from nextpnr-ice40 (oss-cad-suite 2026-10-22).
    #[test]
    fn nextpnr_constraint_errors() {
        let out = "ERROR: package does not have a pin named '999' (on line 2)\n\
                   ERROR: Loading PCF failed.\n0 warnings, 2 errors";
        let p = parse_nextpnr(out, "pinout.pcf");
        assert_eq!(p.errors.len(), 1);
        assert_eq!(p.errors[0].file.as_deref(), Some("pinout.pcf"));
        assert_eq!(p.errors[0].line, Some(2));
        assert_eq!(p.errors[0].text, "package does not have a pin named '999'");

        let p = parse_nextpnr(
            "ERROR: IO 'LED1' is unconstrained in PCF (override this error with --pcf-allow-unconstrained)\nWarning: unmatched constraint 'NOPE' (on line 4)",
            "pinout.pcf",
        );
        assert_eq!(p.errors[0].file, None);
        assert!(p.errors[0].text.contains("LED1"));
        assert_eq!(p.warnings[0].line, Some(4));
    }

    #[test]
    fn report() {
        let json = r#"{"critical_paths": [], "fmax": {"CLK$SB_IO_IN_$glb_clk": {"achieved": 194.36346435546875, "constraint": 12}},
            "utilization": {"ICESTORM_LC": {"available": 7680, "used": 27}, "SB_IO": {"available": 107, "used": 3}}}"#;
        let (res, timing) = parse_report(json).unwrap();
        assert_eq!(
            res["ICESTORM_LC"],
            ResourceUsage {
                used: 27,
                available: 7680
            }
        );
        assert_eq!(timing["CLK$SB_IO_IN_$glb_clk"], 194.36);
    }
}
