//! The debug session tools: open_debug, read_debug, write_debug and
//! close_debug (docs/debugger.md, Host side). A session holds a board's
//! serial port and the debug map of the build on the board.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::{Category, ToolError};
use crate::serial::{DeviceKey, usb_ports};
use crate::tools::EngineTool;

use super::lock;
use super::protocol::{self, DebugMap};

/// How long a reply may take. A READ of 255 bytes takes about 23 ms at
/// 115200 baud and 270 ms at 9600; USB serial bridges add a few ms.
const REPLY_TIMEOUT: Duration = Duration::from_millis(1000);

struct Session {
    /// The open port; None after the link broke, such as with the board
    /// unplugged, until the next read or write opens it again.
    port: Option<Box<dyn serialport::SerialPort>>,
    port_name: String,
    /// Set when the port was opened again: the board may have restarted,
    /// so once its build is checked, everything is released to match.
    reconnected: bool,
    map: DebugMap,
    /// The write values and the override enables, as the debugger last
    /// reported them (each READ does) or the host last wrote them.
    wr: Vec<bool>,
    en: Vec<bool>,
    /// The port's lock across processes (super::lock), held while the
    /// session is open; None when the lock file couldn't be used.
    _lock: Option<lock::PortLock>,
    /// The port's own key and USB interface, when open_debug had a device:
    /// to find the port again after an unplug, even under another name.
    owner: Option<(DeviceKey, Option<u8>)>,
    /// Set while program_board writes the board: the port stays closed,
    /// and reads and writes are refused, until it's done (Pause).
    paused: bool,
}

fn sessions() -> &'static Mutex<BTreeMap<String, Session>> {
    static S: OnceLock<Mutex<BTreeMap<String, Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn with_session<T>(
    id: &str,
    f: impl FnOnce(&mut Session) -> Result<T, ToolError>,
) -> Result<T, ToolError> {
    let mut all = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let s = all.get_mut(id).ok_or_else(|| {
        ToolError::request(format!(
            "session: {id:?} isn't an open debug session. Open one with open_debug."
        ))
    })?;
    f(s)
}

/// The ports of this process's debug sessions.
pub fn session_ports() -> Vec<String> {
    let all = sessions().lock().unwrap_or_else(|e| e.into_inner());
    all.values().map(|s| s.port_name.clone()).collect()
}

/// Pauses this process's sessions on `ports` while program_board writes
/// their board: each closes its port, so the programmer and a restarting
/// board don't meet an open port, and refuses reads and writes. Dropping
/// the guard ends the pause; the next read or write then reopens the port
/// and checks the build ID, as after an unplug.
pub fn pause(ports: &[String]) -> Pause {
    let mut all = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let mut ids = Vec::new();
    for (id, s) in all.iter_mut() {
        if ports.iter().any(|p| lock::same_port(p, &s.port_name)) {
            s.port = None;
            s.paused = true;
            ids.push(id.clone());
            log::info!("Debug session {id} paused for program_board");
        }
    }
    Pause { ids }
}

/// Pauses this process's sessions on `device`'s board while program_board
/// writes it: the sessions opened with that device, and those on one of
/// its serial ports.
pub fn pause_device(device: &crate::devices::Device) -> Pause {
    let key = DeviceKey::of(device);
    let mut ports: Vec<String> = device.serial_ports.iter().map(|p| p.port.clone()).collect();
    ports.extend(device.serial_port.clone());
    let mut all = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let mut ids = Vec::new();
    for (id, s) in all.iter_mut() {
        let owned = key.is_some() && s.owner.as_ref().map(|(k, _)| k) == key.as_ref();
        if owned || ports.iter().any(|p| lock::same_port(p, &s.port_name)) {
            s.port = None;
            s.paused = true;
            ids.push(id.clone());
            log::info!("Debug session {id} paused for program_board");
        }
    }
    Pause { ids }
}

/// Paused sessions, resumed when dropped.
pub struct Pause {
    ids: Vec<String>,
}

impl Drop for Pause {
    fn drop(&mut self) {
        let mut all = sessions().lock().unwrap_or_else(|e| e.into_inner());
        for id in &self.ids {
            if let Some(s) = all.get_mut(id) {
                s.paused = false;
            }
        }
    }
}

/// A frame to send: its command and payload.
type Frame = (u8, Vec<u8>);

/// The most bytes sent in one write. The debugger's receive buffer holds 64
/// while it replies to earlier frames, and has no full check: never more
/// than this may be on the way. A larger frame goes alone (the debugger
/// takes in a frame as fast as it arrives).
const MAX_BATCH: usize = 48;

/// Sends frames and returns each reply's payload, in order, checking their
/// status. The frames go out together, up to MAX_BATCH bytes a write, so a
/// batch pays the link's round trip once: USB serial chips hold received
/// bytes for a while (an FTDI chip, such as the Alhambra II's, for 16 ms).
/// Opens the port again first when the link broke; a link error drops the
/// port, so the next call tries again (the board unplugged and back).
fn transact_many(s: &mut Session, frames: &[Frame]) -> Result<Vec<Vec<u8>>, ToolError> {
    if s.paused {
        return Err(ToolError::setup(format!(
            "The board on {} is being programmed (program_board): the session reconnects when it's done.",
            s.port_name
        )));
    }
    if s.port.is_none() {
        follow_device(s)?;
        let port = serialport::new(&s.port_name, s.map.baud)
            .timeout(Duration::from_millis(20))
            .open()
            .map_err(|e| {
                ToolError::setup(format!(
                    "The board on {} isn't there ({e}). Plug it back in: the session reconnects at the next read or write.",
                    s.port_name
                ))
            })?;
        s.port = Some(port);
        // A board plugged back in has restarted, with nothing overridden.
        s.wr = vec![false; s.map.write_bits];
        s.en = vec![false; s.map.enables];
        s.reconnected = true;
        log::info!("Debug session reopened {}", s.port_name);
    }
    let result = exchange(s, frames);
    if let Err(e) = &result
        && e.category == Category::Setup
    {
        s.port = None;
    }
    result
}

/// With a device, the port's name now: found again by the port's key and
/// interface, as the engine's serial sessions do, since a board plugged
/// back in may get another name. The lock moves to the new name.
fn follow_device(s: &mut Session) -> Result<(), ToolError> {
    let Some((key, interface)) = &s.owner else {
        return Ok(());
    };
    let Some((_, p)) = usb_ports()
        .into_iter()
        .find(|(k, p)| k == key && p.interface == *interface)
    else {
        return Ok(()); // Not back yet: opening the old name says so.
    };
    if lock::same_port(&p.port, &s.port_name) {
        return Ok(());
    }
    let new_lock = lock::take(&p.port).map_err(|h| {
        ToolError::setup(format!(
            "The board is back as {}, which is in a debug session in another program{}.",
            p.port,
            h.process()
        ))
    })?;
    log::info!(
        "Debug session's board moved from {} to {}",
        s.port_name,
        p.port
    );
    s._lock = new_lock;
    s.port_name = p.port;
    Ok(())
}

/// A device ID's USB IDs and serial number (the engine's format,
/// usb:<location>:<vid>:<pid>:<serial>), as the engine's open_serial reads it.
fn device_key(id: &str) -> Result<DeviceKey, ToolError> {
    let parts: Option<Vec<&str>> = id
        .strip_prefix("usb:")
        .map(|rest| rest.splitn(4, ':').collect());
    let bad = || {
        ToolError::request(format!(
            "device: {id:?} isn't a device ID. Use the id of a device from list_devices."
        ))
    };
    let Some([_location, vid, pid, serial]) = parts.as_deref() else {
        return Err(bad());
    };
    Ok(DeviceKey {
        vid: u16::from_str_radix(vid, 16).map_err(|_| bad())?,
        pid: u16::from_str_radix(pid, 16).map_err(|_| bad())?,
        serial_number: (!serial.is_empty()).then(|| serial.to_string()),
    })
}

/// One frame and its reply's payload.
fn transact(s: &mut Session, command: u8, payload: &[u8]) -> Result<Vec<u8>, ToolError> {
    Ok(transact_many(s, &[(command, payload.to_vec())])?.remove(0))
}

/// Frames and their replies, on the open port. A refused frame doesn't stop
/// the rest of its batch from being read, so no late reply is left on the
/// line for the next request.
fn exchange(s: &mut Session, frames: &[Frame]) -> Result<Vec<Vec<u8>>, ToolError> {
    let name = s.port_name.clone();
    let Some(port) = s.port.as_mut() else {
        return Err(ToolError::engine("The debug session has no open port."));
    };
    let link = |e: String| {
        ToolError::new(
            Category::Setup,
            format!("The debugger on {name} didn't answer: {e}"),
        )
        .with_details(
            "Check that the board runs this debug build (program_board), and that no other \
             program, such as a serial session, uses the port.",
        )
    };
    let _ = port.clear(serialport::ClearBuffer::Input);
    let mut payloads = Vec::with_capacity(frames.len());
    let mut refused: Option<ToolError> = None;
    let mut next = 0;
    while next < frames.len() {
        // As many frames as fit in one write.
        let mut bytes = Vec::new();
        let mut commands = Vec::new();
        while next < frames.len() {
            let f = protocol::frame(frames[next].0, &frames[next].1);
            if !commands.is_empty() && bytes.len() + f.len() > MAX_BATCH {
                break;
            }
            bytes.extend(f);
            commands.push(frames[next].0);
            next += 1;
        }
        port.write_all(&bytes)
            .and_then(|_| port.flush())
            .map_err(|e| link(e.to_string()))?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 256];
        for command in commands {
            loop {
                match protocol::parse_reply(&buf).map_err(link)? {
                    Some((reply, used)) => {
                        buf.drain(..used);
                        if reply.command != command {
                            return Err(link(format!(
                                "it answered command {:02x} to command {command:02x}",
                                reply.command
                            )));
                        }
                        if reply.status != 0 && refused.is_none() {
                            refused = Some(ToolError::engine(format!(
                                "The debugger refused command {command:02x}: {}.",
                                protocol::status_text(reply.status)
                            )));
                        }
                        payloads.push(reply.payload);
                        break;
                    }
                    None if Instant::now() >= deadline => {
                        return Err(link(format!(
                            "no reply in {} ms",
                            REPLY_TIMEOUT.as_millis()
                        )));
                    }
                    None => match port.read(&mut chunk) {
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(e)
                            if e.kind() == ErrorKind::TimedOut
                                || e.kind() == ErrorKind::Interrupted => {}
                        Err(e) => return Err(link(e.to_string())),
                    },
                }
            }
        }
    }
    match refused {
        Some(e) => Err(e),
        None => Ok(payloads),
    }
}

/// The WRITE frame that sets the write values and the enables to the
/// session's copy.
fn write_frame(s: &Session) -> Frame {
    let bits: Vec<bool> = s.wr.iter().chain(&s.en).copied().collect();
    (protocol::WRITE, protocol::pack(&bits))
}

/// The READ frame.
fn read_frame() -> Frame {
    (protocol::READ, vec![])
}

/// Takes a READ reply: the read nets' bytes, packed, for get_bits; the
/// write values and the enables go into the session's copy, so what it
/// reports is the board's.
fn take_read(s: &mut Session, reply: &[u8]) -> Result<Vec<u8>, ToolError> {
    if reply.len() != s.map.read_reply_bytes() {
        return Err(ToolError::engine(
            "A debugger READ reply has the wrong length.",
        ));
    }
    let (r, w, e) = (s.map.read_bits, s.map.write_bits, s.map.enables);
    s.wr = protocol::get_bits(reply, r, w);
    s.en = protocol::get_bits(reply, r + w, e);
    Ok(protocol::pack(&protocol::get_bits(reply, 0, r)))
}

/// Releases every override and clears the write values, in one frame.
fn reset(s: &mut Session) -> Result<(), ToolError> {
    s.en = vec![false; s.map.enables];
    s.wr = vec![false; s.map.write_bits];
    let frame = write_frame(s);
    transact_many(s, &[frame]).map(|_| ())
}

/// Asks the debugger for its build: Ok(Ok(id)) when it is the session's
/// map's build, Ok(Err(id)) when it is another.
fn board_build(s: &mut Session) -> Result<Result<String, String>, ToolError> {
    let info = transact(s, protocol::INFO, &[])?;
    info_build(s, &info)
}

/// An INFO reply's build: Ok(id) when it is the session's map's, Err(id)
/// when it is another.
fn info_build(s: &Session, info: &[u8]) -> Result<Result<String, String>, ToolError> {
    if info.len() != protocol::INFO_BYTES {
        return Err(ToolError::engine(
            "The debugger's INFO reply has the wrong length.",
        ));
    }
    let id = format!(
        "{:08x}",
        u32::from_le_bytes([info[1], info[2], info[3], info[4]])
    );
    // The build ID covers the map, sizes and all.
    let same = info[0] == protocol::PROTOCOL_VERSION && id == s.map.build_id;
    Ok(if same { Ok(id) } else { Err(id) })
}

/// Before each read and write: the board may have been programmed with
/// another build since open_debug, and the map would then be wrong.
fn same_build(s: &mut Session) -> Result<(), ToolError> {
    let build = board_build(s)?;
    build_ok(s, build)
}

/// The board's build checked against the session's map, from an INFO
/// reply: an error when it is another build. After a reconnect, a matching
/// board is reset to match the session's copy.
fn build_ok(s: &mut Session, build: Result<String, String>) -> Result<(), ToolError> {
    match build {
        Ok(_) => {
            if s.reconnected {
                s.reconnected = false;
                reset(s)?;
            }
            Ok(())
        }
        Err(board_id) => Err(ToolError::new(
            Category::Setup,
            format!(
                "The board now runs another build (ID {board_id}), not this session's (ID {}): it was programmed after open_debug. Close this session, and open one with the build folder of the build on the board.",
                s.map.build_id
            ),
        )
        .with_data("board_build_id", &board_id)
        .with_data("map_build_id", &s.map.build_id)),
    }
}

// --- open_debug -------------------------------------------------------------

pub struct DebugOpen;

impl EngineTool for DebugOpen {
    const NAME: &'static str = "open_debug";
    const DESCRIPTION: &'static str = "Opens a debug session with the Logix debugger on a board, \
        for a build made by build with debug_file and programmed with program_board. Give the build \
        folder (its debug_map.json names the nets) and the board's serial port (a device's \
        serial_port from list_devices). Give device too (its id from list_devices): then the \
        session finds the port again when the board is plugged back in, even under another \
        name. Checks that the board runs that very build, by its build ID, and refuses \
        another. Opening releases every override, so the design runs on \
        its own. The debugger owns the port while the session is open: don't open a serial \
        session on it too. program_board on the board pauses the session, which reconnects \
        after and checks the build again.";
    const READ_ONLY: bool = false;
    type Input = DebugOpenInput;
    type Output = DebugOpenOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugOpenInput {
    /// The debug build's folder, as an absolute path.
    pub build_folder: String,
    /// The serial port, such as /dev/cu.usbmodem1102 or COM5.
    pub port: String,
    /// The device the port belongs to, by its id from list_devices, so the
    /// session finds the port again after the board is unplugged, even
    /// under another name. Omitted means the port alone.
    #[serde(default)]
    pub device: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugOpenOutput {
    /// The session's ID, for the other debug tools.
    pub session: String,
    pub port: String,
    /// The device given, whose port the session follows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// The build ID the board's debugger reported, which matches the map.
    pub build_id: String,
    /// What can be driven: each driven net's name and width.
    pub drive: Vec<NetInfo>,
    /// What can be watched: each watched net's name and width.
    pub watch: Vec<NetInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NetInfo {
    pub name: String,
    pub net: String,
    pub width: usize,
    /// For watched nets: the debug file's format for showing it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

static NEXT: AtomicU32 = AtomicU32::new(1);

pub fn open(input: DebugOpenInput) -> Result<DebugOpenOutput, ToolError> {
    let folder = PathBuf::from(&input.build_folder);
    if !folder.is_absolute() {
        return Err(ToolError::request("build_folder must be an absolute path."));
    }
    let map_path = folder.join(protocol::MAP_FILE);
    let text = std::fs::read_to_string(&map_path).map_err(|e| {
        ToolError::request(format!(
            "Can't read {}: {e}. Build into this folder with build and debug_file first.",
            map_path.display()
        ))
    })?;
    let map: DebugMap = serde_json::from_str(&text).map_err(|e| {
        ToolError::request(format!("{} isn't a debug map: {e}", map_path.display()))
    })?;
    if map.format != protocol::MAP_FORMAT || map.protocol != protocol::PROTOCOL_VERSION {
        return Err(ToolError::request(format!(
            "{} is a {} map for protocol {}; this Logix Engine reads {} for protocol {}. Build again with build and debug_file.",
            map_path.display(),
            map.format,
            map.protocol,
            protocol::MAP_FORMAT,
            protocol::PROTOCOL_VERSION
        )));
    }
    // The port's own key and USB interface, as the serial port list reports
    // them (its serial number may differ from the device ID's), as the
    // engine's open_serial finds them.
    let owner = match &input.device {
        Some(id) => {
            let device = device_key(id)?;
            usb_ports()
                .into_iter()
                .find(|(k, p)| k.vid == device.vid && k.pid == device.pid && p.port == input.port)
                .map(|(k, p)| (k, p.interface))
        }
        None => None,
    };
    {
        let all = sessions().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((id, _)) = all
            .iter()
            .find(|(_, s)| lock::same_port(&s.port_name, &input.port))
        {
            return Err(ToolError::request(format!(
                "Debug session {id} already has {}: use it, or close it first.",
                input.port
            )));
        }
    }
    let port_lock = lock::take(&input.port).map_err(|h| {
        ToolError::setup(format!(
            "{} is in a debug session in another program{}.",
            input.port,
            h.process()
        ))
        .with_details(
            "Close that session first: press Enter in the terminal of debug_run.py or \
             debug_panel.py, or call close_debug in the program that opened it.",
        )
    })?;
    let port = serialport::new(&input.port, map.baud)
        .timeout(Duration::from_millis(20))
        .open()
        .map_err(|e| {
            ToolError::setup(format!("Can't open {}: {e}", input.port)).with_details(
                "Check the port with list_devices, and that no serial session (close_serial) \
                 or other program has it.",
            )
        })?;
    let mut s = Session {
        port: Some(port),
        reconnected: false,
        port_name: input.port.clone(),
        wr: vec![false; map.write_bits],
        en: vec![false; map.enables],
        map,
        _lock: port_lock,
        owner,
        paused: false,
    };
    let board_id = match board_build(&mut s)? {
        Ok(id) => id,
        Err(board_id) => return Err(ToolError::new(
            Category::Setup,
            format!(
                "The board runs another debug build (ID {board_id}), not the one in {} (ID {}). Program this build with program_board, or open the build folder of the one on the board.",
                input.build_folder, s.map.build_id
            ),
        )
        .with_data("board_build_id", &board_id)
        .with_data("map_build_id", &s.map.build_id)),
    };
    reset(&mut s)?;
    let out = DebugOpenOutput {
        session: String::new(),
        port: input.port.clone(),
        device: input.device.clone(),
        build_id: board_id,
        watch: s
            .map
            .watch
            .iter()
            .map(|i| NetInfo {
                name: i.name.clone(),
                net: i.net.clone(),
                width: i.width,
                format: i.format.clone(),
            })
            .collect(),
        drive: s
            .map
            .drive
            .iter()
            .map(|o| NetInfo {
                name: o.name.clone(),
                net: o.net.clone(),
                width: o.width,
                format: None,
            })
            .collect(),
    };
    let id = format!("debug{}", NEXT.fetch_add(1, Ordering::Relaxed));
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), s);
    Ok(DebugOpenOutput { session: id, ..out })
}

// --- read_debug -------------------------------------------------------------

pub struct DebugRead;

impl EngineTool for DebugRead {
    const NAME: &'static str = "read_debug";
    const DESCRIPTION: &'static str = "Reads nets of the running design through a debug \
        session, all in one snapshot taken in a single clock cycle. Give the watched names to \
        read (from open_debug), or none for all. Each value comes as bits (most significant \
        first), as hex and, up to 53 bits, as a number.";
    const READ_ONLY: bool = true;
    type Input = DebugReadInput;
    type Output = DebugReadOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugReadInput {
    /// The session, from open_debug.
    pub session: String,
    /// The watched nets to read, by name. Omitted means all.
    #[serde(default)]
    pub names: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugReadOutput {
    pub values: Vec<NetValue>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NetValue {
    pub name: String,
    pub net: String,
    pub width: usize,
    /// The value as a number, when it has 53 bits or fewer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<u64>,
    /// The value in hex, such as 0x5b8d7f.
    pub hex: String,
    /// The bits, most significant first.
    pub bits: String,
}

pub fn read(input: DebugReadInput) -> Result<DebugReadOutput, ToolError> {
    with_session(&input.session, |s| {
        let wanted: Vec<protocol::MapWatch> = match &input.names {
            None => s.map.watch.clone(),
            Some(names) => names
                .iter()
                .map(|n| {
                    s.map
                        .watch
                        .iter()
                        .find(|i| i.name == *n)
                        .cloned()
                        .ok_or_else(|| {
                            let known: Vec<&str> =
                                s.map.watch.iter().map(|i| i.name.as_str()).collect();
                            ToolError::request(format!(
                                "names: {n:?} isn't watched in this build. It watches: {}.",
                                known.join(", ")
                            ))
                        })
                })
                .collect::<Result<_, _>>()?,
        };
        // The build check and the snapshot, in one batch.
        let frames: Vec<Frame> = vec![(protocol::INFO, vec![]), read_frame()];
        let replies = transact_many(s, &frames)?;
        let build = info_build(s, &replies[0])?;
        build_ok(s, build)?;
        let bytes = take_read(s, &replies[1])?;
        Ok(DebugReadOutput {
            values: wanted
                .into_iter()
                .map(|i| {
                    let bits = protocol::get_bits(&bytes, i.offset, i.width);
                    net_value(&i.name, &i.net, &bits)
                })
                .collect(),
        })
    })
}

fn net_value(name: &str, net: &str, bits: &[bool]) -> NetValue {
    let msb_first: String = bits
        .iter()
        .rev()
        .map(|&b| if b { '1' } else { '0' })
        .collect();
    let mut hex = String::new();
    for chunk in bits.chunks(4).rev() {
        let nibble = chunk
            .iter()
            .enumerate()
            .fold(0u8, |a, (k, &b)| a | ((b as u8) << k));
        hex.push(char::from_digit(nibble as u32, 16).unwrap_or('0'));
    }
    let value = (bits.len() <= 53).then(|| {
        bits.iter()
            .enumerate()
            .fold(0u64, |a, (k, &b)| a | ((b as u64) << k))
    });
    NetValue {
        name: name.to_string(),
        net: net.to_string(),
        width: bits.len(),
        value,
        hex: format!("0x{hex}"),
        bits: msb_first,
    }
}

// --- write_debug ------------------------------------------------------------

pub struct DebugWrite;

impl EngineTool for DebugWrite {
    const NAME: &'static str = "write_debug";
    const DESCRIPTION: &'static str = "Drives a net of the running design through a debug \
        session: one of its drive names from open_debug. Writing takes the net over from the \
        design until release; release gives it back. The value is a number, or a string in hex (0x..), binary (0b..) \
        or decimal, and must fit the net's width. pulse, for a one-bit net, flips its value for one \
        clock, whether it is driven or released, and leaves it as it was.";
    const READ_ONLY: bool = false;
    type Input = DebugWriteInput;
    type Output = DebugWriteOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugWriteInput {
    /// The session, from open_debug.
    pub session: String,
    /// The net to drive, by its name in the debug file's drive list.
    pub name: String,
    /// The value to drive. Required unless release is true.
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// Give an override net back to the design. Defaults to false.
    #[serde(default)]
    pub release: bool,
    /// For a one-bit net: flip its value for one clock, driven or released,
    /// then leave it as it was. Defaults to false.
    #[serde(default)]
    pub pulse: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugWriteOutput {
    pub name: String,
    /// True while the debugger drives the net.
    pub driven: bool,
    /// The value the debugger drives, read back from it, in hex; omitted
    /// when it doesn't drive the net.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hex: Option<String>,
}

pub fn write(input: DebugWriteInput) -> Result<DebugWriteOutput, ToolError> {
    with_session(&input.session, |s| {
        same_build(s)?;
        let o = s
            .map
            .drive
            .iter()
            .find(|o| o.name == input.name)
            .cloned()
            .ok_or_else(|| {
                let known: Vec<&str> = s.map.drive.iter().map(|o| o.name.as_str()).collect();
                ToolError::request(format!(
                    "name: {:?} isn't driven in this build. It drives: {}.",
                    input.name,
                    if known.is_empty() {
                        "none".into()
                    } else {
                        known.join(", ")
                    }
                ))
            })?;
        let bit = o.enable_bit;
        if input.pulse {
            if input.value.is_some() || input.release {
                return Err(ToolError::request("Give one of value, release and pulse."));
            }
            if o.width != 1 {
                return Err(ToolError::request(format!(
                    "{} is {} bits: only a one-bit net can pulse.",
                    o.name, o.width
                )));
            }
            // The pulse and the read-back, in one batch.
            let mut mask = vec![false; s.map.enables];
            mask[bit] = true;
            let frames: Vec<Frame> = vec![(protocol::PULSE, protocol::pack(&mask)), read_frame()];
            let replies = transact_many(s, &frames)?;
            take_read(s, &replies[1])?;
            let driven = s.en[bit];
            let held = s.wr[o.offset..o.offset + o.width].to_vec();
            return Ok(DebugWriteOutput {
                name: o.name.clone(),
                driven,
                hex: driven.then(|| net_value(&o.name, &o.net, &held).hex),
            });
        }
        if input.release {
            if input.value.is_some() {
                return Err(ToolError::request("Give value or release, not both."));
            }
            s.en[bit] = false;
            // The release and the read-back, in one batch.
            let frames = vec![write_frame(s), read_frame()];
            let replies = transact_many(s, &frames)?;
            take_read(s, &replies[1])?;
            let driven = s.en[bit];
            return Ok(DebugWriteOutput {
                name: o.name,
                driven,
                hex: None,
            });
        }
        let Some(value) = &input.value else {
            return Err(ToolError::request(
                "value is required, unless release is true.",
            ));
        };
        let bits = parse_value(value, o.width)?;
        s.wr[o.offset..o.offset + o.width].copy_from_slice(&bits);
        s.en[bit] = true;
        // The value and the take-over (one WRITE: both in the same clock),
        // and the read-back, in one batch.
        let frames = vec![write_frame(s), read_frame()];
        let replies = transact_many(s, &frames)?;
        // What the debugger now holds.
        take_read(s, &replies[1])?;
        let held = s.wr[o.offset..o.offset + o.width].to_vec();
        let driven = s.en[bit];
        Ok(DebugWriteOutput {
            name: o.name.clone(),
            driven,
            hex: driven.then(|| net_value(&o.name, &o.net, &held).hex),
        })
    })
}

/// A value for a net of `width` bits, as bits, least significant first.
fn parse_value(value: &serde_json::Value, width: usize) -> Result<Vec<bool>, ToolError> {
    let bad = |why: String| ToolError::request(format!("value: {why}"));
    let text = match value {
        serde_json::Value::Number(n) => n
            .as_u64()
            .map(|v| v.to_string())
            .ok_or_else(|| bad(format!("{n} isn't a whole number of 0 or more")))?,
        serde_json::Value::String(s) => s.trim().replace('_', ""),
        serde_json::Value::Bool(b) => (*b as u8).to_string(),
        other => return Err(bad(format!("{other} isn't a number or a string"))),
    };
    let lower = text.to_ascii_lowercase();
    let (digits, radix) = if let Some(h) = lower.strip_prefix("0x") {
        (h, 16)
    } else if let Some(b) = lower.strip_prefix("0b") {
        (b, 2)
    } else {
        (lower.as_str(), 10)
    };
    if digits.is_empty() {
        return Err(bad(format!("{text:?} has no digits")));
    }
    // Bits, least significant first, for any width.
    let mut bits: Vec<bool> = Vec::new();
    if radix == 10 {
        let v: u128 = digits
            .parse()
            .map_err(|_| bad(format!("{text:?} isn't a number")))?;
        let mut v = v;
        while v > 0 {
            bits.push(v & 1 == 1);
            v >>= 1;
        }
    } else {
        let per = if radix == 16 { 4 } else { 1 };
        for c in digits.chars().rev() {
            let d = c
                .to_digit(radix)
                .ok_or_else(|| bad(format!("{text:?} has the digit {c:?}")))?;
            for k in 0..per {
                bits.push(d >> k & 1 == 1);
            }
        }
    }
    while bits.last() == Some(&false) {
        bits.pop();
    }
    if bits.len() > width {
        return Err(bad(format!(
            "{text} needs {} bits, and the net has {width}",
            bits.len()
        )));
    }
    bits.resize(width, false);
    Ok(bits)
}

// --- close_debug ------------------------------------------------------------

pub struct DebugClose;

impl EngineTool for DebugClose {
    const NAME: &'static str = "close_debug";
    const DESCRIPTION: &'static str = "Ends a debug session: releases every override, so the \
        design runs on its own, and frees the serial port.";
    const READ_ONLY: bool = false;
    type Input = DebugCloseInput;
    type Output = DebugCloseOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugCloseInput {
    /// The session, from open_debug.
    pub session: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DebugCloseOutput {
    /// False when the overrides couldn't be released, such as with the
    /// board unplugged.
    pub released: bool,
}

pub fn close(input: DebugCloseInput) -> Result<DebugCloseOutput, ToolError> {
    let mut all = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let mut s = all.remove(&input.session).ok_or_else(|| {
        ToolError::request(format!(
            "session: {:?} isn't an open debug session.",
            input.session
        ))
    })?;
    drop(all);
    let released = reset(&mut s).is_ok();
    Ok(DebugCloseOutput { released })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse_in_three_bases() {
        let v = |j: serde_json::Value, w| parse_value(&j, w);
        assert_eq!(
            v(serde_json::json!(5), 4).unwrap(),
            vec![true, false, true, false]
        );
        assert_eq!(
            v(serde_json::json!("0x5"), 3).unwrap(),
            vec![true, false, true]
        );
        assert_eq!(
            v(serde_json::json!("0b1_01"), 3).unwrap(),
            vec![true, false, true]
        );
        assert!(v(serde_json::json!(8), 3).is_err());
        assert!(v(serde_json::json!("0xg"), 8).is_err());
    }

    #[test]
    fn values_show_three_ways() {
        let n = net_value("c", "c", &[true, false, true, true, true]);
        assert_eq!(n.bits, "11101");
        assert_eq!(n.hex, "0x1d");
        assert_eq!(n.value, Some(29));
    }
}

// --- For the panel ----------------------------------------------------------

/// A session's driven nets as the host last set them, and its build, for the
/// debug panel.
#[derive(Debug, Serialize)]
pub struct SessionInfo {
    pub session: String,
    pub port: String,
    pub build_id: String,
    pub board: String,
    pub top_module: String,
    /// True when the driven state was just read back from the board;
    /// false when it couldn't be, and it is what the host last wrote.
    pub verified: bool,
    pub drive: Vec<OutputState>,
    pub watch: Vec<NetInfo>,
}

#[derive(Debug, Serialize)]
pub struct OutputState {
    pub name: String,
    pub net: String,
    pub width: usize,
    /// True while the debugger drives it.
    pub driven: bool,
    /// The value the debugger holds for it, in hex and as bits (most
    /// significant first); read back from the board when `verified`.
    pub hex: String,
    pub bits: String,
    /// The debug file's format for showing it, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

pub fn info(id: &str) -> Result<SessionInfo, ToolError> {
    with_session(id, |s| {
        // The build check and the read-back, in one batch.
        let verified = (|| {
            let frames: Vec<Frame> = vec![(protocol::INFO, vec![]), read_frame()];
            let replies = transact_many(s, &frames)?;
            let build = info_build(s, &replies[0])?;
            build_ok(s, build)?;
            take_read(s, &replies[1]).map(|_| ())
        })()
        .is_ok();
        Ok(SessionInfo {
            verified,
            session: id.to_string(),
            port: s.port_name.clone(),
            build_id: s.map.build_id.clone(),
            board: s.map.board.clone(),
            top_module: s.map.top_module.clone(),
            watch: s
                .map
                .watch
                .iter()
                .map(|i| NetInfo {
                    name: i.name.clone(),
                    net: i.net.clone(),
                    width: i.width,
                    format: i.format.clone(),
                })
                .collect(),
            drive: s
                .map
                .drive
                .iter()
                .map(|o| {
                    let bits = s.wr[o.offset..o.offset + o.width].to_vec();
                    OutputState {
                        name: o.name.clone(),
                        net: o.net.clone(),
                        width: o.width,
                        driven: s.en[o.enable_bit],
                        hex: net_value(&o.name, &o.net, &bits).hex,
                        bits: bits
                            .iter()
                            .rev()
                            .map(|&b| if b { '1' } else { '0' })
                            .collect(),
                        format: o.format.clone(),
                    }
                })
                .collect(),
        })
    })
}

/// The open sessions' IDs.
pub fn ids() -> Vec<String> {
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect()
}
