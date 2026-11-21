//! The waveform viewer (docs/requirements.md, "Waveform viewer"; docs/tool-api.md,
//! "The waveform viewer"): a page the engine writes next to a simulation's
//! waveform, with the waveform in it, so it opens anywhere, offline, and
//! outlives the engine. Hosts show it in a window of their own; other
//! clients open it in the default web browser.
//!
//! The page itself is `viewer.html`; this module fills in its data: the
//! waveform, the testbench's own signals in the order it declares them (the
//! simulator writes them in its own order), where each is declared, the
//! testbench's failures, and the view the user saved (`<testbench>.view`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::errors::{Diagnostic, Severity, ToolError};
use crate::paths;

/// The page's template.
const VIEWER: &str = include_str!("viewer.html");

/// The extension of a saved view.
pub const VIEW_EXTENSION: &str = "view";

/// The most of a waveform file the page holds. A larger one is cut at a
/// timestamp, and the page says so (R30). The page holds the waveform as
/// text, which a browser reads quickly up to tens of megabytes.
pub const MAX_WAVEFORM_BYTES: u64 = 50 * 1024 * 1024;

/// The page's file name for a testbench's build stem, in the build folder.
pub fn page_name(stem: &str) -> String {
    format!("{stem}.html")
}

/// A waveform cut to fit the page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Truncated {
    /// The waveform file's size, in megabytes.
    pub size_mb: u64,
    /// The most the page holds, in megabytes.
    pub limit_mb: u64,
}

/// Where a signal is declared.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Declaration {
    /// The file, relative to the source folder.
    pub file: String,
    pub line: u32,
}

/// A testbench failure, as the page marks it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Failure {
    pub time_ns: f64,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

/// Everything the page shows, written into it as JSON.
#[derive(Debug, Clone, Serialize)]
pub struct PageData {
    /// The waveform's file name.
    pub waveform: String,
    /// The testbench, relative to the source folder.
    pub testbench: String,
    /// The name Save view gives the downloaded view, such as main_tb.view.
    pub view_file: String,
    /// The waveform file's text (VCD), maybe cut (see truncated).
    pub vcd: String,
    pub truncated: Option<Truncated>,
    /// The testbench's own signals, as paths such as main_tb.clk, in the
    /// order it declares them.
    pub order: Vec<String>,
    /// Where each of those signals is declared.
    pub declarations: BTreeMap<String, Declaration>,
    /// The failures the result lists, first first.
    pub failures: Vec<Failure>,
    /// How many failures the testbench reported, more than failures lists
    /// when there were many.
    pub failure_count: u64,
    /// The view the user saved, when there is one.
    pub view: Option<Value>,
    /// What a client asked the viewer to show first (R8), over the view.
    pub focus: Option<Focus>,
}

/// What a client asks the viewer to show first: these signals, this time
/// range, the cursor here (R8). Each part is optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Focus {
    pub signals: Option<Vec<String>>,
    pub from_ns: Option<f64>,
    pub to_ns: Option<f64>,
    pub cursor_ns: Option<f64>,
}

/// Reads a waveform for the page: the whole file, or, when it's larger than
/// `limit`, its start up to the last timestamp that fits.
pub fn read_waveform(vcd: &Path, limit: u64) -> Result<(String, Option<Truncated>), ToolError> {
    use std::io::Read;
    let cant = |e: std::io::Error| {
        ToolError::engine(format!("Can't read the waveform {}: {e}", vcd.display()))
    };
    let size = std::fs::metadata(vcd).map_err(cant)?.len();
    let mut bytes = Vec::new();
    std::fs::File::open(vcd)
        .map_err(cant)?
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(cant)?;
    if size <= limit {
        return Ok((String::from_utf8_lossy(&bytes).into_owned(), None));
    }
    // Cut before the last timestamp that fits, so every value kept is whole.
    let cut = bytes
        .windows(2)
        .rposition(|w| w == b"\n#")
        .map(|i| i + 1)
        .unwrap_or(bytes.len());
    bytes.truncate(cut);
    let mb = |b: u64| b.div_ceil(1024 * 1024);
    Ok((
        String::from_utf8_lossy(&bytes).into_owned(),
        Some(Truncated {
            size_mb: mb(size),
            limit_mb: mb(limit),
        }),
    ))
}

/// The name of a VCD file's first scope, the testbench's module.
pub fn top_scope(vcd: &str) -> Option<String> {
    let mut words = vcd.split_whitespace();
    while let Some(w) = words.next() {
        if w == "$enddefinitions" {
            return None;
        }
        if w == "$scope" {
            words.next()?;
            return words.next().map(str::to_string);
        }
    }
    None
}

/// A VCD file's signals, as the viewer names them: the scopes and the
/// signal's name, such as main_tb.dut.count; and the run's end in
/// nanoseconds.
pub fn vcd_signals(vcd: &str) -> (Vec<String>, f64) {
    let mut words = vcd.split_whitespace();
    let (mut scopes, mut signals) = (Vec::<String>::new(), Vec::new());
    let mut unit_s = 1e-9;
    while let Some(w) = words.next() {
        match w {
            "$enddefinitions" => break,
            "$timescale" => {
                let text: String = words.by_ref().take_while(|w| *w != "$end").collect();
                unit_s = timescale_seconds(&text).unwrap_or(1e-9);
            }
            "$scope" => {
                let section: Vec<&str> = words.by_ref().take_while(|w| *w != "$end").collect();
                scopes.push(section.get(1).copied().unwrap_or_default().to_string());
            }
            "$upscope" => {
                words.by_ref().take_while(|w| *w != "$end").for_each(drop);
                scopes.pop();
            }
            "$var" => {
                let section: Vec<&str> = words.by_ref().take_while(|w| *w != "$end").collect();
                if let Some(name) = section.get(3) {
                    let path = scopes
                        .iter()
                        .map(String::as_str)
                        .chain([*name])
                        .collect::<Vec<_>>()
                        .join(".");
                    if !signals.contains(&path) {
                        signals.push(path);
                    }
                }
            }
            _ => {}
        }
    }
    let end = vcd
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix('#')?.trim().parse::<u64>().ok())
        .unwrap_or(0);
    (signals, end as f64 * unit_s * 1e9)
}

/// A VCD $timescale, such as "1ps" or "10 ns", in seconds.
fn timescale_seconds(text: &str) -> Option<f64> {
    let text = text.trim();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let unit = text[digits.len()..].trim();
    let scale = match unit {
        "s" => 1.0,
        "ms" => 1e-3,
        "us" => 1e-6,
        "ns" => 1e-9,
        "ps" => 1e-12,
        "fs" => 1e-15,
        _ => return None,
    };
    Some(digits.parse::<f64>().ok()? * scale)
}

/// The keywords that start a signal declaration in a module's body.
const DECLARATIONS: &[&str] = &[
    "reg", "wire", "logic", "integer", "bit", "int", "real", "realtime", "time", "byte",
    "shortint", "longint", "tri", "tri0", "tri1", "triand", "trior", "wand", "wor", "uwire",
    "supply0", "supply1", "var",
];
/// Words in a declaration that aren't its names.
const MODIFIERS: &[&str] = &[
    "signed",
    "unsigned",
    "vectored",
    "scalared",
    "automatic",
    "static",
    "const",
];
/// Words that open and close blocks: declarations inside them aren't the
/// module's own signals (or the simulator puts them in scopes of their own).
const OPENERS: &[&str] = &[
    "begin",
    "fork",
    "task",
    "function",
    "case",
    "casez",
    "casex",
    "generate",
    "specify",
    "class",
    "covergroup",
    "property",
    "sequence",
    "clocking",
    "interface",
];
const CLOSERS: &[&str] = &[
    "end",
    "join",
    "join_any",
    "join_none",
    "endtask",
    "endfunction",
    "endcase",
    "endgenerate",
    "endspecify",
    "endclass",
    "endgroup",
    "endproperty",
    "endsequence",
    "endclocking",
    "endinterface",
];

/// A word or a punctuation mark of Verilog source, with its line.
struct Token<'a> {
    text: &'a str,
    line: u32,
}

/// The source's words, numbers and punctuation, without comments and
/// strings.
fn tokens(src: &str) -> Vec<Token<'_>> {
    let bytes = src.as_bytes();
    let (mut i, mut line, mut out) = (0, 1u32, Vec::new());
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line += 1;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if src[i..].starts_with("/*") {
            let end = src[i + 2..]
                .find("*/")
                .map_or(bytes.len(), |e| i + 2 + e + 2);
            line += src[i..end].matches('\n').count() as u32;
            i = end;
        } else if c == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\n' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
        } else if c == b'\\' {
            // An escaped identifier runs to the next white space.
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(Token {
                text: &src[start..i],
                line,
            });
        } else if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'`' || c == b'\'' {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            out.push(Token {
                text: &src[start..i],
                line,
            });
        } else {
            let n = src[i..].chars().next().map_or(1, char::len_utf8);
            out.push(Token {
                text: &src[i..i + n],
                line,
            });
            i += n;
        }
    }
    out
}

fn is_identifier(t: &str) -> bool {
    let first = t.as_bytes().first().copied().unwrap_or(b'0');
    (first.is_ascii_alphabetic() || first == b'_' || first == b'\\')
        && !DECLARATIONS.contains(&t)
        && !MODIFIERS.contains(&t)
}

/// The signals a module declares in its own body, in order, each with the
/// line of its name: what the page shows first (R9), and where a
/// double-click on a name leads (R27). Only the module's own level counts:
/// declarations inside tasks, functions and blocks are left out. The module
/// is `module` when given, else the source's first.
pub fn declared_signals(src: &str, module: Option<&str>) -> Vec<(String, u32)> {
    let toks = tokens(src);
    let mut i = 0;
    // The module's start.
    loop {
        let Some(t) = toks.get(i) else {
            return Vec::new();
        };
        if t.text == "module" && module.is_none_or(|m| toks.get(i + 1).is_some_and(|n| n.text == m))
        {
            break;
        }
        i += 1;
    }
    // Skip the header, up to its ';' outside parentheses.
    let mut depth = 0i32;
    while let Some(t) = toks.get(i) {
        match t.text {
            "(" => depth += 1,
            ")" => depth -= 1,
            ";" if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    i += 1;
    let mut found: Vec<(String, u32)> = Vec::new();
    let mut blocks = 0i32;
    while let Some(t) = toks.get(i) {
        if t.text == "endmodule" {
            break;
        }
        if OPENERS.contains(&t.text) {
            blocks += 1;
        } else if CLOSERS.contains(&t.text) {
            blocks -= 1;
        } else if blocks == 0 && DECLARATIONS.contains(&t.text) {
            i = declaration(&toks, i + 1, &mut found);
            continue;
        } else if blocks == 0 && matches!(t.text, "parameter" | "localparam") {
            while toks.get(i).is_some_and(|t| t.text != ";") {
                i += 1;
            }
        }
        i += 1;
    }
    let mut seen = std::collections::HashSet::new();
    found.retain(|(n, _)| seen.insert(n.clone()));
    found
}

/// Reads a declaration's names from `i` to its ';', and returns where
/// reading goes on.
fn declaration(toks: &[Token], mut i: usize, found: &mut Vec<(String, u32)>) -> usize {
    let mut depth = 0i32;
    let mut expect_name = true;
    while let Some(t) = toks.get(i) {
        match t.text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            ";" if depth <= 0 => return i + 1,
            "," if depth == 0 => expect_name = true,
            "=" if depth == 0 => expect_name = false,
            "#" if depth == 0 => {}
            w if depth == 0 && expect_name && is_identifier(w) => {
                found.push((w.to_string(), t.line));
                expect_name = false;
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// The page's data from a testbench's run.
pub struct RunInfo<'a> {
    pub source_folder: &'a Path,
    pub testbench: &'a str,
    /// The waveform's file name.
    pub waveform: &'a str,
    pub vcd: String,
    pub truncated: Option<Truncated>,
    pub diagnostics: &'a [Diagnostic],
    pub failure_count: u64,
    /// The saved view's name and contents.
    pub view_file: String,
    pub view: Option<Value>,
}

impl RunInfo<'_> {
    pub fn page_data(self) -> PageData {
        let top = top_scope(&self.vcd);
        let source =
            std::fs::read_to_string(self.source_folder.join(self.testbench)).unwrap_or_default();
        let (mut order, mut declarations) = (Vec::new(), BTreeMap::new());
        if let Some(top) = &top {
            for (name, line) in declared_signals(&source, Some(top)) {
                let path = format!("{top}.{name}");
                declarations.insert(
                    path.clone(),
                    Declaration {
                        file: self.testbench.to_string(),
                        line,
                    },
                );
                order.push(path);
            }
        }
        let failures = self
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .filter_map(|d| {
                Some(Failure {
                    time_ns: d.time_ns?,
                    message: d.text.clone(),
                    file: d.file.clone(),
                    line: d.line,
                })
            })
            .collect();
        PageData {
            waveform: self.waveform.to_string(),
            testbench: self.testbench.to_string(),
            view_file: self.view_file,
            vcd: self.vcd,
            truncated: self.truncated,
            order,
            declarations,
            failures,
            failure_count: self.failure_count,
            view: self.view,
            focus: None,
        }
    }
}

/// The page with its data.
pub fn page(data: &PageData) -> String {
    // The data goes into a <script>: "</" can't appear in it, or a signal
    // name or message could end the script.
    let json = serde_json::to_string(data)
        .expect("page data serializes")
        .replace("</", "<\\/");
    VIEWER.replace("/*__DATA__*/null", &json)
}

/// Writes the page.
pub fn write_page(path: &Path, data: &PageData) -> Result<(), ToolError> {
    std::fs::write(path, page(data)).map_err(|e| {
        ToolError::engine(format!(
            "Can't write the waveform viewer {}: {e}",
            path.display()
        ))
    })
}

/// Checks the view file: a relative path ending in .view, in a folder that
/// exists inside the source folder. The file itself may not exist yet.
/// Returns its real path.
pub fn check_view_file(src: &Path, rel: &str) -> Result<PathBuf, ToolError> {
    let req = |m: String| ToolError::request(format!("view_file: {m}."));
    paths::check_relative_path(rel).map_err(req)?;
    if !rel.to_lowercase().ends_with(".view") {
        return Err(req(format!("{rel:?} must end with .view")));
    }
    let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    let dir = if parent.is_empty() {
        src.to_path_buf()
    } else {
        let dir = paths::resolve_inside(src, parent).map_err(req)?;
        if !dir.is_dir() {
            return Err(req(format!("{parent:?} isn't a folder")));
        }
        dir
    };
    let path = dir.join(name);
    if path.symlink_metadata().is_ok() {
        let real = paths::resolve_inside(src, rel).map_err(req)?;
        if !real.is_file() {
            return Err(req(format!("{rel:?} isn't a file")));
        }
        return Ok(real);
    }
    Ok(path)
}

/// A saved view: the file's JSON object, or none when it's missing or
/// isn't one (the page then shows the default view; the file is never
/// changed, R24).
pub fn read_view(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if v.is_object() => Some(v),
        _ => {
            log::info!(
                "{} isn't a view file: the viewer opens its default view",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TB: &str = r#"// A testbench.
module main_tb;
  localparam N = 4;  // not a signal
  reg clk = 1'b0;
  reg [7:0] data = 8'd0, other;
  wire done;  /* a comment with reg fake; */
  wire [3:0] count = data[3:0];
  integer i, errors = 0;
  main dut (.clk(clk), .done(done));
  task check(input [7:0] b);
    reg [7:0] got;
    begin got = b; end
  endtask
  initial begin
    integer j;
    $display("reg not_a_signal;");
  end
  reg late;
endmodule

module other_tb;
  reg x;
endmodule
"#;

    #[test]
    fn the_testbench_declares_its_signals_in_order() {
        let names: Vec<(String, u32)> = declared_signals(TB, Some("main_tb"));
        assert_eq!(
            names,
            [
                ("clk", 4),
                ("data", 5),
                ("other", 5),
                ("done", 6),
                ("count", 7),
                ("i", 8),
                ("errors", 8),
                ("late", 18)
            ]
            .map(|(n, l)| (n.to_string(), l))
        );
        assert_eq!(
            declared_signals(TB, Some("other_tb")),
            [("x".to_string(), 22)]
        );
        assert_eq!(declared_signals(TB, None)[0].0, "clk");
        assert!(declared_signals(TB, Some("missing")).is_empty());
    }

    #[test]
    fn ansi_ports_and_escaped_names() {
        let src = "module t (input wire a, output reg b);\n  wire \\odd.name ;\n  logic signed [3:0] s;\nendmodule\n";
        assert_eq!(
            declared_signals(src, Some("t")),
            [("\\odd.name".to_string(), 2), ("s".to_string(), 3)]
        );
    }

    #[test]
    fn a_vcd_files_signals_and_length() {
        let vcd = "$timescale 10ns $end\n$scope module tb $end\n$var wire 1 ! clk $end\n$scope module dut $end\n$var reg 4 \" count [3:0] $end\n$var wire 1 ! clk $end\n$upscope $end\n$upscope $end\n$enddefinitions $end\n#0\n#25\n";
        let (signals, end_ns) = vcd_signals(vcd);
        assert_eq!(signals, ["tb.clk", "tb.dut.count", "tb.dut.clk"]);
        assert_eq!(end_ns, 250.0);
        assert_eq!(timescale_seconds("1 ps"), Some(1e-12));
        assert_eq!(timescale_seconds("x"), None);
    }

    #[test]
    fn the_top_scope_names_the_testbench() {
        let vcd = "$timescale 1ps $end\n$scope module uart_tb $end\n$var wire 1 ! clk $end\n$upscope $end\n$enddefinitions $end\n#0\n";
        assert_eq!(top_scope(vcd).as_deref(), Some("uart_tb"));
        assert_eq!(top_scope("$enddefinitions $end"), None);
    }

    #[test]
    fn a_large_waveform_is_cut_at_a_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.vcd");
        let text = "$enddefinitions $end\n#0\n1!\n#10\n0!\n#20\n1!\n#30\n0!\n";
        std::fs::write(&f, text).unwrap();
        let (all, cut) = read_waveform(&f, 1000).unwrap();
        assert_eq!((all.as_str(), cut), (text, None));
        let (part, cut) = read_waveform(&f, 40).unwrap();
        assert_eq!(part, "$enddefinitions $end\n#0\n1!\n#10\n0!\n");
        assert_eq!(
            cut,
            Some(Truncated {
                size_mb: 1,
                limit_mb: 1
            })
        );
    }

    #[test]
    fn the_page_holds_its_data_safely() {
        let data = PageData {
            waveform: "main_tb.vcd".into(),
            testbench: "main_tb.v".into(),
            view_file: "main_tb.view".into(),
            vcd: "$scope module </script><b> $end".into(),
            truncated: None,
            order: vec!["main_tb.clk".into()],
            declarations: BTreeMap::new(),
            failures: vec![Failure {
                time_ns: 12.5,
                message: "got 3, expected 4".into(),
                file: Some("main_tb.v".into()),
                line: Some(40),
            }],
            failure_count: 1,
            view: None,
            focus: None,
        };
        let html = page(&data);
        assert!(!html.contains("/*__DATA__*/"));
        assert!(!html.contains("</script><b>"));
        assert!(html.contains(r#""time_ns":12.5"#));
        // The page needs no network (R28).
        assert!(!html.contains("http://") && !html.contains("https://"));
    }

    #[test]
    fn failures_come_from_the_errors_with_a_time() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("t_tb.v"),
            "module t_tb;\n  reg a;\nendmodule\n",
        )
        .unwrap();
        let d = |sev, time: Option<f64>| Diagnostic {
            file: Some("t_tb.v".into()),
            line: Some(9),
            column: None,
            severity: sev,
            text: "bad".into(),
            tool: None,
            time_ns: time,
        };
        let diags = [
            d(Severity::Warning, Some(1.0)),
            d(Severity::Error, None),
            d(Severity::Error, Some(20.0)),
        ];
        let data = RunInfo {
            source_folder: tmp.path(),
            testbench: "t_tb.v",
            waveform: "t_tb.vcd",
            vcd: "$scope module t_tb $end\n$enddefinitions $end\n".into(),
            truncated: None,
            diagnostics: &diags,
            failure_count: 1,
            view_file: "t_tb.view".into(),
            view: None,
        }
        .page_data();
        assert_eq!(data.failures.len(), 1);
        assert_eq!(data.failures[0].time_ns, 20.0);
        assert_eq!(data.order, ["t_tb.a"]);
        assert_eq!(
            data.declarations["t_tb.a"],
            Declaration {
                file: "t_tb.v".into(),
                line: 2
            }
        );
    }

    #[test]
    fn view_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = dunce::canonicalize(tmp.path()).unwrap();
        assert_eq!(
            check_view_file(&src, "main_tb.view").unwrap(),
            src.join("main_tb.view")
        );
        for (bad, needle) in [
            ("main_tb.gtkw", "must end with .view"),
            ("../x.view", "view_file"),
        ] {
            let err = check_view_file(&src, bad).unwrap_err();
            assert!(err.message.contains(needle), "{bad}: {}", err.message);
        }
        let f = src.join("a.view");
        assert_eq!(read_view(&f), None);
        std::fs::write(&f, "not json").unwrap();
        assert_eq!(read_view(&f), None);
        std::fs::write(&f, r#"{"version": 1, "rows": []}"#).unwrap();
        assert_eq!(read_view(&f).unwrap()["version"], 1);
    }
}
