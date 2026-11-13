//! Custom boards (docs/tool-api.md, Board specification): every tool that
//! deals with boards takes `custom_boards_folder`, the absolute path of a
//! folder of board files in Logix Catalog's format, one board each, named
//! `<id>.json` (such as a project's `boards/` folder). The engine reads only
//! the folder the request names: every `*.json` file in it, other files
//! ignored. It reports every problem in them as a diagnostic at file, line
//! and column, as it reports problems in source files.

use std::collections::BTreeSet;
use std::path::Path;

use crate::board::{CustomBoard, CustomBoards};
use crate::catalog::{self, PROGRAMMERS, STANDARD_DIMENSIONS, Variant};
use crate::errors::{Diagnostic, Severity, ToolError};
use crate::json_pos::{Position, PositionIndex, segment};

/// A board file to read: its name for diagnostics (the folder as the
/// request gave it, then the file name), its board ID (the file name
/// without .json) and its text.
pub struct BoardFile {
    pub name: String,
    pub id: String,
    pub text: String,
}

/// The custom boards of a request: none without a folder, else the folder's
/// boards when none has a problem. A folder that can't be read, or any file
/// in it that isn't valid JSON or has an invalid board, is a request error
/// whose diagnostics name each problem. Diagnostics name files by the folder
/// as the request gave it (a request's display paths may rename it,
/// docs/tool-api.md).
pub fn load(folder: Option<&str>) -> Result<CustomBoards, ToolError> {
    let Some(given) = folder else {
        return Ok(CustomBoards::new());
    };
    let path = Path::new(given);
    if !path.is_absolute() {
        return Err(ToolError::request(format!(
            "custom_boards_folder: {given:?} isn't an absolute path."
        )));
    }
    let cant = |e: std::io::Error| {
        ToolError::request(format!("custom_boards_folder: can't read {given}: {e}."))
    };
    let real = crate::paths::resolve(path).map_err(cant)?;
    let mut names: Vec<String> = std::fs::read_dir(&real)
        .map_err(cant)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    // Each file as the request named its folder, with the platform's
    // separator (\ on Windows).
    let sep = std::path::MAIN_SEPARATOR;
    let base = given.trim_end_matches(['/', sep]);
    let mut files = Vec::new();
    for n in &names {
        let text = std::fs::read_to_string(real.join(n)).map_err(|e| {
            ToolError::request(format!(
                "custom_boards_folder: can't read {base}{sep}{n}: {e}."
            ))
        })?;
        files.push(BoardFile {
            name: format!("{base}{sep}{n}"),
            id: n.trim_end_matches(".json").to_string(),
            text,
        });
    }
    let result = parse(given, &files);
    log::info!("{}", log_line(given, &files, &result));
    result
}

/// The log's line about a request's custom boards folder: its path, a short
/// SHA-256 of its board files (to tell which version was used) and its board
/// IDs, or how many problems they have.
fn log_line(given: &str, files: &[BoardFile], result: &Result<CustomBoards, ToolError>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for f in files {
        h.update(f.id.as_bytes());
        h.update([0]);
        h.update(f.text.as_bytes());
        h.update([0]);
    }
    let digest = hex::encode(h.finalize());
    let head = format!("Custom boards folder: {given} (sha256 {})", &digest[..12]);
    match result {
        Ok(boards) => {
            let ids: Vec<&str> = boards.keys().map(String::as_str).collect();
            format!(
                "{head}, boards: {}",
                if ids.is_empty() {
                    "none".into()
                } else {
                    ids.join(", ")
                }
            )
        }
        Err(e) => format!("{head}, problems: {}", e.diagnostics.len().max(1)),
    }
}

/// At most this many problems are listed. Past it, a last note says how
/// many more there are: editing these files by hand is rare, and a few
/// problems at a time are easier to fix than a long list.
pub const MAX_PROBLEMS: usize = 10;

/// The boards in `files`, or a request error with a diagnostic for each
/// problem, by file name, then in file order. `folder` names the folder as
/// the request gave it.
pub fn parse(folder: &str, files: &[BoardFile]) -> Result<CustomBoards, ToolError> {
    let mut found = Vec::new();
    let mut boards = CustomBoards::new();
    for f in files {
        let (board, problems) = parse_board(&f.text, &f.id);
        found.extend(problems.into_iter().map(|p| (f.name.clone(), p)));
        if let Some(b) = board {
            boards.insert(f.id.clone(), b);
        }
    }
    if found.is_empty() {
        return Ok(boards);
    }
    // By file, then file order; a problem without a place first in its file.
    found.sort_by(|(fa, a), (fb, b)| {
        (fa, a.pos.unwrap_or((0, 0))).cmp(&(fb, b.pos.unwrap_or((0, 0))))
    });
    Err(problems(folder, found))
}

/// One board file's board, when it has no problem, and its problems. Invalid
/// JSON is one problem, the parser's. Otherwise the board and each variant
/// are read on their own, so a wrong type or field in one doesn't hide
/// problems in the others, and every part that reads cleanly is checked in
/// full.
fn parse_board(text: &str, id: &str) -> (Option<CustomBoard>, Vec<Found>) {
    let mut found = Vec::new();
    if !catalog::is_catalog_id(id) {
        found.push(Found::at(
            None,
            format!("{id:?} isn't a board ID: name the file with lowercase letters, digits and hyphens, then .json."),
        ));
    }
    let root: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            found.push(from_serde(&e, (1, 1)));
            return (None, found);
        }
    };
    if !root.is_object() {
        found.push(Found::at(
            Some((1, 1)),
            "Expected a board: an object with name, fpga_id and variants.".into(),
        ));
        return (None, found);
    }
    let doc = Doc::new(text, &root);
    // The file's schema, for editors: optional, but only this one.
    if let Some(value) = root.get(catalog::SCHEMA_KEY)
        && let Some(problem) = catalog::schema_problem(value)
    {
        found.push(Found::at(
            doc.index
                .value(&format!("/{}", segment(catalog::SCHEMA_KEY))),
            problem,
        ));
    }
    let variants_value = root.get("variants").and_then(|v| v.as_array());
    let board = doc.read_board("");
    if let Err(e) = &board {
        found.push(e.clone());
    }
    let mut variants = Vec::new();
    for i in 0..variants_value.map_or(0, Vec::len) {
        match doc.read::<Variant>(&format!("/variants/{i}")) {
            Ok(v) => variants.push((i, v)),
            Err(e) => found.push(e),
        }
    }
    let checks = board_problems(board.as_ref().ok(), variants_value.map(Vec::len), &variants);
    found.extend(checks.into_iter().map(|p| {
        let pos = match p.at {
            At::Key(path) => doc.index.key(&path),
            At::Value(path) => doc.index.nearest(&path),
        };
        Found::at(pos, p.text)
    }));
    let board = match board {
        Ok(mut b) if found.is_empty() => {
            b.variants = variants.into_iter().map(|(_, v)| v).collect();
            b.schema = None;
            Some(b)
        }
        _ => None,
    };
    (board, found)
}

/// A problem found, at its place in the file when known.
#[derive(Debug, Clone)]
struct Found {
    pos: Option<Position>,
    text: String,
}

impl Found {
    fn at(pos: Option<Position>, text: String) -> Found {
        Found { pos, text }
    }
}

/// The parser's problem, its place counted from `start`, where the text it
/// read starts in the file. A missing field is at the object it belongs in
/// (`start`): the parser notices it only at the object's end.
fn from_serde(e: &serde_json::Error, start: Position) -> Found {
    if e.to_string().starts_with("missing field") {
        return Found::at(Some(start), serde_message(e));
    }
    let (line, column) = (e.line() as u32, e.column() as u32);
    let pos = match (line, column) {
        (0, _) | (_, 0) => Some(start),
        (1, c) => Some((start.0, start.1 + c - 1)),
        (l, c) => Some((start.0 + l - 1, c)),
    };
    Found::at(pos, serde_message(e))
}

/// The file's text and where its values are, to read its parts one by one.
struct Doc<'a> {
    chars: Vec<char>,
    root: &'a serde_json::Value,
    index: PositionIndex,
}

impl<'a> Doc<'a> {
    fn new(text: &str, root: &'a serde_json::Value) -> Doc<'a> {
        Doc {
            chars: text.chars().collect(),
            root,
            index: PositionIndex::new(text).unwrap_or_default(),
        }
    }

    /// The value at `path` as written in the file, or None when the index
    /// doesn't have it.
    fn text(&self, path: &str) -> Option<Vec<char>> {
        let (a, b) = self.index.span(path)?;
        Some(self.chars[a..b].to_vec())
    }

    /// The value at `path` read as a `T`, with the parser's problem at its
    /// place in the file.
    fn read<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, Found> {
        let start = self.index.value(path).unwrap_or((1, 1));
        match self.text(path) {
            Some(chars) => serde_json::from_str(&chars.iter().collect::<String>())
                .map_err(|e| from_serde(&e, start)),
            None => serde_json::from_value(self.root.pointer(path).cloned().unwrap_or_default())
                .map_err(|e| Found::at(Some(start), serde_message(&e))),
        }
    }

    /// The board at `path` without its variants, which are read one by one:
    /// a list of variants is blanked to `[]` (lines kept, so places stay
    /// right).
    fn read_board(&self, path: &str) -> Result<CustomBoard, Found> {
        let variants = format!("{path}/variants");
        let is_list = self.root.pointer(&variants).is_some_and(|v| v.is_array());
        let (Some((a, _)), Some((va, vb)), Some(mut chars), true) = (
            self.index.span(path),
            self.index.span(&variants),
            self.text(path),
            is_list,
        ) else {
            return self.read(path);
        };
        for (n, c) in chars[va - a..vb - a].iter_mut().enumerate() {
            *c = match (n, *c) {
                (0, _) => '[',
                (1, _) => ']',
                (_, '\n') => '\n',
                _ => ' ',
            };
        }
        let start = self.index.value(path).unwrap_or((1, 1));
        serde_json::from_str(&chars.iter().collect::<String>()).map_err(|e| from_serde(&e, start))
    }
}

/// The request error for the folder's problems: the first ones, at most
/// `MAX_PROBLEMS`, then a note with how many more there are.
fn problems(folder: &str, found: Vec<(String, Found)>) -> ToolError {
    let total = found.len();
    let mut diagnostics: Vec<Diagnostic> = found
        .into_iter()
        .take(MAX_PROBLEMS)
        .map(|(file, f)| Diagnostic {
            file: Some(file),
            line: f.pos.map(|p| p.0),
            column: f.pos.map(|p| p.1),
            severity: Severity::Error,
            text: f.text,
            tool: None,
            time_ns: None,
        })
        .collect();
    if total > MAX_PROBLEMS {
        let more = total - MAX_PROBLEMS;
        diagnostics.push(Diagnostic {
            file: Some(folder.to_string()),
            line: None,
            column: None,
            severity: Severity::Info,
            text: format!(
                "{} not listed. Fix these first, then check again.",
                if more == 1 {
                    "1 more problem".to_string()
                } else {
                    format!("{more} more problems")
                }
            ),
            tool: None,
            time_ns: None,
        });
    }
    let first = &diagnostics[0];
    let name = first.file.clone().unwrap_or_default();
    let place = match (first.line, first.column) {
        (Some(l), Some(c)) => format!("{name}:{l}:{c}"),
        (Some(l), None) => format!("{name}:{l}"),
        _ => name,
    };
    let more = match total {
        1 => String::new(),
        2 => " (and 1 more problem)".to_string(),
        n => format!(" (and {} more problems)", n - 1),
    };
    let mut e = ToolError::request(format!(
        "The custom boards folder has problems. {place}: {}{more}",
        first.text
    ));
    e.diagnostics = diagnostics;
    e
}

/// serde's message without its own " at line L column C".
/// An unknown field is just named, without serde's list of the expected
/// ones (the user's request: too much to read).
fn serde_message(e: &serde_json::Error) -> String {
    let text = e.to_string();
    let text = match text.rfind(" at line ") {
        Some(i) => text[..i].to_string(),
        None => text,
    };
    if let Some(rest) = text.strip_prefix("unknown field `")
        && let Some(end) = rest.find('`')
    {
        return format!("Unknown field {:?}.", &rest[..end]);
    }
    let mut c = text.chars();
    match c.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), c.as_str()),
        None => "Invalid JSON.".into(),
    }
}

/// Where a problem is: a member's key, or a value (or, when it's missing,
/// the object it belongs in).
enum At {
    Key(String),
    Value(String),
}

struct Problem {
    at: At,
    text: String,
}

/// Every problem with one board's contents: its FPGA and resources, when
/// `board` (without its variants) could be read; that it has variants, when
/// `variant_count` says how many are listed; and each of `variants`, those
/// that could be read, with their place in the list.
fn board_problems(
    board: Option<&CustomBoard>,
    variant_count: Option<usize>,
    variants: &[(usize, Variant)],
) -> Vec<Problem> {
    let mut out = Vec::new();
    let mut add = |at: At, text: String| out.push(Problem { at, text });
    if let Some(b) = board {
        let f = &b.fpga_id;
        let fpga = catalog::get().fpgas.get(f);
        if fpga.is_none() {
            add(
                At::Value("/fpga_id".into()),
                format!("{f:?} isn't an FPGA in the catalog. Use get_catalog to list them."),
            );
        }
        for (field, text) in crate::resources::board_requires_problems(
            &b.requires,
            &b.resources,
            fpga.map(|f| f.family),
        ) {
            add(At::Value(format!("/requires/{field}")), text);
        }
        for (field, text) in crate::resources::ai_hints_problems(b.ai_hints.as_ref()) {
            let path = format!("/ai_hints/{field}");
            add(At::Value(path.trim_end_matches('/').to_string()), text);
        }
        for (name, field, text) in crate::resources::problems(&b.resources, fpga.map(|f| f.family))
        {
            let base = format!("/resources/{}", segment(&name));
            add(
                if field.is_empty() {
                    At::Key(base)
                } else {
                    At::Value(format!("{base}/{field}"))
                },
                text,
            );
        }
    }
    if variant_count == Some(0) {
        add(
            At::Value("/variants".into()),
            "A board needs at least one variant.".into(),
        );
    }
    let mut selectors = BTreeSet::new();
    for (i, v) in variants {
        let vf = |f: &str| At::Value(format!("/variants/{i}/{f}"));
        for (dim, _, values) in STANDARD_DIMENSIONS {
            let value = v.value(dim).unwrap_or_default();
            if !values.iter().any(|(x, _, _)| *x == value) {
                let known: Vec<_> = values.iter().map(|(x, _, _)| *x).collect();
                add(
                    vf(&format!("selector/{dim}")),
                    format!(
                        "{value:?} isn't a {dim} value. Known: {}.",
                        known.join(", ")
                    ),
                );
            }
        }
        if !selectors.insert(v.selector.clone()) {
            add(
                vf("selector"),
                format!("Another variant has the same selector ({}).", v.selector),
            );
        }
        if !PROGRAMMERS.contains(&v.programmer.as_str()) {
            add(
                vf("programmer"),
                format!(
                    "Unknown programmer {:?}. Known: {}.",
                    v.programmer,
                    PROGRAMMERS.join(", ")
                ),
            );
        }
        if let Err(e) = catalog::check_template(&v.args) {
            add(vf("args"), format!("{}.", capitalize(&e)));
        }
        for (name, value) in [
            ("vid", Some(&v.usb.vid)),
            ("pid", Some(&v.usb.pid)),
            ("release", v.usb.release.as_ref()),
        ] {
            if value.is_some_and(|x| !catalog::is_usb_id(x)) {
                add(
                    vf(&format!("usb/{name}")),
                    format!("usb {name} must be four lowercase hex digits, such as 0403."),
                );
            }
        }
        for (name, re) in [
            ("product_regex", &v.usb.product_regex),
            ("serial_regex", &v.usb.serial_regex),
        ] {
            if let Some(Err(e)) = re.as_deref().map(regex::Regex::new) {
                add(
                    vf(&format!("usb/{name}")),
                    format!(
                        "Bad {name}: {}.",
                        e.to_string().lines().last().unwrap_or("invalid")
                    ),
                );
            }
        }
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Writes `boards` (board ID to board) as board files, `<id>.json`, in a new
/// temporary folder, for tests: the temporary folder (kept while it lives)
/// and the board folder's path.
#[cfg(test)]
pub(crate) fn test_folder(boards: &serde_json::Value) -> (tempfile::TempDir, String) {
    let tmp = tempfile::tempdir().expect("a temporary folder");
    let dir = tmp.path().join("boards");
    std::fs::create_dir(&dir).expect("the boards folder");
    for (id, b) in boards.as_object().into_iter().flatten() {
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::to_string_pretty(b).unwrap_or_default(),
        )
        .expect("writing a board file");
    }
    (tmp, dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
  "name": "My board",
  "fpga_id": "ice40up5k-sg48",
  "variants": [
    {
      "selector": {"connection": "usb", "memory": "flash"},
      "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
      "usb": {"vid": "0403", "pid": "6014"}, "tested": false
    }
  ]
}
"#;

    fn file(id: &str, text: &str) -> BoardFile {
        BoardFile {
            name: format!("/proj/boards/{id}.json"),
            id: id.into(),
            text: text.into(),
        }
    }

    /// The problems with one file named my-board.json, as (line, column, text).
    fn diags(text: &str) -> Vec<(Option<u32>, Option<u32>, String)> {
        let e = parse("/proj/boards", &[file("my-board", text)]).unwrap_err();
        e.diagnostics
            .iter()
            .map(|d| (d.line, d.column, d.text.clone()))
            .collect()
    }

    #[test]
    fn a_folder_of_good_files() {
        let boards = parse("/proj/boards", &[file("a", GOOD), file("b", GOOD)]).unwrap();
        assert_eq!(boards.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(boards["a"].name, "My board");
        assert!(parse("/proj/boards", &[]).unwrap().is_empty());
    }

    #[test]
    fn the_schema_key_is_optional_but_must_be_this_one() {
        let named = GOOD.replacen(
            "{\n",
            &format!("{{\n  \"$schema\": \"{}\",\n", catalog::BOARDS_SCHEMA),
            1,
        );
        let boards = parse("/p", &[file("my-board", &named)]).unwrap();
        assert!(boards["my-board"].schema.is_none(), "not kept");
        let other = GOOD.replacen(
            "{\n",
            "{\n  \"$schema\": \"https://www.openlogix.org/schemas/logix-boards-v1.json\",\n",
            1,
        );
        assert_eq!(
            diags(&other),
            [(
                Some(2),
                Some(14),
                format!(
                    "Unknown $schema \"https://www.openlogix.org/schemas/logix-boards-v1.json\": this Logix Engine reads {}.",
                    catalog::BOARDS_SCHEMA
                )
            )]
        );
    }

    #[test]
    fn syntax_type_and_definition_errors_at_their_line_and_column() {
        let d = diags("{\n  \"name\": \"x\",\n}\n");
        assert_eq!((d[0].0, d[0].1), (Some(3), Some(1)), "{d:?}");
        let d = diags("[]\n");
        assert_eq!((d[0].0, d[0].1), (Some(1), Some(1)), "{d:?}");
        assert!(d[0].2.starts_with("Expected a board"), "{d:?}");
        let d = diags(&GOOD.replace("\"tested\"", "\"testd\""));
        assert_eq!(d[0].0, Some(8), "{d:?}");
        assert_eq!(d[0].2, "Unknown field \"testd\".");
        let d = diags(
            &GOOD
                .replace("ice40up5k-sg48", "nope")
                .replace("\"iceprog\"", "\"tinyprog\""),
        );
        assert_eq!((d[0].0, d[0].1), (Some(3), Some(14)), "{d:?}");
        assert!(
            d[0].2.starts_with("\"nope\" isn't an FPGA in the catalog"),
            "{d:?}"
        );
        assert_eq!(d[1].0, Some(7), "{d:?}");
        assert!(
            d[1].2.starts_with("Unknown programmer \"tinyprog\""),
            "{d:?}"
        );
        // The old field, description, is an unknown field.
        let d = diags(&GOOD.replace("\"name\"", "\"description\""));
        assert_eq!(d[0].2, "Unknown field \"description\".");
    }

    #[test]
    fn selector_problems_point_at_the_selector() {
        let d = diags(&GOOD.replace("\"memory\": \"flash\"", "\"memory\": \"eeprom\""));
        assert_eq!(d[0].0, Some(6), "{d:?}");
        assert!(
            d[0].2
                .starts_with("\"eeprom\" isn't a memory value. Known: flash, ram."),
            "{d:?}"
        );
        let d = diags(&GOOD.replace("\"connection\": \"usb\", ", ""));
        assert!(d[0].2.starts_with("Missing field `connection`"), "{d:?}");
    }

    #[test]
    fn resource_problems_point_at_their_values() {
        let with = GOOD.replacen(
            "  \"variants\": [",
            "  \"resources\": {\n    \"sys_clk\": {\"type\": \"clock\", \"pin\": \"35\", \"mhz\": 0},\n    \"2led\": {\"type\": \"leds\", \"pins\": [\"1\"]}\n  },\n  \"variants\": [",
            1,
        );
        let d = diags(&with);
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!(
            (d[0].0, d[0].1, d[0].2.as_str()),
            (Some(5), Some(54), "0 isn't a frequency in MHz.")
        );
        assert_eq!(
            (d[1].0, d[1].1),
            (Some(6), Some(5)),
            "the name's key: {d:?}"
        );
        let ok = with
            .replace("\"mhz\": 0", "\"mhz\": 12")
            .replace("\"2led\"", "\"led\"");
        let boards = parse("/p", &[file("my-board", &ok)]).unwrap();
        assert_eq!(
            boards["my-board"].resources["sys_clk"].clock_mhz(),
            Some(12.0)
        );
        // An unknown resource type is a problem too.
        let d = diags(&ok.replace("\"type\": \"leds\"", "\"type\": \"lamp\""));
        assert_eq!(d.len(), 1, "{d:?}");
    }

    #[test]
    fn requirement_problems_point_at_their_values() {
        // A board's drive named like a resource, and a resource's empty setup.
        let with = GOOD.replacen(
            "  \"variants\": [",
            "  \"requires\": [\n    {\"type\": \"drive\", \"name\": \"led\", \"pins\": [\"2\"], \"level\": \"high\"}\n  ],\n  \"resources\": {\n    \"led\": {\"type\": \"leds\", \"pins\": [\"1\"], \"requires\": [{\"type\": \"setup\", \"text\": \"\"}]}\n  },\n  \"variants\": [",
            1,
        );
        let d = diags(&with);
        let got: Vec<_> = d.iter().map(|d| (d.0, d.2.as_str())).collect();
        assert_eq!(
            got,
            [
                (Some(5), "Another resource or drive is named led."),
                (Some(8), "Say what to do."),
            ],
            "{d:?}"
        );
        let ok = with
            .replace("\"name\": \"led\"", "\"name\": \"led_en\"")
            .replace("\"text\": \"\"", "\"text\": \"Close J1.\"");
        let boards = parse("/p", &[file("my-board", &ok)]).unwrap();
        assert_eq!(boards["my-board"].requires.len(), 1);
        assert_eq!(boards["my-board"].resources["led"].requires().len(), 1);
        // An unknown requirement type is a problem too.
        let d = diags(&ok.replace("\"type\": \"setup\"", "\"type\": \"jumper\""));
        assert_eq!(d.len(), 1, "{d:?}");
    }

    #[test]
    fn ai_hints_are_read_checked_and_written_last() {
        // On the board and on a resource: an empty hint, and one too long.
        let long = "x".repeat(crate::resources::MAX_AI_HINT_CHARS + 1);
        let with = GOOD.replacen(
            "  ]\n}\n",
            &format!("  ],\n  \"resources\": {{\n    \"led\": {{\"ai_hints\": [\"\"], \"type\": \"leds\", \"pins\": [\"1\"]}}\n  }},\n  \"ai_hints\": [\"One line.\", \"{long}\"]\n}}\n"),
            1,
        );
        let d = diags(&with);
        let got: Vec<_> = d.iter().map(|d| (d.0, d.2.as_str())).collect();
        assert_eq!(
            got,
            [
                (Some(12), "An empty hint."),
                (
                    Some(14),
                    "A hint is at most 90 characters, this one 91: split it."
                ),
            ],
            "{d:?}"
        );
        // An empty list is left out instead.
        let d = diags(&with.replace(&format!("[\"One line.\", \"{long}\"]"), "[]"));
        assert!(
            d.iter().any(|d| d.2.starts_with("Leave ai_hints out")),
            "{d:?}"
        );
        // Fixed: read, and written last in the board and the resource,
        // wherever the file has them.
        let ok = with
            .replace("[\"\"]", "[\"A resource hint.\"]")
            .replace(&format!(", \"{long}\""), "");
        let boards = parse("/p", &[file("my-board", &ok)]).unwrap();
        let b = &boards["my-board"];
        assert_eq!(b.ai_hints, Some(vec!["One line.".to_string()]));
        let led = serde_json::to_string(&b.resources["led"]).unwrap();
        assert!(
            led.ends_with(r#""ai_hints":["A resource hint."]}"#),
            "{led}"
        );
        let board = serde_json::to_string(b).unwrap();
        assert!(board.ends_with(r#""ai_hints":["One line."]}"#), "{board}");
    }

    #[test]
    fn every_problem_at_once_by_file_then_line() {
        let bad = GOOD.replace("ice40up5k-sg48", "nope");
        let e = parse(
            "/proj/boards",
            &[
                file("b-board", &bad),
                file("a-board", &bad),
                file("Bad ID", GOOD),
            ],
        )
        .unwrap_err();
        let got: Vec<_> = e
            .diagnostics
            .iter()
            .map(|d| (d.file.clone().unwrap(), d.line))
            .collect();
        assert_eq!(
            got,
            [
                ("/proj/boards/Bad ID.json".to_string(), None),
                ("/proj/boards/a-board.json".to_string(), Some(3)),
                ("/proj/boards/b-board.json".to_string(), Some(3)),
            ]
        );
        assert!(
            e.diagnostics[0].text.contains("isn't a board ID"),
            "{:?}",
            e.diagnostics
        );
        assert!(
            e.message
                .starts_with("The custom boards folder has problems. /proj/boards/Bad ID.json: "),
            "{}",
            e.message
        );
        assert!(
            e.message.ends_with(" (and 2 more problems)"),
            "{}",
            e.message
        );
    }

    #[test]
    fn at_most_ten_problems_then_how_many_more() {
        let files: Vec<BoardFile> = (0..13)
            .map(|i| {
                file(
                    &format!("b{i:02}"),
                    "{\"name\": \"x\", \"fpga_id\": \"ice40up5k-sg48\", \"variants\": []}",
                )
            })
            .collect();
        let e = parse("/proj/boards", &files).unwrap_err();
        assert_eq!(e.diagnostics.len(), MAX_PROBLEMS + 1, "{:?}", e.diagnostics);
        let last = &e.diagnostics[MAX_PROBLEMS];
        assert_eq!(last.severity, Severity::Info);
        assert_eq!(last.file.as_deref(), Some("/proj/boards"));
        assert_eq!(
            last.text,
            "3 more problems not listed. Fix these first, then check again."
        );
    }

    #[test]
    fn the_log_names_the_folder_its_version_and_boards() {
        let files = [file("a", GOOD)];
        let line = log_line("/proj/boards", &files, &parse("/proj/boards", &files));
        assert!(
            line.starts_with("Custom boards folder: /proj/boards (sha256 "),
            "{line}"
        );
        assert!(line.ends_with(", boards: a"), "{line}");
        let bad = [file("a", "{")];
        let line = log_line("/proj/boards", &bad, &parse("/proj/boards", &bad));
        assert!(line.ends_with(", problems: 1"), "{line}");
    }

    #[test]
    fn folders_are_read_by_absolute_path_and_named_as_given() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("boards");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("mine.json"), GOOD.replace("\"0403\"", "\"x\"")).unwrap();
        std::fs::write(dir.join("README.md"), "not a board").unwrap();
        let given = format!("{}/", dir.to_string_lossy());
        let e = load(Some(&given)).unwrap_err();
        assert_eq!(
            e.diagnostics.len(),
            1,
            "other files are ignored: {:?}",
            e.diagnostics
        );
        assert_eq!(
            e.diagnostics[0].file.as_deref(),
            Some(format!("{}/mine.json", dir.to_string_lossy()).as_str())
        );
        std::fs::write(dir.join("mine.json"), GOOD).unwrap();
        assert_eq!(
            load(Some(&given)).unwrap().keys().collect::<Vec<_>>(),
            ["mine"]
        );
        assert!(
            load(Some("boards"))
                .unwrap_err()
                .message
                .contains("absolute")
        );
        let missing = tmp.path().join("none").to_string_lossy().into_owned();
        assert!(
            load(Some(&missing))
                .unwrap_err()
                .message
                .contains("can't read")
        );
        assert!(load(None).unwrap().is_empty());
    }
}
