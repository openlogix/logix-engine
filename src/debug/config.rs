//! The debug configuration file (docs/debugger.md, The debug configuration
//! file): TOML, checked against the types below, whose schema is
//! schemas/logix-debug-v1.json. Editors find the schema from a first line
//! `#:schema <SCHEMA_URL>` (Taplo, Even Better TOML).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The schema's address, for a debug file's `#:schema` line.
#[cfg_attr(not(test), allow(dead_code))]
pub const SCHEMA_URL: &str = "https://www.openlogix.org/schemas/logix-debug-v1.json";

/// A debug configuration file: which nets the debugger reads and drives, how
/// it reaches the host and which clock it runs on.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Logix debug configuration, version 1")]
pub struct DebugConfig {
    /// The clock the debugger and its UART run on: a top-level input port of
    /// the design, such as sys_clk. Every watched and driven net must be on
    /// it.
    pub clock: String,
    /// The clock's frequency in MHz. Needed only when the clock port isn't
    /// on the board's clock pin, whose frequency the catalog gives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_mhz: Option<f64>,
    /// How the debugger reaches the host: a UART, named as a resource of the
    /// board, or by its pins.
    pub uplink: Uplink,
    /// Nets the host can drive: take them over from the design, or drive
    /// them alone, and release them.
    #[serde(default)]
    pub drive: Vec<DriveNet>,
    /// Nets the host reads.
    #[serde(default)]
    pub watch: Vec<WatchNet>,
    /// The debug panel's settings, which the host writes when they change
    /// in the panel. They don't change the debug build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<PanelSettings>,
    /// Reserved for the triggers phase; refused in version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub trigger: Option<toml::Value>,
    /// Reserved for the counters of the triggers phase; refused in version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub count: Option<toml::Value>,
    /// Reserved for the capture phase; refused in version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub capture: Option<toml::Value>,
}

/// The debug panel's settings: how often it reads the board and how much
/// of the chart it shows. Each is optional; the panel picks the nearest
/// rate and span it offers.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelSettings {
    /// How often the panel reads the board, in milliseconds between reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub refresh_ms: Option<u32>,
    /// How much of the chart the panel shows, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub window_s: Option<u32>,
}

/// The link to the host: give resource, or tx and rx.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Uplink {
    /// The kind of link: uart, the only one in version 1.
    pub kind: UplinkKind,
    /// The board's UART, by its resource name in the catalog (get_catalog's
    /// resources), such as uart: the debugger uses its tx and rx pins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// Instead of resource, the FPGA pin the debugger transmits on, such as
    /// a PMOD pin wired to a USB serial adapter's RX.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx: Option<String>,
    /// Instead of resource, the FPGA pin the debugger receives on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rx: Option<String>,
    /// Bits per second. Defaults to 115200.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baud: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum UplinkKind {
    Uart,
}

/// A net the debugger reads.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchNet {
    /// The host's name for it. Defaults to the net's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The net, of any width, by its instance path from the top module,
    /// starting with the top module's name, such as main.u_ctrl.state, or
    /// main.blink for a net in the top module itself; a package pin as
    /// pin.18; or a list of one-bit nets and pins combined into one bus, such
    /// as ["pin.21", "pin.20", "pin.19", "pin.18"], the first the most
    /// significant, as in a Verilog concatenation (a list needs a name).
    #[serde(rename = "net")]
    pub spec: NetSpec,
    /// The net's path, or a list as text ({a, b}). Set by parse.
    #[serde(skip)]
    #[schemars(skip)]
    pub net: String,
    /// How hosts such as the debug panel show its value at first: hex, dec
    /// or bin. Omitted means dec for one bit and hex for more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ValueFormat>,
    /// The paths it reads, most significant first: the net, or the list's
    /// members. Set by parse.
    #[serde(skip)]
    #[schemars(skip)]
    pub parts: Vec<String>,
}

impl WatchNet {
    /// True for a bus combined from a list.
    pub fn is_list(&self) -> bool {
        matches!(self.spec, NetSpec::List(_))
    }
}

impl DriveNet {
    pub fn is_list(&self) -> bool {
        matches!(self.spec, NetSpec::List(_))
    }
}

/// How a watched value is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ValueFormat {
    Hex,
    Dec,
    Bin,
}

impl ValueFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            ValueFormat::Hex => "hex",
            ValueFormat::Dec => "dec",
            ValueFormat::Bin => "bin",
        }
    }
}

/// A net: one path, or a list of one-bit nets and pins combined.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum NetSpec {
    Path(String),
    List(Vec<String>),
}

/// A net the debugger drives.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DriveNet {
    /// The host's name for it. Defaults to the net's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The net, named as for watch; a list's nets are driven and released
    /// together, and can't include pins.
    #[serde(rename = "net")]
    pub spec: NetSpec,
    /// The net's path, or a list as text ({a, b}). Set by parse.
    #[serde(skip)]
    #[schemars(skip)]
    pub net: String,
    /// How hosts such as the debug panel show and read its value at first: hex, dec
    /// or bin. Omitted means dec for one bit and hex for more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ValueFormat>,
    /// The paths it drives, most significant first. Set by parse.
    #[serde(skip)]
    #[schemars(skip)]
    pub parts: Vec<String>,
}

/// Parses a debug file's text, and checks what its types can't.
pub fn parse(text: &str) -> Result<DebugConfig, String> {
    let mut config: DebugConfig = toml::from_str(text).map_err(|e| {
        // One line, with the place: toml's own text adds an excerpt.
        let message = e.message().trim_end_matches('.').to_string();
        match e.span() {
            Some(span) => {
                let before = &text[..span.start.min(text.len())];
                let line = before.matches('\n').count() + 1;
                let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                format!("{message} (at line {line}, column {column}).")
            }
            None => format!("{message}."),
        }
    })?;
    // net or bus: after this, net holds the path either way (for a list,
    // the list as text), parts the paths, and bus stays set for a bus.
    for i in &mut config.watch {
        i.parts = parts(&mut i.net, &i.spec, &i.name)?;
    }
    for o in &mut config.drive {
        o.parts = parts(&mut o.net, &o.spec, &o.name)?;
    }
    for (key, present) in [
        ("trigger", config.trigger.is_some()),
        ("count", config.count.is_some()),
        ("capture", config.capture.is_some()),
    ] {
        if present {
            return Err(format!(
                "{key} isn't supported yet: it comes in a later version."
            ));
        }
    }
    let u = &config.uplink;
    match (&u.resource, &u.tx, &u.rx) {
        (Some(_), None, None) | (None, Some(_), Some(_)) => {}
        (Some(_), _, _) => {
            return Err("uplink: give resource, or tx and rx, not both.".into());
        }
        (None, _, _) => {
            return Err(
                "uplink: name the board's UART as resource, such as \"resource\": \"uart\", or give both tx and rx pins."
                    .into(),
            );
        }
    }
    if config.watch.is_empty() && config.drive.is_empty() {
        return Err("The file names no nets: add some to watch or drive.".into());
    }
    for p in config
        .watch
        .iter()
        .flat_map(|i| &i.parts)
        .chain(config.drive.iter().flat_map(|o| &o.parts))
    {
        check_net(p)?;
    }
    let mut driven = std::collections::BTreeSet::new();
    for p in config.drive.iter().flat_map(|o| &o.parts) {
        if !driven.insert(p) {
            return Err(format!(
                "{p} is in two drive entries: drive each net from one."
            ));
        }
    }
    // Host names: one per watch entry and one per drive entry. A watch
    // and a drive entry of the same net may share one (their default, the
    // net's path): the host reads one and writes the other, so they never
    // meet. Any other repeat is an error.
    let mut names: [std::collections::BTreeMap<&str, &str>; 2] = Default::default();
    for (k, kind, net, name) in config
        .watch
        .iter()
        .map(|i| (0, "watch", &i.net, &i.name))
        .chain(config.drive.iter().map(|o| (1, "drive", &o.net, &o.name)))
    {
        if !net.starts_with('{') {
            check_net(net)?;
        }
        let host = name.as_deref().unwrap_or(net);
        if host.is_empty() {
            return Err(format!("The name of net {net:?} is empty."));
        }
        if names[k].insert(host, net).is_some() {
            return Err(format!(
                "Two {kind} entries are named {host:?}: give one of them another name, or remove it."
            ));
        }
        if let Some(other) = names[1 - k].get(host)
            && *other != net.as_str()
        {
            return Err(format!(
                "The watch and drive entries of {other} and {net} are both named {host:?}: give one of them another name."
            ));
        }
    }
    if let Some(mhz) = config.clock_mhz
        && !(mhz > 0.0 && mhz <= 1000.0)
    {
        return Err(format!("clock_mhz {mhz} isn't a clock frequency."));
    }
    Ok(config)
}

/// A watch or drive entry's paths, from its net.
fn parts(net: &mut String, spec: &NetSpec, name: &Option<String>) -> Result<Vec<String>, String> {
    match spec {
        NetSpec::Path(path) => {
            *net = path.clone();
            Ok(vec![path.clone()])
        }
        NetSpec::List(list) => {
            *net = format!("{{{}}}", list.join(", "));
            if list.len() < 2 {
                return Err(format!(
                    "The net list {net} has fewer than two members: name a single net as a string."
                ));
            }
            if name.is_none() {
                return Err(format!(
                    "The net list {net} needs a name, as it combines several nets."
                ));
            }
            Ok(list.clone())
        }
    }
}

/// Version 1's naming rules: an instance path then a net, whole nets only.
fn check_net(net: &str) -> Result<(), String> {
    let bad = |why: &str| {
        Err(format!(
            "The net {net:?} {why}. Name a whole net by its instance path, such as main.u_ctrl.state."
        ))
    };
    if net.is_empty() {
        return bad("is empty");
    }
    if net
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\')
    {
        return bad("has a space, quote or backslash");
    }
    if net.contains(['*', '?']) {
        return bad("has a wildcard, which version 1 doesn't support");
    }
    // Brackets are for generate blocks in the path, not on the net itself.
    if net
        .rsplit('.')
        .next()
        .is_some_and(|last| last.contains('['))
    {
        return bad("selects bits, which version 1 doesn't support");
    }
    if net.starts_with('.') || net.ends_with('.') || net.contains("..") {
        return bad("isn't a path");
    }
    Ok(())
}

/// The schema of the file, as written to schemas/logix-debug-v1.json (by the
/// schema_file_is_current test).
#[cfg(test)]
pub fn schema_json() -> String {
    let mut schema = serde_json::to_value(schemars::schema_for!(DebugConfig)).unwrap();
    if let Some(o) = schema.as_object_mut() {
        o.insert("$id".into(), SCHEMA_URL.into());
    }
    serde_json::to_string_pretty(&schema).unwrap() + "\n"
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// For tests: a debug file written as JSON, converted to TOML and parsed.
    pub fn parse_json(json: &str) -> Result<DebugConfig, String> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        parse(&toml::to_string(&value).map_err(|e| e.to_string())?)
    }

    #[test]
    fn parses_the_spec_example() {
        let c = parse(
            r#"#:schema https://www.openlogix.org/schemas/logix-debug-v1.json
clock = "sys_clk"
uplink = { kind = "uart", resource = "uart", baud = 115200 }  # a comment

[[drive]]
net = "main.blink"
name = "blink_force"

[[watch]]
net = "main.count"
name = "counter"
format = "dec"

[[watch]]
net = "main.gen_lane[2].u_lane.err"

[[watch]]
net = ["pin.21", "pin.20"]
name = "two_pins"
"#,
        )
        .unwrap();
        assert_eq!(c.watch.len(), 3);
        assert_eq!(c.watch[2].parts, vec!["pin.21", "pin.20"]);
        assert_eq!(c.watch[0].format, Some(ValueFormat::Dec));
        assert_eq!(c.watch[1].format, None);
        assert!(parse_json(r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"main.a","format":"octal"}]}"#).is_err());
    }

    #[test]
    fn a_net_watched_and_driven_shares_its_name() {
        // The net picker writes both with their default names.
        let c = parse_json(
            r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"main.led"}],"drive":[{"net":"main.led"}]}"#,
        )
        .unwrap();
        assert_eq!((c.watch.len(), c.drive.len()), (1, 1));
    }

    #[test]
    fn reads_the_panel_settings() {
        let c = parse(
            "clock = \"c\"\nuplink = { kind = \"uart\", resource = \"uart\" }\n\n[[watch]]\nnet = \"main.led\"\n\n[panel]\nrefresh_ms = 197\nwindow_s = 60\n",
        )
        .unwrap();
        let panel = c.panel.unwrap();
        assert_eq!((panel.refresh_ms, panel.window_s), (Some(197), Some(60)));
        let e = parse_json(
            r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"main.led"}],"panel":{"rate":5}}"#,
        )
        .unwrap_err();
        assert!(e.contains("rate"), "{e}");
    }

    #[test]
    fn syntax_errors_name_the_line() {
        let e = parse("clock = \"c\"\nuplink = {\n").unwrap_err();
        assert!(e.contains("line 2"), "{e}");
    }

    #[test]
    fn refuses_what_version_1_lacks() {
        for (text, why) in [
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"bus[7:4]"}]}"#,
                "selects bits",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"u_*.x"}]}"#,
                "wildcard",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"a"}],"capture":{}}"#,
                "capture isn't supported yet",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"a"}],"trigger":[]}"#,
                "trigger isn't supported yet",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"a"}],"count":[]}"#,
                "count isn't supported yet",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"}}"#,
                "no nets",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"a"},{"net":"b","name":"a"}]}"#,
                "Two watch entries",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"a"}],"drive":[{"net":"b","name":"a"}]}"#,
                "both named",
            ),
            (
                r#"{"clock":"c","uplink":{"kind":"uart","resource":"uart"},"watch":[{"net":"a"}],"extra":1}"#,
                "unknown field",
            ),
        ] {
            let e = parse_json(text).unwrap_err();
            assert!(e.contains(why), "{text}: {e}");
        }
    }

    #[test]
    fn uplinks_name_a_resource_or_pins() {
        let up = |u: &str| {
            parse_json(&format!(
                r#"{{"clock":"c","uplink":{u},"watch":[{{"net":"main.a"}}]}}"#
            ))
        };
        let u = up(r#"{"kind":"uart","resource":"uart"}"#).unwrap().uplink;
        assert_eq!(
            (u.kind, u.resource.as_deref()),
            (UplinkKind::Uart, Some("uart"))
        );
        assert!(up(r#"{"kind":"uart","tx":"1","rx":"2"}"#).is_ok());
        for (bad, why) in [
            (r#"{}"#, "kind"),
            (r#"{"kind":"uart"}"#, "resource"),
            (r#"{"kind":"uart","tx":"1"}"#, "resource"),
            (r#"{"kind":"uart","resource":"uart","tx":"1"}"#, "not both"),
        ] {
            let e = up(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn net_is_a_path_or_a_list() {
        let p = |e: &str| {
            parse_json(&format!(
                r#"{{"clock":"c","uplink":{{"kind":"uart","resource":"uart"}},"watch":[{e}]}}"#
            ))
        };
        let c = p(r#"{"net":"main.count"}"#).unwrap();
        assert_eq!(
            (c.watch[0].net.as_str(), c.watch[0].is_list()),
            ("main.count", false)
        );
        assert!(p(r#"{"name":"x"}"#).unwrap_err().contains("net"));
        assert!(
            p(r#"{"net":"main.a","bus":"main.a"}"#)
                .unwrap_err()
                .contains("bus")
        );
    }

    #[test]
    fn bus_lists() {
        let p = |io: &str, e: &str| {
            parse_json(&format!(
                r#"{{"clock":"c","uplink":{{"kind":"uart","resource":"uart"}},"{io}":[{e}]}}"#
            ))
        };
        let c = p("watch", r#"{"net":["pin.19","main.a"],"name":"x"}"#).unwrap();
        assert_eq!(c.watch[0].parts, vec!["pin.19", "main.a"]);
        assert_eq!(c.watch[0].net, "{pin.19, main.a}");
        let c = p("drive", r#"{"net":["main.a","main.b"],"name":"y"}"#).unwrap();
        assert!(c.drive[0].is_list());
        for (io, e, why) in [
            ("watch", r#"{"net":["main.a","main.b"]}"#, "needs a name"),
            (
                "watch",
                r#"{"net":["main.a"],"name":"x"}"#,
                "fewer than two",
            ),
            (
                "drive",
                r#"{"net":["main.a","main.a"],"name":"y"}"#,
                "two drive entries",
            ),
        ] {
            let err = p(io, e).unwrap_err();
            assert!(err.contains(why), "{e}: {err}");
        }
    }

    /// schemas/logix-debug-v1.json is the types' schema. To update it:
    /// UPDATE_SCHEMA=1 cargo test schema_file
    #[test]
    fn schema_file_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/logix-debug-v1.json");
        if std::env::var_os("UPDATE_SCHEMA").is_some() {
            std::fs::write(path, schema_json()).unwrap();
        }
        // Git on Windows may check the file out with CRLF line endings.
        let on_disk = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            on_disk == schema_json(),
            "schemas/logix-debug-v1.json is out of date: run UPDATE_SCHEMA=1 cargo test schema_file"
        );
    }
}
