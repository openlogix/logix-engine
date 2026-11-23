//! Output rules: everything the engine knows about how particular tools
//! print, in one table (docs/architecture.md, "Output rules"). A rule names
//! the tools it applies to, a pattern on the line as the tool printed it,
//! and what to do with a matching line:
//!
//! - change how the line is shown live (hide it, change its level, remove
//!   noise around a message, add a note from the engine after it), or
//! - read progress from it (a phase starts, or the phase's percentage).
//!
//! Rules affect only the live output (`logix/output`) and progress. Results
//! and diagnostics come from the raw output, which rules never touch. Rules
//! are internal: they may change in any version, and hosts only see their
//! effect. Each rule has real lines it must match and nearby lines it must
//! not, checked by a test, with where the lines come from in `reason`.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use regex::Regex;

use crate::progress::{OutputLevel, OutputSink, ToolLine};

/// What a rule does with a line that matches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Leaves the line out of the live output (it's still logged).
    Hide,
    /// Shows the line at this level instead of the one it reads as.
    Level(OutputLevel),
    /// Replaces the pattern's match with this text, to remove noise around
    /// a message, never to change what it says. The host also gets the
    /// line as printed.
    Rewrite(&'static str),
    /// Adds a line from the engine after the line, once per tool run.
    Note(&'static str),
    /// A progress phase with this message starts, such as "Verifying".
    Phase(&'static str),
    /// The pattern's first group is the current phase's percentage.
    Percent,
}

/// One output rule.
#[derive(Debug)]
pub struct Rule {
    /// A short name, in the log when the rule hides a line.
    pub id: &'static str,
    /// The programs it applies to, by file name, such as `iceprog`. Empty
    /// for any program.
    pub tools: &'static [&'static str],
    /// A regex on the line as the tool printed it.
    pub pattern: &'static str,
    pub action: Action,
    /// Why the rule exists and where its lines come from.
    pub reason: &'static str,
    /// Real lines from these tools that the rule must match.
    pub matches: &'static [&'static str],
    /// Nearby lines it must not match.
    pub misses: &'static [&'static str],
}

const NEXTPNR: &[&str] = &[
    "nextpnr-ice40",
    "nextpnr-ecp5",
    "nextpnr-himbaechel",
    "nextpnr-xilinx",
];

/// The rules, in order: rewrites apply in this order.
pub const RULES: &[Rule] = &[
    Rule {
        id: "blank-line",
        tools: &[],
        pattern: r"^\s+$",
        action: Action::Hide,
        reason: "A line of only spaces says nothing; progress bars print one to erase the line before redrawing it (iceprog.c prints 22 spaces and a carriage return before each address).",
        matches: &["                      ", "\t "],
        misses: &["  x", "    Info: x"],
    },
    Rule {
        id: "gowin-pack-speedups",
        tools: &["gowin_pack"],
        pattern: r"(Numpy|Msgspec|fastcrc) is not available, performance will be degraded",
        action: Action::Hide,
        reason: "apycula's optional speedups aren't in the toolchain package; gowin_pack works without them (Python prints each warning and the line that raised it; Apio hides them too, YosysHQ/oss-cad-suite-build#194).",
        matches: &[
            "/pkg/lib/python3.11/site-packages/apycula/bitmatrix.py:60: UserWarning: Numpy is not available, performance will be degraded.",
            "  warnings.warn(\"Numpy is not available, performance will be degraded.\")",
            "/pkg/lib/python3.11/site-packages/apycula/chipdb.py:261: UserWarning: Msgspec is not available, performance will be degraded.",
            "  warnings.warn(\"Msgspec is not available, performance will be degraded.\")",
            "/pkg/lib/python3.11/site-packages/apycula/crc16.py:19: UserWarning: fastcrc is not available, performance will be degraded.",
        ],
        misses: &[
            "gowin_pack: error: the following arguments are required: netlist",
            "/pkg/apycula/gowin_pack.py:12: UserWarning: something else",
        ],
    },
    Rule {
        id: "nextpnr-info-prefix",
        tools: NEXTPNR,
        pattern: r"^Info: ",
        action: Action::Rewrite(""),
        reason: "nextpnr starts nearly every line of its log with \"Info: \", which says nothing about the message (Apio removes it too).",
        matches: &[
            "Info: Max frequency for clock 'clk': 194.36 MHz (PASS at 12.00 MHz)",
            "Info: Program finished normally.",
        ],
        misses: &[
            "Warning: unmatched constraint 'LED7' (on line 9)",
            "ERROR: IO 'LED0' is unconstrained in PCF (override this error with --pcf-allow-unconstrained)",
            "    Info: indented",
        ],
    },
    Rule {
        id: "openfpgaloader-gowin-no-verify",
        tools: &["openFPGALoader"],
        pattern: r"^writing verification not supported$",
        action: Action::Note(
            "openFPGALoader can't read a Gowin flash back to verify it. The CRC check that follows tells whether the FPGA loaded the design.",
        ),
        reason: "openFPGALoader 1.2.0 src/gowin.cpp prints it (as a warning, in color) for every Gowin flash write with --verify, then reloads the FPGA and runs checkCRC (\"CRC check: Success\").",
        matches: &["writing verification not supported"],
        misses: &["CRC check: Success", "Verifying write (May take time)"],
    },
    Rule {
        id: "openfpgaloader-verify",
        tools: &["openFPGALoader"],
        pattern: r"^Verifying write",
        action: Action::Phase("Verifying"),
        reason: "openFPGALoader 1.2.0 src/spiFlash.cpp prints it before reading the flash back (a \"Reading\" progress bar).",
        matches: &["Verifying write (May take time)"],
        misses: &[
            "Writing: [==>                 ] 10.00%",
            "Reading: [==>      ] 10.00%",
        ],
    },
    Rule {
        id: "progress-bar-percent",
        tools: &["openFPGALoader", "dfu-util"],
        pattern: r"\]\s*(\d{1,3}(?:\.\d+)?)%",
        action: Action::Percent,
        reason: "The programmers' progress bars end in \"] NN%\": openFPGALoader 1.2.0 src/progressBar.cpp (\"] %3.2f%%\") and dfu-util's (\"[%s] %3llu%% %12llu bytes\").",
        matches: &[
            "Writing: [====>     ] 45.33%",
            "Reading: [==================================================] 100.00%",
            "Download\t[=========                ]  36%        49152 bytes",
        ],
        misses: &["Open file DONE", "Erasing: 100%"],
    },
    Rule {
        id: "iceprog-verify",
        tools: &["iceprog"],
        pattern: r"^reading\.\.$",
        action: Action::Phase("Verifying"),
        reason: "icestorm iceprog/iceprog.c prints it before reading the flash back to compare it.",
        matches: &["reading.."],
        misses: &["programming..", "cdone: high", "reading flash ID.."],
    },
    Rule {
        id: "iceprog-percent",
        tools: &["iceprog"],
        pattern: r"^addr 0x[0-9A-Fa-f]+\s+(\d{1,3})%$",
        action: Action::Percent,
        reason: "icestorm iceprog/iceprog.c prints \"addr 0x%06X %3ld%%\" while writing and reading back.",
        matches: &["addr 0x000000   0%", "addr 0x020F00  99%"],
        misses: &["file size: 135100", "erase 64kB sector at 0x010000.."],
    },
];

fn compiled() -> &'static [Regex] {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        RULES
            .iter()
            .map(|r| Regex::new(r.pattern).expect("a rule's pattern"))
            .collect()
    })
}

/// The rules for `tool` that match `text`, in order, with their regexes.
fn matching<'a>(
    tool: &'a str,
    text: &'a str,
) -> impl Iterator<Item = (&'static Rule, &'static Regex)> + 'a {
    RULES
        .iter()
        .zip(compiled())
        .filter(move |(r, re)| (r.tools.is_empty() || r.tools.contains(&tool)) && re.is_match(text))
}

/// What a line says about progress.
#[derive(Debug, Default, PartialEq)]
pub struct Progress {
    /// A phase that starts with this line.
    pub phase: Option<&'static str>,
    /// The current phase's percentage.
    pub percent: Option<f64>,
}

/// Reads progress from a tool's line.
pub fn progress(tool: &str, text: &str) -> Progress {
    let mut p = Progress::default();
    for (rule, re) in matching(tool, text) {
        match rule.action {
            Action::Phase(name) => p.phase = p.phase.or(Some(name)),
            Action::Percent => {
                if let Some(v) = re
                    .captures(text)
                    .and_then(|c| c.get(1)?.as_str().parse().ok())
                {
                    p.percent = p.percent.or(Some(v));
                }
            }
            _ => {}
        }
    }
    p
}

/// How a line is shown live.
#[derive(Debug, PartialEq)]
pub enum Shown {
    Hidden(&'static str),
    Line {
        level: OutputLevel,
        /// The text shown, after the rewrites.
        text: String,
        notes: Vec<&'static str>,
    },
}

/// How the rules show a tool's line.
pub fn shown(tool: &str, text: &str) -> Shown {
    let mut level = OutputLevel::of(text);
    let mut shown = text.to_string();
    let mut notes = Vec::new();
    for (rule, re) in matching(tool, text) {
        match rule.action {
            Action::Hide => return Shown::Hidden(rule.id),
            Action::Level(l) => level = l,
            Action::Rewrite(with) => shown = re.replace(&shown, with).into_owned(),
            Action::Note(note) => notes.push(note),
            Action::Phase(_) | Action::Percent => {}
        }
    }
    if shown.trim().is_empty() {
        return Shown::Hidden("blank-line");
    }
    Shown::Line {
        level,
        text: shown,
        notes,
    }
}

/// One tool run's live output: each line goes to the sink as the rules
/// show it, with each note once per run.
pub struct ToolOutput<'a> {
    tool: &'a str,
    sink: &'a dyn OutputSink,
    noted: Mutex<HashSet<&'static str>>,
}

impl<'a> ToolOutput<'a> {
    pub fn new(tool: &'a str, sink: &'a dyn OutputSink) -> Self {
        ToolOutput {
            tool,
            sink,
            noted: Mutex::new(HashSet::new()),
        }
    }

    /// Shows a line the tool printed (`redraw`: the tool prints over it)
    /// and returns what it says about progress.
    pub fn line(&self, text: &str, redraw: bool) -> Progress {
        match shown(self.tool, text) {
            Shown::Hidden(id) => log::info!("Hid a {} line (rule {id}): {text:?}", self.tool),
            Shown::Line {
                level,
                text: shown,
                notes,
            } => {
                self.sink.line(&ToolLine {
                    tool: self.tool,
                    level,
                    text: &shown,
                    redraw,
                    original: (shown != text).then_some(text),
                    location: None,
                });
                for note in notes {
                    if self.noted.lock().unwrap().insert(note) {
                        self.sink.message(note);
                    }
                }
            }
        }
        progress(self.tool, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_matches_its_lines_and_misses_the_others() {
        for (rule, re) in RULES.iter().zip(compiled()) {
            assert!(
                !rule.matches.is_empty() && !rule.misses.is_empty(),
                "{}: needs samples",
                rule.id
            );
            assert!(!rule.reason.is_empty(), "{}", rule.id);
            for line in rule.matches {
                assert!(re.is_match(line), "{} should match {line:?}", rule.id);
            }
            for line in rule.misses {
                assert!(!re.is_match(line), "{} shouldn't match {line:?}", rule.id);
            }
        }
    }

    #[test]
    fn rules_apply_only_to_their_tools() {
        for rule in RULES.iter().filter(|r| !r.tools.is_empty()) {
            for line in rule.matches {
                let fired = matching("some-other-tool", line).any(|(r, _)| r.id == rule.id);
                assert!(!fired, "{} fired for another tool on {line:?}", rule.id);
            }
        }
        // iceprog's verify line doesn't start a phase for openFPGALoader.
        assert_eq!(progress("openFPGALoader", "reading.."), Progress::default());
    }

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<_> = RULES.iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), RULES.len());
    }

    #[test]
    fn programmer_progress() {
        let p = |tool, line| progress(tool, line);
        assert_eq!(
            p("openFPGALoader", "Writing: [====>     ] 45.33%").percent,
            Some(45.33)
        );
        assert_eq!(
            p("openFPGALoader", "Verifying write (May take time)"),
            Progress {
                phase: Some("Verifying"),
                percent: None
            }
        );
        assert_eq!(p("iceprog", "reading..").phase, Some("Verifying"));
        assert_eq!(p("iceprog", "addr 0x020F00  99%").percent, Some(99.0));
        assert_eq!(
            p(
                "dfu-util",
                "Download\t[=========                ]  36%        49152 bytes"
            )
            .percent,
            Some(36.0)
        );
        assert_eq!(p("iceprog", "cdone: high"), Progress::default());
    }

    #[test]
    fn shown_lines() {
        assert_eq!(
            shown("iceprog", "                      "),
            Shown::Hidden("blank-line")
        );
        assert_eq!(
            shown(
                "gowin_pack",
                "  warnings.warn(\"Numpy is not available, performance will be degraded.\")"
            ),
            Shown::Hidden("gowin-pack-speedups")
        );
        // The level comes from the line as printed, before the rewrite.
        assert_eq!(
            shown(
                "nextpnr-ice40",
                "Info: Max frequency for clock 'clk': 194.36 MHz"
            ),
            Shown::Line {
                level: OutputLevel::Info,
                text: "Max frequency for clock 'clk': 194.36 MHz".into(),
                notes: vec![]
            }
        );
        // Another tool's "Info: " stays.
        assert_eq!(
            shown("yosys", "Info: x"),
            Shown::Line {
                level: OutputLevel::Info,
                text: "Info: x".into(),
                notes: vec![]
            }
        );
        assert!(matches!(shown("nextpnr-ecp5", "Info: "), Shown::Hidden(_)));
    }

    #[derive(Default)]
    struct Sink(Mutex<Vec<String>>);
    impl OutputSink for Sink {
        fn line(&self, l: &ToolLine) {
            self.0.lock().unwrap().push(format!(
                "{} {}{}",
                l.tool,
                l.text,
                l.original
                    .map(|o| format!(" (was {o:?})"))
                    .unwrap_or_default()
            ));
        }
        fn message(&self, text: &str) {
            self.0.lock().unwrap().push(format!("engine {text}"));
        }
    }

    #[test]
    fn a_tool_run_shows_lines_with_notes_once() {
        let sink = Sink::default();
        let gowin = ToolOutput::new("openFPGALoader", &sink);
        gowin.line("writing verification not supported", false);
        gowin.line("writing verification not supported", false);
        gowin.line("CRC check: Success", false);
        let pnr = ToolOutput::new("nextpnr-ice40", &sink);
        pnr.line("Info: Program finished normally.", false);
        pnr.line("   ", true);
        let got = sink.0.lock().unwrap().clone();
        assert_eq!(
            got,
            [
                "openFPGALoader writing verification not supported",
                "engine openFPGALoader can't read a Gowin flash back to verify it. The CRC check that follows tells whether the FPGA loaded the design.",
                "openFPGALoader writing verification not supported",
                "openFPGALoader CRC check: Success",
                "nextpnr-ice40 Program finished normally. (was \"Info: Program finished normally.\")",
            ]
        );
    }
}
