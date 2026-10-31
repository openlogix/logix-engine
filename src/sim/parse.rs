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
}
