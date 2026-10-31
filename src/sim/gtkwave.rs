//! GTKWave layouts (.gtkw) and launching GTKWave for simulate.
//!
//! A layout is a project file the user changes by saving from GTKWave, so it
//! usually lives in the source folder. When it is missing, or is still a
//! default one the engine or Apio generated, the engine writes a default with
//! the testbench's top-level signals, so GTKWave doesn't open empty. A layout
//! the user saved has no marker and is never changed.

use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::errors::{ActionCode, Category, ToolError};

/// Marks a layout the engine generated.
pub const MARKER: &str = "THIS FILE WAS GENERATED AUTOMATICALLY BY LOGIX ENGINE";
/// Marks a layout Apio generated, treated the same way.
pub const APIO_MARKER: &str = "THIS FILE WAS GENERATED AUTOMATICALLY BY APIO";

/// True if the layout at `path` was generated, by the engine or by Apio, so
/// it may be replaced. A file that can't be read counts as the user's.
pub fn is_generated(path: &Path) -> bool {
    let Ok(text) = fs::read(path) else {
        return false;
    };
    let text = String::from_utf8_lossy(&text);
    text.contains(MARKER) || text.contains(APIO_MARKER)
}

/// The top-level signals in a VCD file's header, as GTKWave names them: the
/// scope and the signal name, with the bit range except for integers,
/// parameters, reals and strings, such as "main_tb.data[7:0]" and
/// "main_tb.i". Sorted without regard to case, as Apio does.
pub fn top_level_signals(vcd: impl BufRead) -> Vec<String> {
    let mut signals = Vec::new();
    let mut depth = 0usize;
    let mut scope = String::new();
    let mut tokens = vcd
        .lines()
        .map_while(Result::ok)
        .flat_map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>());
    // The words of a section up to its $end.
    fn until_end(tokens: &mut dyn Iterator<Item = String>) -> Vec<String> {
        tokens.take_while(|t| t != "$end").collect()
    }
    while let Some(t) = tokens.next() {
        match t.as_str() {
            "$enddefinitions" => break,
            "$scope" => {
                let words = until_end(&mut tokens);
                depth += 1;
                if depth == 1 {
                    scope = words.get(1).cloned().unwrap_or_default();
                }
            }
            "$upscope" => {
                until_end(&mut tokens);
                depth = depth.saturating_sub(1);
            }
            "$var" => {
                let words = until_end(&mut tokens);
                if depth != 1 || words.len() < 4 {
                    continue;
                }
                let kind = words[0].as_str();
                let mut name = words[3..].concat();
                if matches!(kind, "integer" | "parameter" | "real" | "string")
                    && let Some(i) = name.rfind('[')
                    && name.ends_with(']')
                {
                    name.truncate(i);
                }
                signals.push(format!("{scope}.{name}"));
            }
            s if s.starts_with('$') => {
                until_end(&mut tokens);
            }
            _ => {}
        }
    }
    signals.sort_by_key(|s| s.to_lowercase());
    signals
}

/// The wave area's width GTKWave has in its default window, in pixels,
/// used to fit the zoom to the whole run.
const WAVE_WIDTH_PX: f64 = 900.0;

/// GTKWave's zoom that fits `span` waveform time units into the wave area,
/// as its own zoom-to-fit computes it: -log2(span / width * 200), at most 0.
pub fn fit_zoom(span: u64) -> f64 {
    let span = span.max(1) as f64;
    (-(span / WAVE_WIDTH_PX * 200.0).log2()).min(0.0)
}

/// The last timestamp in a VCD file, in its time units: the end of the run.
/// Reads only the end of the file, since waveforms can be large.
pub fn vcd_end_ticks(vcd: &Path) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(vcd).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(64 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    f.read_to_end(&mut tail).ok()?;
    String::from_utf8_lossy(&tail)
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix('#')?.trim().parse().ok())
}

/// The default layout for a testbench: a header with the marker, the zoom
/// and GTKWave's primary marker, then the top-level signals.
///
/// GTKWave reads "*<zoom> <marker>" as the zoom and the primary marker's
/// time, in the waveform's time units; -1 means no marker. The zoom fits
/// the whole run, `end_ticks` long, so the view is right even when GTKWave's
/// own zoom-to-fit at startup doesn't apply. The marker is at the first
/// failure, when there is one.
pub fn default_layout(
    testbench: &str,
    signals: &[String],
    end_ticks: Option<u64>,
    failure_ticks: Option<u64>,
) -> String {
    let mut text = format!(
        "[*] GTKWave layout for the testbench {testbench}\n\
         [*] {MARKER}.\n\
         [*] It is replaced on each simulation until you save your own\n\
         [*] layout from GTKWave, which is then kept.\n\
         [*]\n"
    );
    if failure_ticks.is_some() {
        text.push_str("[*] The marker is at the first failure.\n");
    }
    if end_ticks.is_some() || failure_ticks.is_some() {
        let zoom = fit_zoom(end_ticks.unwrap_or(0).max(failure_ticks.unwrap_or(0)));
        let marker = failure_ticks.map_or("-1".to_string(), |t| t.to_string());
        text.push_str(&format!("*{zoom:.6} {marker}\n"));
    }
    for s in signals {
        text.push_str(s);
        text.push('\n');
    }
    text
}

/// Writes the default layout to `layout` unless the user saved one there.
/// Returns true if it wrote one.
pub fn write_default_if_needed(
    layout: &Path,
    vcd: &Path,
    testbench: &str,
    failure_ticks: Option<u64>,
) -> Result<bool, ToolError> {
    if layout.exists() && !is_generated(layout) {
        log::info!("Keeping the user's layout {}", layout.display());
        return Ok(false);
    }
    let file = fs::File::open(vcd).map_err(|e| {
        ToolError::new(
            Category::Tool,
            format!("Can't read the waveform {}: {e}", vcd.display()),
        )
    })?;
    let signals = top_level_signals(BufReader::new(file));
    fs::write(
        layout,
        default_layout(testbench, &signals, vcd_end_ticks(vcd), failure_ticks),
    )
    .map_err(|e| {
        ToolError::setup(format!(
            "Can't write the layout file {}: {e}",
            layout.display()
        ))
    })?;
    log::info!(
        "Wrote the default layout {} ({} signals)",
        layout.display(),
        signals.len()
    );
    Ok(true)
}

/// GTKWave's arguments, as Apio gives them: zoom to fit, the waveform and
/// the layout.
pub fn gtkwave_args(vcd: &Path, layout: &Path) -> Vec<OsString> {
    vec![
        "--rcvar=do_initial_zoom_fit 1".into(),
        vcd.into(),
        layout.into(),
    ]
}

/// Deletes the layout at `path` if it is still a generated default, so
/// layouts the user never saved don't pile up in the project. A layout the
/// user saved has no marker and is kept.
pub fn delete_if_generated(path: &Path) {
    if !is_generated(path) {
        log::info!("GTKWave closed; keeping the layout {}", path.display());
        return;
    }
    match fs::remove_file(path) {
        Ok(()) => log::info!(
            "GTKWave closed; deleted the unsaved default layout {}",
            path.display()
        ),
        Err(e) => log::info!("Can't delete the default layout {}: {e}", path.display()),
    }
}

/// Starts `program` fully detached and returns at once: its own session, no
/// input or output, and not stopped when the operation is cancelled or the
/// engine exits. The environment is the same minimal one the tools get, plus
/// what a window needs on Linux. `on_exit` runs when the program exits, if
/// the engine is still running then.
pub fn launch_detached(
    program: &Path,
    args: &[OsString],
    cwd: &Path,
    bin_dir: &Path,
    on_exit: impl FnOnce() + Send + 'static,
) -> Result<(), ToolError> {
    let path = crate::commands::tool_path(bin_dir);
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for var in [
        "HOME",
        "TMPDIR",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XAUTHORITY",
    ] {
        if let Some(v) = std::env::var_os(var) {
            cmd.env(var, v);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid only changes the child's session, before exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    log::info!(
        "Starting {} {} (detached, in {})",
        program.display(),
        args.iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
        cwd.display()
    );
    let mut child = cmd.spawn().map_err(|e| {
        ToolError::new(
            Category::Tool,
            format!("Can't start {}: {e}", program.display()),
        )
        .with_action(
            ActionCode::InstallPackages,
            format!(
                "Reinstall the {} package, which may be damaged.",
                super::TOOLCHAIN
            ),
        )
    })?;
    // Reap it when it exits, so it doesn't linger as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
        on_exit();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::time::{Duration, Instant};

    // A real VCD header from iverilog (oss-cad-suite 2026-10-22).
    const VCD: &str = "$date
	Sat Oct 31 00:02:47 2026
$end
$version
	Icarus Verilog
$end
$timescale
	1ps
$end
$scope module multi_tb $end
$var wire 4 ! led [3:0] $end
$var parameter 32 \" P $end
$var reg 1 # clk $end
$var reg 8 $ data [7:0] $end
$var integer 32 % i [31:0] $end
$var real 1 & r $end
$scope module dut $end
$var wire 1 # CLK $end
$var reg 4 ' LED [3:0] $end
$upscope $end
$upscope $end
$enddefinitions $end
#0
$dumpvars
bx $
$end
";

    #[test]
    fn signals_match_apios_default_layout() {
        // Apio's create_gtkwave_file gives these for the same VCD.
        assert_eq!(
            top_level_signals(Cursor::new(VCD)),
            [
                "multi_tb.clk",
                "multi_tb.data[7:0]",
                "multi_tb.i",
                "multi_tb.led[3:0]",
                "multi_tb.P",
                "multi_tb.r",
            ]
        );
    }

    #[test]
    fn default_layouts_are_replaced_but_user_layouts_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let vcd = tmp.path().join("main_tb.vcd");
        fs::write(&vcd, VCD).unwrap();
        let layout = tmp.path().join("main_tb.gtkw");

        assert!(write_default_if_needed(&layout, &vcd, "main_tb.v", None).unwrap());
        let text = fs::read_to_string(&layout).unwrap();
        assert!(text.contains(MARKER));
        assert!(text.contains("main_tb.v"));
        assert!(text.ends_with("multi_tb.r\n"), "{text}");

        // A generated one, by the engine or by Apio, is replaced.
        assert!(write_default_if_needed(&layout, &vcd, "main_tb.v", None).unwrap());
        fs::write(&layout, format!("[*] {APIO_MARKER}.\nold\n")).unwrap();
        assert!(write_default_if_needed(&layout, &vcd, "main_tb.v", None).unwrap());
        assert!(!fs::read_to_string(&layout).unwrap().contains("old"));

        // One the user saved is kept.
        fs::write(&layout, "[*] saved by the user\nmulti_tb.clk\n").unwrap();
        assert!(!write_default_if_needed(&layout, &vcd, "main_tb.v", None).unwrap());
        assert_eq!(
            fs::read_to_string(&layout).unwrap(),
            "[*] saved by the user\nmulti_tb.clk\n"
        );
    }

    #[test]
    fn detached_programs_outlive_the_call() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("ran");
        let args: Vec<OsString> = vec![
            "-c".into(),
            format!("sleep 1; echo \"$PATH\" > '{}'", marker.display()).into(),
        ];
        let start = Instant::now();
        launch_detached(
            Path::new("/bin/sh"),
            &args,
            tmp.path(),
            Path::new("/pkg/bin"),
            || {},
        )
        .unwrap();
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "returns at once"
        );
        assert!(!marker.exists());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(100));
        let path = fs::read_to_string(&marker).unwrap();
        assert!(path.starts_with("/pkg/bin:"), "{path}");

        let err = launch_detached(
            Path::new("/no/such/gtkwave"),
            &[],
            tmp.path(),
            tmp.path(),
            || {},
        )
        .unwrap_err();
        assert_eq!(err.category, Category::Tool);
    }

    #[test]
    fn unsaved_default_layouts_are_deleted_when_the_viewer_exits() {
        let tmp = tempfile::tempdir().unwrap();
        let vcd = tmp.path().join("main_tb.vcd");
        fs::write(&vcd, VCD).unwrap();
        let default = tmp.path().join("default.gtkw");
        let saved = tmp.path().join("saved.gtkw");
        write_default_if_needed(&default, &vcd, "main_tb.v", None).unwrap();
        write_default_if_needed(&saved, &vcd, "main_tb.v", None).unwrap();
        // The user saves one of them from GTKWave, which drops the marker.
        fs::write(&saved, "[*] saved by the user\n").unwrap();

        // A viewer that exits after a moment.
        let (tx, rx) = std::sync::mpsc::channel();
        let paths = [default.clone(), saved.clone()];
        let args: Vec<OsString> = vec!["-c".into(), "sleep 0.3".into()];
        launch_detached(
            Path::new("/bin/sh"),
            &args,
            tmp.path(),
            tmp.path(),
            move || {
                paths.iter().for_each(|p| delete_if_generated(p));
                tx.send(()).unwrap();
            },
        )
        .unwrap();
        assert!(default.exists(), "kept while the viewer runs");
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(!default.exists());
        assert!(saved.exists());
        delete_if_generated(&tmp.path().join("missing.gtkw"));
    }

    #[test]
    fn default_layouts_fit_the_run_and_mark_the_first_failure() {
        let signals = vec!["main_tb.clk".to_string()];
        let text = default_layout("main_tb.v", &signals, Some(3_196_000), Some(6000));
        // 3196 ns in ps over 900 px: -log2(3196000 / 900 * 200) = -19.44.
        assert!(text.contains("\n*-19.437"), "{text}");
        assert!(text.ends_with(" 6000\nmain_tb.clk\n"), "{text}");
        let text = default_layout("main_tb.v", &signals, Some(3_196_000), None);
        assert!(text.contains(" -1\nmain_tb.clk\n"), "no marker: {text}");
        assert!(!default_layout("main_tb.v", &signals, None, None).contains("\n*"));
        // Matches a layout GTKWave saved for a 6.5 us run in ps, about -20.4.
        assert!((fit_zoom(6_500_000) + 20.5).abs() < 0.2);
        assert_eq!(fit_zoom(1), 0.0);
    }

    #[test]
    fn the_end_of_a_vcd_file() {
        let tmp = tempfile::tempdir().unwrap();
        let vcd = tmp.path().join("a.vcd");
        fs::write(&vcd, format!("{VCD}#5000\n1#\n#101500\nb0 !\n")).unwrap();
        assert_eq!(vcd_end_ticks(&vcd), Some(101500));
        assert_eq!(vcd_end_ticks(&tmp.path().join("none.vcd")), None);
    }
}
