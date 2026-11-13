//! Display paths (docs/tool-api.md, Display paths): how a request asks for
//! paths to be named in what people read. Files are named as a whole,
//! folders keep the part of a path below them; files first, then folders,
//! and within each the first match wins. Paths are compared by real
//! location (symbolic links, the same file on disk), falling back to their
//! parts for files that don't exist. After the request's own entries come
//! the request's paths as it gave them: custom board files as given,
//! and the source folder's files relative to it, as requests name source
//! files. Any other path is left as a tool printed it.

use std::path::{Component, Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::ToolError;

/// One file or folder and how to name it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisplayPath {
    /// The file or folder, as an absolute path.
    pub path: String,
    /// How people read it: any text. For a folder, a path inside it keeps
    /// its part below the folder after this and "/", or alone when this is
    /// empty, so "" gives names relative to the folder.
    #[serde(rename = "as")]
    pub as_: String,
}

/// How to name paths in what people read: diagnostics, messages, testbench
/// output, live tool output and diagram labels. Never command lines or
/// fields that are paths.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisplayPaths {
    /// Files named as a whole. They come before folders; the first that
    /// matches wins.
    #[serde(default)]
    pub files: Vec<DisplayPath>,
    /// Folders whose files keep their path below the folder, under the
    /// folder's name `as`. The first that matches wins.
    #[serde(default)]
    pub folders: Vec<DisplayPath>,
}

/// An entry, ready to match: its spellings and its identity on disk.
#[derive(Debug)]
struct Entry {
    /// The path as given and its real path, without `.`, `..` or a
    /// trailing `/`, most specific first, without repeats.
    spellings: Vec<String>,
    id: Option<FileId>,
    as_: String,
}

/// A file's identity on disk, the same for every spelling of it.
type FileId = (u64, u64);

#[cfg(unix)]
fn file_id(path: &Path) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

#[cfg(not(unix))]
fn file_id(_path: &Path) -> Option<FileId> {
    None
}

/// A path without `.`, `..` or a trailing `/` (only its parts, nothing on
/// disk).
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

impl Entry {
    fn new(given: &DisplayPath) -> Entry {
        let path = normal(Path::new(&given.path));
        let mut spellings = vec![path.to_string_lossy().into_owned()];
        if let Ok(real) = crate::paths::resolve(&path) {
            let real = real.to_string_lossy().into_owned();
            if !spellings.contains(&real) {
                spellings.push(real);
            }
        }
        // Longer spellings first, so text finds the whole of one.
        spellings.sort_by_key(|s| std::cmp::Reverse(s.len()));
        Entry {
            spellings,
            id: file_id(&path),
            as_: given.as_.clone(),
        }
    }

    /// Whether `path` (normal, absolute) is this file.
    fn is(&self, path: &Path) -> bool {
        self.spellings.iter().any(|s| Path::new(s) == path)
            || self.id.is_some_and(|id| file_id(path) == Some(id))
    }

    /// The part of `path` (normal, absolute) below this folder, when it's
    /// inside it or is it.
    fn below(&self, path: &Path) -> Option<PathBuf> {
        for s in &self.spellings {
            if let Ok(rest) = path.strip_prefix(s) {
                return Some(rest.to_path_buf());
            }
        }
        let id = self.id?;
        path.ancestors()
            .find(|a| file_id(a) == Some(id))
            .and_then(|a| path.strip_prefix(a).ok())
            .map(Path::to_path_buf)
    }

    /// A path below this folder, `rest`, as people read it.
    fn named(&self, rest: &str) -> String {
        match (self.as_.is_empty(), rest.is_empty()) {
            (true, true) => ".".into(),
            (true, false) => rest.into(),
            (false, true) => self.as_.clone(),
            (false, false) => format!("{}/{rest}", self.as_.trim_end_matches('/')),
        }
    }
}

/// A request's display paths, ready to apply, followed by the request's
/// own paths as it gave them. With no entries it changes nothing.
#[derive(Debug, Default)]
pub struct Display {
    files: Vec<Entry>,
    folders: Vec<Entry>,
    /// The request's source folder: relative paths in diagnostics are
    /// relative to it.
    source_folder: Option<PathBuf>,
}

impl Display {
    /// From a request's display paths, source folder and custom boards
    /// file. An entry whose path isn't absolute is a request error.
    pub fn new(
        given: &DisplayPaths,
        source_folder: Option<&str>,
        custom_boards_folder: Option<&str>,
    ) -> Result<Display, ToolError> {
        for e in given.files.iter().chain(&given.folders) {
            if !Path::new(&e.path).is_absolute() {
                return Err(ToolError::request(format!(
                    "display_paths: {:?} isn't an absolute path.",
                    e.path
                )));
            }
        }
        let absolute = |p: &&str| Path::new(p).is_absolute();
        // The request's paths as it gave them, after its own entries.
        let as_given = |path: &str, as_: &str| {
            Entry::new(&DisplayPath {
                path: path.into(),
                as_: as_.into(),
            })
        };
        let mut files: Vec<Entry> = given.files.iter().map(Entry::new).collect();
        files.extend(
            custom_boards_folder
                .filter(absolute)
                .map(|f| as_given(f, f)),
        );
        let mut folders: Vec<Entry> = given.folders.iter().map(Entry::new).collect();
        folders.extend(source_folder.filter(absolute).map(|s| as_given(s, "")));
        Ok(Display {
            files,
            folders,
            source_folder: source_folder.map(|s| normal(Path::new(s))),
        })
    }

    /// From a tool call's arguments: their `display_paths`,
    /// `source_folder` and `custom_boards_folder`, if any.
    pub fn from_arguments(
        arguments: Option<&serde_json::Map<String, Value>>,
    ) -> Result<Display, ToolError> {
        let Some(args) = arguments else {
            return Ok(Display::default());
        };
        let given: DisplayPaths = match args.get("display_paths") {
            None | Some(Value::Null) => DisplayPaths::default(),
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| ToolError::request(format!("Invalid display_paths: {e}.")))?,
        };
        let text = |key: &str| args.get(key).and_then(Value::as_str);
        Display::new(&given, text("source_folder"), text("custom_boards_folder"))
    }

    fn is_empty(&self) -> bool {
        self.files.is_empty() && self.folders.is_empty()
    }

    /// A file as people read it: `file` is absolute, or relative to the
    /// source folder, as in diagnostics. Unchanged when nothing matches.
    pub fn file(&self, file: &str) -> String {
        if self.is_empty() {
            return file.to_string();
        }
        let path = Path::new(file);
        let abs = if path.is_absolute() {
            normal(path)
        } else {
            match &self.source_folder {
                Some(src) => normal(&src.join(path)),
                None => return file.to_string(),
            }
        };
        if let Some(e) = self.files.iter().find(|e| e.is(&abs)) {
            return e.as_.clone();
        }
        for e in &self.folders {
            if let Some(rest) = e.below(&abs) {
                return e.named(&rest.to_string_lossy());
            }
        }
        file.to_string()
    }

    /// Text with the paths it knows named as people read them. A path is
    /// found as a request or a tool spelled it: one of an entry's
    /// spellings, starting where no path goes on before it, and for a file
    /// ending where no path goes on after it.
    pub fn text(&self, text: &str) -> String {
        if self.is_empty() || !text.contains('/') {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        'scan: while i < text.len() {
            let rest = &text[i..];
            let starts = starts_here(&text[..i]);
            if starts && rest.starts_with('/') {
                for e in &self.files {
                    for s in &e.spellings {
                        if rest.starts_with(s.as_str())
                            && rest[s.len()..].chars().next().is_none_or(|c| !path_char(c))
                        {
                            out.push_str(&e.as_);
                            i += s.len();
                            continue 'scan;
                        }
                    }
                }
                for e in &self.folders {
                    for s in &e.spellings {
                        if !rest.starts_with(s.as_str()) {
                            continue;
                        }
                        match rest[s.len()..].chars().next() {
                            Some('/') => {
                                // What follows is the part below the folder.
                                if !e.as_.is_empty() {
                                    out.push_str(e.as_.trim_end_matches('/'));
                                    out.push('/');
                                }
                                i += s.len() + 1;
                                continue 'scan;
                            }
                            next if next.is_none_or(|c| !path_char(c)) => {
                                out.push_str(&e.named(""));
                                i += s.len();
                                continue 'scan;
                            }
                            _ => {}
                        }
                    }
                }
            }
            let c = rest.chars().next().unwrap_or_default();
            out.push(c);
            i += c.len_utf8();
        }
        out
    }

    /// A tool result or error, as JSON, with the paths people read named:
    /// every diagnostic's `file` and `text`, and `message`, `details` and
    /// `output` text. `commands` and fields that are paths are left as they are.
    pub fn json(&self, value: &mut Value) {
        if self.is_empty() {
            return;
        }
        match value {
            Value::Object(map) => {
                let diagnostic = map.contains_key("severity") && map.contains_key("text");
                for (key, v) in map.iter_mut() {
                    match (key.as_str(), v) {
                        ("commands", _) => {}
                        ("file", Value::String(s)) if diagnostic => *s = self.file(s),
                        ("text", Value::String(s)) if diagnostic => *s = self.text(s),
                        ("message" | "details" | "output", Value::String(s)) => *s = self.text(s),
                        (_, v) => self.json(v),
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|v| self.json(v)),
            _ => {}
        }
    }
}

/// Whether a path can start after `before`: nothing, a character that
/// isn't part of a path, or an escape such as `\n` (as in Graphviz labels).
fn starts_here(before: &str) -> bool {
    let mut back = before.chars().rev();
    match (back.next(), back.next()) {
        (None, _) => true,
        (Some(c), Some('\\')) if c.is_ascii_alphabetic() => true,
        (Some(c), _) => !path_char(c),
    }
}

/// A character that can go on a path, so a known path that it touches is
/// part of a longer one.
fn path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~' | '+')
}

/// A live output sink that names paths in tool and engine lines as the
/// request says. Command lines stay as they are: they must stay rerunnable.
pub struct DisplayOutput {
    pub inner: std::sync::Arc<dyn crate::progress::OutputSink>,
    pub display: std::sync::Arc<Display>,
}

impl crate::progress::OutputSink for DisplayOutput {
    fn line(&self, line: &crate::progress::ToolLine) {
        if self.display.is_empty() {
            return self.inner.line(line);
        }
        let text = self.display.text(line.text);
        let original = line.original.map(|o| self.display.text(o));
        self.inner.line(&crate::progress::ToolLine {
            text: &text,
            original: original.as_deref(),
            ..*line
        });
    }

    fn command(&self, tool: &str, text: &str) {
        self.inner.command(tool, text);
    }

    fn message(&self, text: &str) {
        self.inner.message(&self.display.text(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn display(given: Value, src: Option<&str>) -> Display {
        Display::new(&serde_json::from_value(given).unwrap(), src, None).unwrap()
    }

    #[test]
    fn files_first_then_folders_and_the_first_match_wins() {
        let d = display(
            json!({
                "files": [{"path": "/s/7/custom-boards.json", "as": "/home/you/blinky/custom-boards.json"}],
                "folders": [
                    {"path": "/s/7/lib", "as": "LIB"},
                    {"path": "/s/7", "as": ""},
                    {"path": "/s/7/lib", "as": "never: the folder before matches first"},
                ]
            }),
            Some("/s/7"),
        );
        assert_eq!(
            d.file("/s/7/custom-boards.json"),
            "/home/you/blinky/custom-boards.json"
        );
        assert_eq!(d.file("/s/7/lib/uart.v"), "LIB/uart.v");
        assert_eq!(d.file("/s/7/src/main.v"), "src/main.v");
        // Relative paths are relative to the source folder.
        assert_eq!(d.file("main.v"), "main.v");
        assert_eq!(
            d.file("custom-boards.json"),
            "/home/you/blinky/custom-boards.json"
        );
        // By whole parts: /s/7 isn't a folder of /s/70.
        assert_eq!(d.file("/s/70/main.v"), "/s/70/main.v");
        // No match: as given.
        assert_eq!(d.file("/elsewhere/x.v"), "/elsewhere/x.v");
    }

    #[test]
    fn a_folder_named_by_any_text() {
        let d = display(
            json!({"folders": [{"path": "/s/ex", "as": "<example>/"}]}),
            None,
        );
        assert_eq!(d.file("/s/ex/main.v"), "<example>/main.v");
        assert_eq!(d.file("/s/ex"), "<example>/");
        assert_eq!(d.text("see /s/ex/a b.v:3"), "see <example>/a b.v:3");
    }

    #[test]
    fn paths_in_text() {
        let d = display(
            json!({
                "files": [{"path": "/s/7/custom-boards.json", "as": "/home/you/blinky 3/custom-boards.json"}],
                "folders": [{"path": "/s/7", "as": ""}, {"path": "/opt/tools", "as": "<tools>"}]
            }),
            None,
        );
        assert_eq!(
            d.text("/s/7/main.v:5: ERROR: syntax error"),
            "main.v:5: ERROR: syntax error"
        );
        assert_eq!(
            d.text("Reading '/s/7/main.v' and \"/opt/tools/share/cells.v\"."),
            "Reading 'main.v' and \"<tools>/share/cells.v\"."
        );
        assert_eq!(
            d.text("/s/7/custom-boards.json:4:3: Trailing comma."),
            "/home/you/blinky 3/custom-boards.json:4:3: Trailing comma."
        );
        // The folder itself.
        assert_eq!(d.text("in /s/7 now"), "in . now");
        // Not where a longer path goes on, before or after.
        assert_eq!(
            d.text("/x/s/7/main.v /s/70/main.v"),
            "/x/s/7/main.v /s/70/main.v"
        );
        // A file's name only when the path ends there.
        assert_eq!(
            d.text("/s/7/custom-boards.json.bak"),
            "custom-boards.json.bak"
        );
        assert_eq!(d.text("no paths here"), "no paths here");
        // After an escaped line break, as in Graphviz labels.
        assert_eq!(
            d.text("label=\"PROC $3\\n/s/7/main.v:6.14-6.23\""),
            "label=\"PROC $3\\nmain.v:6.14-6.23\""
        );
    }

    // Unix only: making a symlink on Windows needs Developer Mode or
    // administrator rights.
    #[cfg(unix)]
    #[test]
    fn real_locations_match_any_spelling() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().canonicalize().unwrap();
        std::fs::create_dir(real.join("proj")).unwrap();
        std::fs::write(real.join("proj/main.v"), "").unwrap();
        // A link to the folder, given as the entry; the tools print the real
        // path.
        let link = real.join("link");
        std::os::unix::fs::symlink(real.join("proj"), &link).unwrap();
        let d = display(
            json!({"folders": [{"path": link.to_string_lossy(), "as": ""}]}),
            None,
        );
        let printed = real.join("proj/main.v").to_string_lossy().into_owned();
        assert_eq!(d.file(&printed), "main.v");
        assert_eq!(d.text(&format!("{printed}:3: oops")), "main.v:3: oops");
        // And the other way: the entry real, the path through the link.
        let d = display(
            json!({"files": [{"path": real.join("proj/main.v").to_string_lossy(), "as": "MAIN"}]}),
            None,
        );
        assert_eq!(d.file(&link.join("main.v").to_string_lossy()), "MAIN");
        // `.` and `..` don't matter.
        assert_eq!(
            d.file(&format!("{}/proj/../proj/./main.v", real.display())),
            "MAIN"
        );
    }

    #[test]
    fn json_names_paths_people_read_only() {
        let d = display(
            json!({"folders": [{"path": "/s/7", "as": ""}]}),
            Some("/s/7"),
        );
        let mut v = json!({
            "message": "Build failed. /s/7/main.v:5: syntax error",
            "diagnostics": [{"file": "/s/7/main.v", "line": 5, "severity": "error", "text": "in /s/7/inc.vh"}],
            "testbenches": [{"file": "main_tb.v", "output": "read /s/7/data.hex", "waveform_file": "/s/7/w.vcd"}],
            "bitstream": "/s/7/_build/hardware.bin",
            "commands": [{"line": "yosys /s/7/main.v", "message": "/s/7"}]
        });
        d.json(&mut v);
        assert_eq!(
            v,
            json!({
                "message": "Build failed. main.v:5: syntax error",
                "diagnostics": [{"file": "main.v", "line": 5, "severity": "error", "text": "in inc.vh"}],
                "testbenches": [{"file": "main_tb.v", "output": "read data.hex", "waveform_file": "/s/7/w.vcd"}],
                "bitstream": "/s/7/_build/hardware.bin",
                "commands": [{"line": "yosys /s/7/main.v", "message": "/s/7"}]
            })
        );
    }

    #[test]
    fn the_requests_own_paths_read_as_it_gave_them() {
        // No display paths: source files relative to the source folder, as
        // requests name them; custom board files as given, even inside
        // the source folder; other paths as printed.
        let d = Display::new(
            &DisplayPaths::default(),
            Some("/p/blinky"),
            Some("/p/blinky/custom-boards.json"),
        )
        .unwrap();
        assert_eq!(d.file("/p/blinky/main.v"), "main.v");
        assert_eq!(d.file("main.v"), "main.v");
        assert_eq!(
            d.file("/p/blinky/custom-boards.json"),
            "/p/blinky/custom-boards.json"
        );
        assert_eq!(
            d.text("/p/blinky/main.v:5.14-5.25 and /opt/share/cells.v"),
            "main.v:5.14-5.25 and /opt/share/cells.v"
        );
        // The request's own entries come first.
        let d = Display::new(
            &serde_json::from_value(json!({"folders": [{"path": "/p/blinky", "as": "B"}]}))
                .unwrap(),
            Some("/p/blinky"),
            None,
        )
        .unwrap();
        assert_eq!(d.file("main.v"), "B/main.v");
    }

    #[test]
    fn live_output_names_paths_but_not_command_lines() {
        use crate::progress::{OutputLevel, OutputSink, ToolLine};
        use std::sync::{Arc, Mutex};
        #[derive(Default)]
        struct Rec(Mutex<Vec<String>>);
        impl OutputSink for Rec {
            fn line(&self, l: &ToolLine) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("{} | {:?}", l.text, l.original));
            }
            fn command(&self, _tool: &str, text: &str) {
                self.0.lock().unwrap().push(format!("$ {text}"));
            }
            fn message(&self, text: &str) {
                self.0.lock().unwrap().push(format!("msg {text}"));
            }
        }
        let rec = Arc::new(Rec::default());
        let out = DisplayOutput {
            inner: rec.clone(),
            display: Arc::new(display(
                json!({"folders": [{"path": "/s/7", "as": ""}]}),
                None,
            )),
        };
        out.command("yosys", "yosys /s/7/main.v");
        out.line(&ToolLine {
            tool: "yosys",
            level: OutputLevel::Error,
            text: "/s/7/main.v:5: ERROR: x",
            redraw: false,
            original: Some("Info: /s/7/main.v:5: ERROR: x"),
        });
        out.message("Programming /s/7/b.bin");
        assert_eq!(
            *rec.0.lock().unwrap(),
            [
                "$ yosys /s/7/main.v",
                "main.v:5: ERROR: x | Some(\"Info: main.v:5: ERROR: x\")",
                "msg Programming b.bin",
            ]
        );
    }

    #[test]
    fn requests_need_absolute_paths() {
        let e = Display::new(
            &serde_json::from_value(json!({"folders": [{"path": "rel", "as": ""}]})).unwrap(),
            None,
            None,
        )
        .unwrap_err();
        assert!(
            e.message.contains("isn't an absolute path"),
            "{}",
            e.message
        );
        let args = json!({"display_paths": {"files": [{"path": "/a", "as": "b", "x": 1}]}});
        assert!(Display::from_arguments(args.as_object()).is_err());
        assert!(
            Display::from_arguments(json!({}).as_object())
                .unwrap()
                .is_empty()
        );
    }
}
