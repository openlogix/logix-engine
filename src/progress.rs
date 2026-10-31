//! Progress by phase, from docs/tool-api.md ("Progress"). An operation
//! decides its phases when it starts. Total is 100 times the number of
//! phases, and progress is 100 times the phase index plus the phase's
//! percentage, so it only increases, as MCP requires.

use std::sync::{Arc, Mutex};

/// Where progress goes, such as MCP progress notifications.
pub trait ProgressSink: Send + Sync {
    fn report(&self, progress: f64, total: f64, message: Option<String>);
}

/// A sink that drops progress, for hosts that didn't ask for it.
pub struct NoProgress;

impl ProgressSink for NoProgress {
    fn report(&self, _progress: f64, _total: f64, _message: Option<String>) {}
}

struct State {
    /// The current phase, 0-based. None before the first phase.
    index: Option<usize>,
    /// The last whole percentage reported in the current phase.
    last_percent: u32,
}

/// How a tool output line reads: an error, a warning, or anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputLevel {
    Error,
    Warning,
    Info,
}

impl OutputLevel {
    /// Classifies a line the way the tools mark their messages: "ERROR:",
    /// "Error:" and "Warning:" (yosys, nextpnr, the programmers, iverilog),
    /// "%Error-CODE:" and "%Warning-CODE:" (Verilator), and "FATAL:" (a
    /// testbench's $fatal, from vvp).
    pub fn of(line: &str) -> OutputLevel {
        let l = line.to_lowercase();
        if l.contains("error:")
            || l.starts_with("error ")
            || l.starts_with("%error")
            || l.starts_with("fatal:")
        {
            OutputLevel::Error
        } else if l.contains("warning:") || l.starts_with("%warning") {
            OutputLevel::Warning
        } else {
            OutputLevel::Info
        }
    }
}

/// Where live tool output goes, line by line.
pub trait OutputSink: Send + Sync {
    /// A line the tool printed.
    fn line(&self, tool: &str, level: OutputLevel, text: &str);
    /// The command line of a tool about to run, before its output.
    fn command(&self, _tool: &str, _text: &str) {}
}

/// A sink that drops output, for hosts that didn't ask for it.
pub struct NoOutput;

impl OutputSink for NoOutput {
    fn line(&self, _tool: &str, _level: OutputLevel, _text: &str) {}
}

/// Reports an operation's progress phase by phase. Shared between tasks.
pub struct Phases {
    sink: Arc<dyn ProgressSink>,
    count: usize,
    state: Mutex<State>,
}

impl Phases {
    /// An operation with `count` phases.
    pub fn new(sink: Arc<dyn ProgressSink>, count: usize) -> Self {
        Phases {
            sink,
            count: count.max(1),
            state: Mutex::new(State {
                index: None,
                last_percent: 0,
            }),
        }
    }

    fn total(&self) -> f64 {
        100.0 * self.count as f64
    }

    /// Starts the next phase and sends its message, such as "Synthesizing".
    pub fn start(&self, message: impl Into<String>) {
        let mut s = self.state.lock().unwrap();
        let index = s.index.map_or(0, |i| (i + 1).min(self.count - 1));
        s.index = Some(index);
        s.last_percent = 0;
        self.sink
            .report(100.0 * index as f64, self.total(), Some(message.into()));
    }

    /// Reports the current phase's percentage, 0 to 100. Sends only when the
    /// whole percentage goes up, so progress never goes back and isn't sent
    /// too often. A phase stops at 99, so the next phase's start is always
    /// higher, as MCP requires each value to be.
    pub fn percent(&self, percent: f64) {
        let mut s = self.state.lock().unwrap();
        let Some(index) = s.index else { return };
        let whole = percent.clamp(0.0, 99.0).floor() as u32;
        if whole <= s.last_percent {
            return;
        }
        s.last_percent = whole;
        self.sink
            .report(100.0 * index as f64 + whole as f64, self.total(), None);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A sink that records what it gets, for tests.
    #[derive(Default)]
    pub struct Recorder(pub Mutex<Vec<(f64, f64, Option<String>)>>);

    impl ProgressSink for Recorder {
        fn report(&self, progress: f64, total: f64, message: Option<String>) {
            self.0.lock().unwrap().push((progress, total, message));
        }
    }

    #[test]
    fn output_levels() {
        assert_eq!(
            OutputLevel::of("main.v:3: ERROR: syntax error"),
            OutputLevel::Error
        );
        assert_eq!(
            OutputLevel::of("Error: Failed to claim cable"),
            OutputLevel::Error
        );
        assert_eq!(
            OutputLevel::of("main.v:2: Warning: Identifier `x' is implicitly declared."),
            OutputLevel::Warning
        );
        assert_eq!(
            OutputLevel::of("Info: Max frequency for clock 'clk': 194.36 MHz"),
            OutputLevel::Info
        );
        assert_eq!(OutputLevel::of("0 warnings, 0 errors"), OutputLevel::Info);
        assert_eq!(
            OutputLevel::of("%Warning-WIDTHTRUNC: main.v:6:9: Operator ASSIGNDLY expects 4 bits"),
            OutputLevel::Warning
        );
        assert_eq!(
            OutputLevel::of(
                "%Error-MODMISSING: m.v:1:11: Cannot find file containing module: 'foo'"
            ),
            OutputLevel::Error
        );
        assert_eq!(
            OutputLevel::of("FATAL: main_tb.v:10: led is  x"),
            OutputLevel::Error
        );
    }

    #[test]
    fn progress_increases_across_phases() {
        let rec = Arc::new(Recorder::default());
        let phases = Phases::new(rec.clone(), 2);
        phases.percent(10.0); // before any phase: ignored
        phases.start("Installing x");
        phases.percent(10.4);
        phases.percent(10.9); // same whole percent: not sent
        phases.percent(5.0); // lower: not sent
        phases.percent(150.0);
        phases.start("Synthesizing");
        phases.percent(50.0);
        let got = rec.0.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![
                (0.0, 200.0, Some("Installing x".into())),
                (10.0, 200.0, None),
                (99.0, 200.0, None),
                (100.0, 200.0, Some("Synthesizing".into())),
                (150.0, 200.0, None),
            ]
        );
    }
}
