//! Reads Verilator and Icarus Verilog output into diagnostics.

use std::sync::OnceLock;

use regex::Regex;

use crate::build::parse::relative_file;
use crate::errors::{Diagnostic, Severity};
use crate::text::to_text;

fn verilator_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^%(?P<kind>Error|Warning)(?:-(?P<code>[A-Za-z0-9_]+))?: (?:(?P<file>[^\s:][^:]*):(?P<line>\d+):(?:(?P<col>\d+):)? )?(?P<text>.*)$",
        )
        .unwrap()
    })
}

/// Fills in a diagnostic's location from a file and line in tool output.
/// Files outside the source folder, such as the toolchain's own models, keep
/// their location in the text instead.
fn locate(
    d: &mut Diagnostic,
    file: Option<&str>,
    line: Option<&str>,
    col: Option<&str>,
    source_folder: &str,
) {
    let (Some(f), Some(l)) = (file, line) else {
        return;
    };
    match relative_file(f, source_folder) {
        Some(rel) => {
            d.file = Some(rel);
            d.line = l.parse().ok();
            d.column = col.and_then(|c| c.parse().ok());
        }
        None => d.text = format!("{f}:{l}: {}", d.text),
    }
}

/// Parses Verilator output: the first line of each "%Error" and "%Warning"
/// message. A warning's code, such as WIDTHTRUNC, is kept at the end of the
/// text, since it is what a lint_off comment names. Summaries, such as
/// "Cannot continue", are left out.
pub fn parse_verilator(output: &str, source_folder: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in output.lines() {
        let Some(c) = verilator_line().captures(line.trim_end()) else {
            continue;
        };
        let text = to_text(&c["text"]);
        if c.name("file").is_none()
            && (text == "Cannot continue" || text.starts_with("Exiting due to"))
        {
            continue;
        }
        let severity = if &c["kind"] == "Error" {
            Severity::Error
        } else {
            Severity::Warning
        };
        let text = match c.name("code") {
            Some(code) => format!("{text} ({})", code.as_str()),
            None => text,
        };
        let mut d = Diagnostic {
            file: None,
            line: None,
            column: None,
            severity,
            text,
            tool: Some("verilator".into()),
            time_ns: None,
        };
        locate(
            &mut d,
            c.name("file").map(|m| m.as_str()),
            c.name("line").map(|m| m.as_str()),
            c.name("col").map(|m| m.as_str()),
            source_folder,
        );
        out.push(d);
    }
    out
}

fn iverilog_located() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?P<file>[^\s:][^:]*):(?P<line>\d+):\s*(?:(?P<kind>error|warning|sorry):\s*)?(?P<text>[^:\s].*)$",
        )
        .unwrap()
    })
}

fn iverilog_unlocated() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?P<kind>error|warning|sorry): (?P<text>.*)$").unwrap())
}

/// Parses iverilog's compile output. Tagged lines are errors, warnings or
/// "sorry" (an unsupported feature, an error). Untagged lines with a
/// location, such as "main.v:3: syntax error", are errors. Notes that
/// continue a message ("main.v:2:        : Padding ...") and summaries are
/// left out.
pub fn parse_iverilog(output: &str, source_folder: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in output.lines() {
        let line = line.trim_end();
        let (caps, located) = match iverilog_located().captures(line) {
            Some(c) => (c, true),
            None => match iverilog_unlocated().captures(line) {
                Some(c) => (c, false),
                None => continue,
            },
        };
        let severity = match caps.name("kind").map(|k| k.as_str()) {
            Some("warning") => Severity::Warning,
            _ => Severity::Error,
        };
        let mut d = Diagnostic {
            file: None,
            line: None,
            column: None,
            severity,
            text: to_text(&caps["text"]),
            tool: Some("iverilog".into()),
            time_ns: None,
        };
        if located {
            locate(
                &mut d,
                caps.name("file").map(|m| m.as_str()),
                caps.name("line").map(|m| m.as_str()),
                None,
                source_folder,
            );
        }
        out.push(d);
    }
    out
}

fn vvp_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?P<kind>FATAL|ERROR|WARNING): (?P<file>[^\s:][^:]*):(?P<line>\d+):\s*(?P<text>.*)$",
        )
        .unwrap()
    })
}

/// "*** ASSERTION FAILED at main_tb.v:52: ..." (Apio's older examples) and
/// "*** ASSERTION FAILED in main_tb (clk_num=400): ..." (a common variant
/// without a location).
fn assertion_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\*+\s*ASSERTION FAILED(?: at (?P<file>[^\s:][^:]*):(?P<line>\d+):)?\s*:?\s*(?P<text>.*)$",
        )
        .unwrap()
    })
}

/// "ERROR at main_tb.v:52: expected ..." (the EXPECT_EQ macro in Apio's
/// docs).
fn error_at_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^ERROR at (?P<file>[^\s:][^:]*):(?P<line>\d+):\s*(?P<text>.*)$").unwrap()
    })
}

fn time_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s+Time: (?P<ticks>\d+)\s+Scope: ").unwrap())
}

/// How a line of a testbench's output reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VvpLine {
    /// A failure from $fatal or $error, followed by its "Time:" line.
    TaggedFailure,
    /// A failure line from Apio's assertion macros, with nothing after it.
    MacroFailure,
    /// Anything else.
    Other,
}

/// Classifies one line of a testbench's output, so a run can be stopped at
/// its first failure.
pub fn classify_vvp_line(line: &str) -> VvpLine {
    let line = line.trim_end();
    if let Some(c) = vvp_line().captures(line) {
        if &c["kind"] != "WARNING" {
            return VvpLine::TaggedFailure;
        }
        return VvpLine::Other;
    }
    if assertion_line().is_match(line) || error_at_line().is_match(line) {
        return VvpLine::MacroFailure;
    }
    VvpLine::Other
}

/// A message from a testbench run, with the simulation time vvp gave for it,
/// in the waveform's time units (its $timescale), when known.
#[derive(Debug, Clone, PartialEq)]
pub struct VvpMessage {
    pub diagnostic: Diagnostic,
    pub ticks: Option<u64>,
}

/// Parses a testbench run's output from vvp: the messages of $fatal, $error
/// and $warning, with the time from the "Time:" line vvp prints after each,
/// and the failure lines Apio's assertion macros print. Under simulate those
/// macros skip $fatal (APIO_SIM is 1), so their line is the only sign of a
/// failure. Other lines, such as a testbench's own $display text, are left
/// out.
pub fn parse_vvp(output: &str, source_folder: &str) -> Vec<VvpMessage> {
    let mut out: Vec<VvpMessage> = Vec::new();
    // The tagged message the next "Time:" line belongs to.
    let mut awaiting_time: Option<usize> = None;
    for line in output.lines() {
        let line = line.trim_end();
        if let (Some(i), Some(c)) = (awaiting_time.take(), time_line().captures(line)) {
            out[i].ticks = c["ticks"].parse().ok();
            continue;
        }
        let (c, severity, text, tagged) = if let Some(c) = vvp_line().captures(line) {
            let kind = c["kind"].to_string();
            let mut text = to_text(&c["text"]);
            if text.is_empty() {
                text = match kind.as_str() {
                    "FATAL" => "The testbench stopped with $fatal.".into(),
                    "ERROR" => "The testbench reported an error.".into(),
                    _ => "The testbench reported a warning.".into(),
                };
            }
            let severity = if kind == "WARNING" {
                Severity::Warning
            } else {
                Severity::Error
            };
            (c, severity, text, true)
        } else if let Some(c) = assertion_line().captures(line) {
            let rest = to_text(&c["text"]);
            let text = if c.name("file").is_some() || !rest.starts_with("in ") {
                format!("Assertion failed: {rest}")
            } else {
                format!("Assertion failed {rest}")
            };
            (c, Severity::Error, text, false)
        } else if let Some(c) = error_at_line().captures(line) {
            let text = to_text(&c["text"]);
            (c, Severity::Error, text, false)
        } else {
            continue;
        };
        let mut d = Diagnostic {
            file: None,
            line: None,
            column: None,
            severity,
            text,
            tool: Some("vvp".into()),
            time_ns: None,
        };
        locate(
            &mut d,
            c.name("file").map(|m| m.as_str()),
            c.name("line").map(|m| m.as_str()),
            None,
            source_folder,
        );
        if tagged {
            awaiting_time = Some(out.len());
        }
        out.push(VvpMessage {
            diagnostic: d,
            ticks: None,
        });
    }
    out
}

/// The length of one time unit of a VCD file, in nanoseconds, from its
/// $timescale, such as "1ps" (0.001) or "10 ns" (10).
pub fn vcd_timescale_ns(vcd: impl std::io::BufRead) -> Option<f64> {
    let mut words = vcd
        .lines()
        .map_while(Result::ok)
        .flat_map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>());
    words.find(|w| w == "$timescale")?;
    let text: String = words.take_while(|w| w != "$end").collect();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let number: f64 = digits.parse().ok()?;
    let unit = match &text[digits.len()..] {
        "s" => 1e9,
        "ms" => 1e6,
        "us" => 1e3,
        "ns" => 1.0,
        "ps" => 1e-3,
        "fs" => 1e-6,
        _ => return None,
    };
    Some(number * unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real output from Verilator 5.053 (oss-cad-suite 2026-10-22).
    #[test]
    fn verilator_warnings_and_errors() {
        let out = "%Warning-WIDTHTRUNC: main.v:6:9: Operator ASSIGNDLY expects 4 bits on the Assign RHS, but Assign RHS's VARREF 'cnt' generates 8 bits.
                               : ... note: In instance 'main'
    6 |     LED <= cnt;
      |         ^~
                     ... For warning description see https://verilator.org/warn/WIDTHTRUNC?v=5.053
%Error: rtl/bad.v:3:1: syntax error, unexpected endmodule, expecting ',' or ';'
    3 | endmodule
      | ^~~~~~~~~
%Error: Cannot continue
        ... This fatal error may be caused by the earlier error(s); resolve those first.
%Error-MODMISSING: m.v:1:11: Cannot find file containing module: 'foo'
%Error: Exiting due to 3 warning(s)
%Warning-UNUSED: /src/sub.v:2:8: Signal is not used: 'x'";
        let d = parse_verilator(out, "/src");
        assert_eq!(d.len(), 4, "{d:#?}");
        assert_eq!(d[0].severity, Severity::Warning);
        assert_eq!(d[0].file.as_deref(), Some("main.v"));
        assert_eq!((d[0].line, d[0].column), (Some(6), Some(9)));
        assert!(d[0].text.starts_with("Operator ASSIGNDLY expects 4 bits"));
        assert!(d[0].text.ends_with("(WIDTHTRUNC)"));
        assert_eq!(d[0].tool.as_deref(), Some("verilator"));
        assert_eq!(d[1].severity, Severity::Error);
        assert_eq!(d[1].file.as_deref(), Some("rtl/bad.v"));
        assert_eq!(
            d[1].text,
            "syntax error, unexpected endmodule, expecting ',' or ';'"
        );
        assert_eq!(
            d[2].text,
            "Cannot find file containing module: 'foo' (MODMISSING)"
        );
        // An absolute path in the source folder becomes relative.
        assert_eq!(d[3].file.as_deref(), Some("sub.v"));
    }

    #[test]
    fn verilator_locations_outside_the_source_folder_stay_in_the_text() {
        let d = parse_verilator(
            "%Warning-PINMISSING: /pkg/share/yosys/ice40/cells_sim.v:10:3: Cell has missing pin",
            "/src",
        );
        assert_eq!(d[0].file, None);
        assert_eq!(d[0].line, None);
        assert!(
            d[0].text
                .starts_with("/pkg/share/yosys/ice40/cells_sim.v:10: Cell has missing pin")
        );
    }

    // Real output from Icarus Verilog 14.0 (oss-cad-suite 2026-10-22).
    #[test]
    fn iverilog_compile_errors_and_warnings() {
        let out = "bad.v:3: syntax error
bad.v:2: error: Syntax error in continuous assignment
rtl/sub.v:2: Include file local.vh not found
Preprocessor failed with 1 error(s).
p.v:2: error: port ``zz'' is not a port of u.
p.v:2: warning: Port 1 (a) of module p expects 4 bit(s), given 2.
p.v:2:        : Padding 2 high bits of the port.
2 error(s) during elaboration.
*** These modules were missing:
        foo referenced 1 times.
***
nothere.v: No such file or directory
error: Unable to find the root module";
        let d = parse_iverilog(out, "/src");
        let texts: Vec<_> = d.iter().map(|d| d.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "syntax error",
                "Syntax error in continuous assignment",
                "Include file local.vh not found",
                "port ``zz'' is not a port of u.",
                "Port 1 (a) of module p expects 4 bit(s), given 2.",
                "Unable to find the root module",
            ]
        );
        assert_eq!(d[0].file.as_deref(), Some("bad.v"));
        assert_eq!(d[0].line, Some(3));
        assert_eq!(d[0].severity, Severity::Error);
        assert_eq!(d[2].file.as_deref(), Some("rtl/sub.v"));
        assert_eq!(d[4].severity, Severity::Warning);
        assert_eq!(d[5].file, None);
        assert_eq!(d[5].tool.as_deref(), Some("iverilog"));
    }

    // Real output from vvp (oss-cad-suite 2026-10-22).
    #[test]
    fn vvp_testbench_messages() {
        let out = "VCD info: dumpfile out/z.vcd opened for output.
WARNING: z_tb.v:1: careful
         Time: 0  Scope: z_tb
ERROR: z_tb.v:1: bad thing
       Time: 0  Scope: z_tb
ERROR: just text from $display
*** ASSERTION FAILED at z_tb.v:7: expected 1
FATAL: tb/z_tb.v:9: 
       Time: 0  Scope: z_tb
z_tb.v:1: $finish called at 0 (1s)";
        let m = parse_vvp(out, "/src");
        let d: Vec<_> = m.iter().map(|m| m.diagnostic.clone()).collect();
        assert_eq!(d.len(), 4, "{d:#?}");
        // Each $fatal, $error and $warning gets the time of its "Time:" line.
        assert_eq!(m[0].ticks, Some(0));
        assert_eq!(m[2].ticks, None, "Apio's macro lines have no time");
        assert_eq!(d[0].severity, Severity::Warning);
        assert_eq!(d[0].text, "careful");
        assert_eq!(d[1].severity, Severity::Error);
        assert_eq!(d[1].text, "bad thing");
        assert_eq!(d[2].text, "Assertion failed: expected 1");
        assert_eq!(d[2].line, Some(7));
        assert_eq!(d[3].text, "The testbench stopped with $fatal.");
        assert_eq!(d[3].file.as_deref(), Some("tb/z_tb.v"));
        assert_eq!(d[3].tool.as_deref(), Some("vvp"));
    }

    #[test]
    fn apio_assertion_macro_lines() {
        // Real lines from the three kinds of Apio assertion macros.
        let out =
            "*** ASSERTION FAILED in main_tb (clk_num=400): expected (leds == 'h32), actual: 'h31
End of simulation
ERROR at bcd/bcd_tb.v:41: expected count = 101 but got 100
*** ASSERTION FAILED at main_tb.v:52: expected 'h01, actual 'h00
main_tb.v:36: $finish called at 80100000 (1ps)";
        let d: Vec<_> = parse_vvp(out, "/src")
            .into_iter()
            .map(|m| m.diagnostic)
            .collect();
        assert_eq!(d.len(), 3, "{d:#?}");
        assert!(d.iter().all(|d| d.severity == Severity::Error));
        assert_eq!(
            d[0].text,
            "Assertion failed in main_tb (clk_num=400): expected (leds == 'h32), actual: 'h31"
        );
        assert_eq!(d[0].file, None);
        assert_eq!(d[1].text, "expected count = 101 but got 100");
        assert_eq!(d[1].file.as_deref(), Some("bcd/bcd_tb.v"));
        assert_eq!(d[1].line, Some(41));
        assert_eq!(d[2].text, "Assertion failed: expected 'h01, actual 'h00");
        assert_eq!(d[2].line, Some(52));
    }

    // Real output from vvp (oss-cad-suite 2026-10-22), timescale 1ns/1ps.
    #[test]
    fn vvp_times_and_line_kinds() {
        let out = "ERROR: a_tb.v:6: first failure
       Time: 100500  Scope: a_tb
done
FATAL: a_tb.v:9: stop
       Time: 101500  Scope: a_tb";
        let m = parse_vvp(out, "/src");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].ticks, Some(100500));
        assert_eq!(m[1].ticks, Some(101500));

        assert_eq!(
            classify_vvp_line("ERROR: a_tb.v:6: first failure"),
            VvpLine::TaggedFailure
        );
        assert_eq!(
            classify_vvp_line("FATAL: a_tb.v:6: "),
            VvpLine::TaggedFailure
        );
        assert_eq!(
            classify_vvp_line("WARNING: a_tb.v:6: careful"),
            VvpLine::Other
        );
        assert_eq!(
            classify_vvp_line("*** ASSERTION FAILED in main_tb (clk_num=4): expected"),
            VvpLine::MacroFailure
        );
        assert_eq!(
            classify_vvp_line("ERROR at main_tb.v:3: expected 1"),
            VvpLine::MacroFailure
        );
        assert_eq!(classify_vvp_line("ERROR: just text"), VvpLine::Other);
        assert_eq!(classify_vvp_line("done"), VvpLine::Other);
    }

    #[test]
    fn vcd_timescales() {
        let ns = |t: &str| vcd_timescale_ns(std::io::Cursor::new(t.to_string()));
        assert_eq!(ns("$date x $end\n$timescale\n\t1ps\n$end\n"), Some(0.001));
        assert_eq!(ns("$timescale 10 ns $end"), Some(10.0));
        assert_eq!(ns("$timescale 1us $end"), Some(1000.0));
        assert_eq!(ns("$scope module a $end"), None);
    }
}
