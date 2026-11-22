//! The open_waveform tool (docs/tool-api.md, "open_waveform tool"): Open
//! Logix's waveform viewer on any waveform file a client names, such as
//! another simulator's, an earlier run's, or one an agent made (R3),
//! optionally at a time range and with chosen signals (R8).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::errors::ToolError;
use crate::state::EngineState;
use crate::wave;

pub struct OpenWaveform;

impl EngineTool for OpenWaveform {
    const NAME: &'static str = "open_waveform";
    const DESCRIPTION: &'static str = "Opens Open Logix's waveform viewer on a waveform \
        file (.vcd), such as one from another simulator, an earlier run, or one you made, \
        in a window of its own (Logix Waveforms) for the person at the screen; the viewer \
        saves the person's view as they change it (view_file). The waveform is only read. \
        To point the person at something, name the signals to show (paths as in the \
        file, such as main_tb.dut.count) and a time range in nanoseconds; signals the file \
        doesn't have come back in unknown_signals. To simulate a testbench and open its \
        waveform, use simulate instead, which also marks its failures.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = OpenWaveformInput;
    type Output = OpenWaveformOutput;
}

fn default_true() -> bool {
    true
}

/// The waveform to open, and what to show first.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenWaveformInput {
    /// The waveform file (.vcd), as an absolute path. It is only read.
    pub waveform_file: String,
    /// Where the viewer's page goes, as an absolute path to a build folder:
    /// one of its last two folder names must contain "build", such as
    /// /home/user/proj/_build. When omitted, the page goes into the engine's
    /// own folder, which keeps the last 20.
    #[serde(default)]
    pub build_folder: Option<String>,
    /// The view (.view) to open with and save to, as an absolute path, such
    /// as one the viewer saved for a testbench. Its folder must exist; the
    /// file may not yet. When omitted, the view goes next to the page, as
    /// <waveform>.view.
    #[serde(default)]
    pub view_file: Option<String>,
    /// Let the viewer save the person's view (to view_file) as they change
    /// it. Defaults to true. False opens the view without writing anything.
    #[serde(default = "default_true")]
    pub save_view: bool,
    /// The signals to show, in this order, as paths in the file, such as
    /// ["main_tb.clk", "main_tb.dut.count"]. Omitted means the view's, or
    /// the file's top-level signals.
    #[serde(default)]
    pub signals: Option<Vec<String>>,
    /// The start of the time range to show, in nanoseconds. Omitted means
    /// the start of the run, or the view's.
    #[serde(default)]
    pub from_ns: Option<f64>,
    /// The end of the time range to show, in nanoseconds. Omitted means the
    /// end of the run, or the view's.
    #[serde(default)]
    pub to_ns: Option<f64>,
    /// Where to put the cursor, in nanoseconds, so the viewer's Value
    /// column shows every signal's value there.
    #[serde(default)]
    pub cursor_ns: Option<f64>,
    /// Open the page in a window of its own (Logix Waveforms) for the person
    /// at the screen. Defaults to true. False only writes it, such as for a
    /// host that shows the page itself.
    #[serde(default = "default_true")]
    pub open: bool,
}

/// The viewer's page.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenWaveformOutput {
    /// The viewer's page, as an absolute path: self-contained, with the
    /// waveform in it.
    pub page_file: String,
    /// True if the page opened in a window.
    pub opened: bool,
    /// Why the page didn't open, in plain words for the person at the
    /// screen, such as a missing library; only when open was true
    /// and it didn't open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_problem: Option<String>,
    /// How many signals the file has.
    pub signal_count: usize,
    /// The run's length, in nanoseconds.
    pub end_ns: f64,
    /// Requested signals the file doesn't have, which the viewer leaves out.
    pub unknown_signals: Vec<String>,
}

/// How many pages the engine's own folder keeps.
const KEEP_PAGES: usize = 20;

/// The engine folder's waveform pages, without the oldest beyond
/// KEEP_PAGES.
fn engine_pages(state: &EngineState) -> Result<PathBuf, ToolError> {
    let dir = state.engine_folder_resolved.join("waveforms");
    std::fs::create_dir_all(&dir)
        .map_err(|e| ToolError::engine(format!("Can't create {}: {e}", dir.display())))?;
    let mut pages: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "html"))
                .map(|e| {
                    let time = e
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    (time, e.path())
                })
                .collect()
        })
        .unwrap_or_default();
    pages.sort();
    let extra = (pages.len() + 1).saturating_sub(KEEP_PAGES);
    for (_, p) in pages.into_iter().take(extra) {
        let _ = std::fs::remove_file(p);
    }
    Ok(dir)
}

/// Checks an absolute path to an existing file with an extension.
fn check_file(field: &str, path: &str, extension: &str) -> Result<PathBuf, ToolError> {
    let req = |m: String| ToolError::request(format!("{field}: {m}."));
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(req(format!("{path:?} must be an absolute path")));
    }
    if !p
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case(extension))
    {
        return Err(req(format!("{path:?} must end with .{extension}")));
    }
    if !p.is_file() {
        return Err(req(format!("{path:?} can't be found")));
    }
    Ok(dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
}

/// Checks a view file's path: absolute, ending in .view, in a folder that
/// exists. The file itself may not exist yet.
fn check_view_path(path: &str) -> Result<PathBuf, ToolError> {
    let req = |m: String| ToolError::request(format!("view_file: {m}."));
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(req(format!("{path:?} must be an absolute path")));
    }
    if !p
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case(wave::VIEW_EXTENSION))
    {
        return Err(req(format!(
            "{path:?} must end with .{}",
            wave::VIEW_EXTENSION
        )));
    }
    match p.parent() {
        Some(dir) if dir.is_dir() => Ok(p.to_path_buf()),
        _ => Err(req(format!("{path:?} is in a folder that doesn't exist"))),
    }
}

pub fn run(state: &EngineState, input: OpenWaveformInput) -> Result<OpenWaveformOutput, ToolError> {
    let vcd = check_file("waveform_file", &input.waveform_file, "vcd")?;
    let view_file = match &input.view_file {
        Some(v) => Some(check_view_path(v)?),
        None => None,
    };
    for (field, value) in [
        ("from_ns", input.from_ns),
        ("to_ns", input.to_ns),
        ("cursor_ns", input.cursor_ns),
    ] {
        if value.is_some_and(|v| !v.is_finite() || v < 0.0) {
            return Err(ToolError::request(format!(
                "{field} must be a time of 0 or more nanoseconds."
            )));
        }
    }
    if let (Some(a), Some(b)) = (input.from_ns, input.to_ns)
        && a >= b
    {
        return Err(ToolError::request("from_ns must be before to_ns."));
    }
    let folder = match &input.build_folder {
        Some(b) => {
            let real =
                crate::paths::check_build_folder(b, crate::paths::home_folder().as_deref(), None)
                    .map_err(|e| ToolError::request(format!("build_folder: {e}.")))?;
            std::fs::create_dir_all(&real)
                .map_err(|e| ToolError::engine(format!("Can't create {}: {e}", real.display())))?;
            real
        }
        None => engine_pages(state)?,
    };

    let (text, truncated) = wave::read_waveform(&vcd, wave::MAX_WAVEFORM_BYTES)?;
    let (all, end_ns) = wave::vcd_signals(&text);
    let unknown_signals: Vec<String> = input
        .signals
        .iter()
        .flatten()
        .filter(|s| !all.contains(s))
        .cloned()
        .collect();
    let stem = vcd
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "waveform".into());
    let waveform = vcd
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let page = folder.join(wave::page_name(&stem));
    let view_path = view_file
        .clone()
        .unwrap_or_else(|| folder.join(format!("{stem}.{}", wave::VIEW_EXTENSION)));
    let view = wave::read_view(&view_path);
    let data = wave::PageData {
        waveform: waveform.clone(),
        testbench: waveform,
        view_file: view_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        vcd: text,
        truncated,
        order: Vec::new(),
        declarations: Default::default(),
        failures: Vec::new(),
        failure_count: 0,
        view,
        focus: Some(wave::Focus {
            signals: input.signals,
            from_ns: input.from_ns,
            to_ns: input.to_ns,
            cursor_ns: input.cursor_ns,
        }),
    };
    wave::write_page(&page, &data)?;
    let viewer_problem = input.open.then(|| {
        let name = crate::viewer::app_name(&state.packages, crate::viewer::WAVEFORMS);
        let mut declared = std::collections::BTreeMap::new();
        if input.save_view {
            declared.insert("view".to_string(), view_path.clone());
        }
        crate::viewer::present(
            &state.packages,
            &crate::viewer::Page {
                kind: crate::viewer::WAVEFORMS,
                page: &page,
                title: crate::viewer::title(&name, &[&stem]),
                declared,
                source_folder: None,
            },
        )
    });
    let opened = input.open && viewer_problem.as_ref().is_some_and(Option::is_none);
    Ok(OpenWaveformOutput {
        page_file: page.to_string_lossy().into_owned(),
        opened,
        viewer_problem: viewer_problem.flatten(),
        signal_count: all.len(),
        end_ns,
        unknown_signals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_file_may_not_exist_yet_but_its_folder_must() {
        let tmp = tempfile::tempdir().unwrap();
        let new = tmp.path().join("main_tb.view");
        assert_eq!(check_view_path(new.to_str().unwrap()).unwrap(), new);
        for (bad, needle) in [
            ("main_tb.view".to_string(), "absolute"),
            (
                tmp.path().join("main_tb.txt").display().to_string(),
                ".view",
            ),
            (
                tmp.path().join("nope/main_tb.view").display().to_string(),
                "doesn't exist",
            ),
        ] {
            let err = check_view_path(&bad).unwrap_err();
            assert!(err.message.contains(needle), "{bad}: {}", err.message);
        }
    }

    #[test]
    fn files_must_be_absolute_and_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.vcd");
        std::fs::write(&f, "").unwrap();
        assert!(check_file("waveform_file", f.to_str().unwrap(), "vcd").is_ok());
        // A path that's absolute on every system, and isn't there.
        let missing = tmp.path().join("missing.vcd");
        for (bad, needle) in [
            ("a.vcd", "absolute"),
            (missing.to_str().unwrap(), "can't be found"),
        ] {
            let e = check_file("waveform_file", bad, "vcd").unwrap_err();
            assert!(e.message.contains(needle), "{}", e.message);
        }
        let e = check_file("view_file", f.to_str().unwrap(), "view").unwrap_err();
        assert!(e.message.contains("must end with .view"), "{}", e.message);
    }
}
