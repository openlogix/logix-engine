//! Serial ports (docs/tool-api.md, Serial ports; docs/requirements.md,
//! Serial Monitor): finding a connected board's serial ports, and serial
//! sessions, for the open_serial, read_serial, write_serial and
//! close_serial tools.
//!
//! A session is the engine's one piece of state that outlives a request
//! (the user's decision, 2026-11-19, for the Serial Monitor): a port stays
//! open between calls, read by a thread of its own into a buffer that
//! read_serial empties. Sessions live until close_serial or until the
//! engine exits. While program_board writes a board, its sessions are
//! paused (the port closed) and reopened after; a board that's unplugged
//! is reopened when it's back, found again by its USB IDs, serial number
//! and USB interface, since the port's name may change.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::devices::Device;
use crate::errors::{ActionCode, ToolError};

/// One serial port of a connected USB device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DevicePort {
    /// The port's name, to pass to open_serial: such as COM5 on Windows,
    /// /dev/cu.usbserial-A10 on macOS, /dev/ttyUSB1 on Linux.
    pub port: String,
    /// The USB interface it's on, when the system says, such as 1 for an
    /// FTDI chip's second channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<u8>,
    /// What it's for, by the device's board (list_devices): uart, the
    /// board's serial port, to open; programmer, the interface its
    /// programmer uses (such as an FTDI chip's first channel), not a serial
    /// port for the design. Omitted when not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<PortRole>,
    /// The port for people, in a phrase: "The Alhambra II's serial port",
    /// "The Alhambra II's programming port", or "A serial port of <the
    /// device>". Empty until list_devices names it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// What a serial port is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PortRole {
    /// The board's UART: the one to open.
    Uart,
    /// The interface the board's programmer uses.
    Programmer,
}

/// A port's phrase for people, by its role and the device's `who` (such as
/// "the Alhambra II").
pub fn port_description(role: Option<PortRole>, who: &str) -> String {
    let mut chars = who.chars();
    let who_cap = match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    match role {
        Some(PortRole::Uart) => format!("{who_cap}'s serial port"),
        Some(PortRole::Programmer) => format!("{who_cap}'s programming port"),
        None => format!("A serial port of {who}"),
    }
}

/// What identifies a USB device across unplugging: its USB IDs and serial
/// number (its bus address and port name may change).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceKey {
    pub vid: u16,
    pub pid: u16,
    pub serial_number: Option<String>,
}

impl DeviceKey {
    pub fn of(device: &Device) -> Option<DeviceKey> {
        Some(DeviceKey {
            vid: u16::from_str_radix(&device.vid, 16).ok()?,
            pid: u16::from_str_radix(&device.pid, 16).ok()?,
            serial_number: device.serial_number.clone(),
        })
    }
}

/// The system's serial ports that are on a USB device, with that device's
/// key (system ports and Bluetooth ports are left out).
pub fn usb_ports() -> Vec<(DeviceKey, DevicePort)> {
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };
    ports
        .into_iter()
        .filter_map(|p| match p.port_type {
            serialport::SerialPortType::UsbPort(u) => Some((
                DeviceKey {
                    vid: u.vid,
                    pid: u.pid,
                    serial_number: u.serial_number,
                },
                DevicePort {
                    port: macos_callout(p.port_name),
                    interface: u.interface,
                    role: None,
                    description: String::new(),
                },
            )),
            _ => None,
        })
        .collect()
}

/// On macOS, the callout device (/dev/cu.*), which opens without waiting
/// for a modem's carrier line, rather than the dial-in one (/dev/tty.*).
fn macos_callout(name: String) -> String {
    match name.strip_prefix("/dev/tty.") {
        Some(rest) if cfg!(target_os = "macos") => format!("/dev/cu.{rest}"),
        _ => name,
    }
}

/// The serial ports of each device in `devices`, by index, from the
/// system's USB ports: those with the device's USB IDs and serial number.
/// A device with no serial number gets ports only when no other listed
/// device has its USB IDs, since they couldn't be told apart.
pub fn ports_of_devices(
    devices: &[Device],
    ports: &[(DeviceKey, DevicePort)],
) -> Vec<Vec<DevicePort>> {
    let keys: Vec<Option<DeviceKey>> = devices.iter().map(DeviceKey::of).collect();
    keys.iter()
        .map(|key| {
            let Some(key) = key else {
                return Vec::new();
            };
            let twins = keys
                .iter()
                .flatten()
                .filter(|k| k.vid == key.vid && k.pid == key.pid)
                .count();
            if key.serial_number.is_none() && twins > 1 {
                return Vec::new();
            }
            let mut found: Vec<DevicePort> = ports
                .iter()
                .filter(|(k, _)| {
                    k.vid == key.vid
                        && k.pid == key.pid
                        && (key.serial_number.is_none() || k.serial_number == key.serial_number)
                })
                .map(|(_, p)| p.clone())
                .collect();
            found.sort_by(|a, b| (a.interface, &a.port).cmp(&(b.interface, &b.port)));
            found.dedup();
            found
        })
        .collect()
}

/// The port of a board's UART among a device's ports: the one on the
/// UART's USB interface when the board's data gives it, else the device's
/// only port. None when it can't be told.
pub fn uart_port(ports: &[DevicePort], usb_interface: Option<u8>) -> Option<String> {
    match usb_interface {
        Some(i) => ports
            .iter()
            .find(|p| p.interface == Some(i))
            .map(|p| p.port.clone()),
        None if ports.len() == 1 => Some(ports[0].port.clone()),
        None => None,
    }
}

// ---- Sessions ----

/// Something that happened to a session's port, in the order it happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SerialEvent {
    /// What happened: paused (program_board is writing the board, so the
    /// port is closed), resumed (reopened after that), disconnected (the
    /// port went away, such as the board being unplugged), reconnected
    /// (reopened when it came back), or overflow (data arrived faster than
    /// it was read, and the oldest was dropped).
    pub kind: SerialEventKind,
    /// The same for people, such as "Closing the serial port while the
    /// board is programmed.".
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SerialEventKind {
    Paused,
    Resumed,
    Disconnected,
    Reconnected,
    Overflow,
}

/// What a session's reader has gathered since the last read.
#[derive(Default)]
struct Inbox {
    bytes: Vec<u8>,
    events: Vec<SerialEvent>,
    overflowed: bool,
}

/// The most a session keeps unread; older data is dropped first.
const INBOX_LIMIT: usize = 1 << 20;

/// How long the reader waits for data in one read, and so how soon it
/// notices a pause or a close.
const READ_SLICE: Duration = Duration::from_millis(20);

/// How often a closed port is tried again.
const RETRY: Duration = Duration::from_millis(300);

struct Session {
    id: String,
    baud: u32,
    /// The port's name now: it may change when the board comes back.
    port_name: Mutex<String>,
    /// The device it belongs to, when open_serial named one, and the USB
    /// interface the port is on: how to find the port again.
    device: Option<DeviceKey>,
    interface: Option<u8>,
    /// The open port's handle for writing (the reader has its own); None
    /// while paused or gone.
    port: Mutex<Option<Box<dyn serialport::SerialPort>>>,
    inbox: Mutex<Inbox>,
    arrived: tokio::sync::Notify,
    /// How many program_board calls are pausing it now.
    pauses: AtomicUsize,
    closed: AtomicBool,
}

impl Session {
    fn push_event(&self, kind: SerialEventKind, message: &str) {
        log::info!("Serial session {}: {message}", self.id);
        self.inbox.lock().unwrap().events.push(SerialEvent {
            kind,
            message: message.to_string(),
        });
        self.arrived.notify_waiters();
    }

    fn push_bytes(&self, data: &[u8]) {
        let mut inbox = self.inbox.lock().unwrap();
        inbox.bytes.extend_from_slice(data);
        if inbox.bytes.len() > INBOX_LIMIT {
            let extra = inbox.bytes.len() - INBOX_LIMIT;
            inbox.bytes.drain(..extra);
            if !inbox.overflowed {
                inbox.overflowed = true;
                inbox.events.push(SerialEvent {
                    kind: SerialEventKind::Overflow,
                    message: "Data arrived faster than it was read: the oldest was dropped.".into(),
                });
            }
        }
        drop(inbox);
        self.arrived.notify_waiters();
    }

    fn is_open(&self) -> bool {
        self.port.lock().unwrap().is_some()
    }

    /// The port's name now: found again by the device's key and interface
    /// when the session has a device, else the name it was opened with.
    fn current_name(&self) -> String {
        if let Some(key) = &self.device
            && let Some((_, p)) = usb_ports()
                .into_iter()
                .find(|(k, p)| k == key && p.interface == self.interface)
        {
            *self.port_name.lock().unwrap() = p.port.clone();
            return p.port;
        }
        self.port_name.lock().unwrap().clone()
    }
}

fn open_port(name: &str, baud: u32) -> serialport::Result<Box<dyn serialport::SerialPort>> {
    serialport::new(name, baud).timeout(READ_SLICE).open()
}

/// The reader: moves what arrives into the inbox, closes the port while
/// paused, and reopens it when the pause ends or the port comes back.
///
/// It reads its own handle to the port (`reader`), and write_serial uses
/// another (`Session::port`, a clone): with one handle under one lock, a
/// write waited for the reader to let go, and on macOS (whose locks don't
/// queue) that took a second or more per keystroke (the user's report,
/// 2026-11-19).
fn read_loop(s: Arc<Session>, mut reader: Option<Box<dyn serialport::SerialPort>>) {
    let mut buf = [0u8; 4096];
    // Why the port is closed, if it is: paused or gone.
    let mut paused = false;
    let mut gone = false;
    let close = |s: &Session, reader: &mut Option<Box<dyn serialport::SerialPort>>| {
        reader.take();
        s.port.lock().unwrap().take();
    };
    while !s.closed.load(Ordering::SeqCst) {
        if s.pauses.load(Ordering::SeqCst) > 0 {
            if reader.is_some() {
                close(&s, &mut reader);
                s.push_event(
                    SerialEventKind::Paused,
                    "Closing the serial port while the board is programmed.",
                );
            }
            paused = true;
            std::thread::sleep(READ_SLICE);
            continue;
        }
        let Some(port) = reader.as_mut() else {
            let name = s.current_name();
            match open_port(&name, s.baud).and_then(|p| Ok((p.try_clone()?, p))) {
                Ok((writer, p)) => {
                    *s.port.lock().unwrap() = Some(writer);
                    reader = Some(p);
                    if gone {
                        s.push_event(
                            SerialEventKind::Reconnected,
                            &format!("The serial port {name} is back: reopened."),
                        );
                    } else if paused {
                        s.push_event(
                            SerialEventKind::Resumed,
                            &format!("Reopened the serial port {name} after programming."),
                        );
                    }
                    paused = false;
                    gone = false;
                }
                Err(_) => std::thread::sleep(RETRY),
            }
            continue;
        };
        match port.read(&mut buf) {
            Ok(0) => std::thread::sleep(READ_SLICE),
            Ok(n) => s.push_bytes(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                close(&s, &mut reader);
                gone = true;
                s.push_event(
                    SerialEventKind::Disconnected,
                    &format!(
                        "The serial port went away ({e}): it's reopened when the board is back."
                    ),
                );
            }
        }
    }
    close(&s, &mut reader);
}

/// The engine's serial sessions.
pub struct Sessions {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    next: AtomicU64,
}

/// The process's sessions.
pub fn sessions() -> &'static Sessions {
    static SESSIONS: OnceLock<Sessions> = OnceLock::new();
    SESSIONS.get_or_init(|| Sessions {
        sessions: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
    })
}

/// What a read returns.
pub struct ReadResult {
    /// Exactly the bytes received.
    pub bytes: Vec<u8>,
    pub events: Vec<SerialEvent>,
    pub open: bool,
    pub port: String,
}

fn no_session(id: &str) -> ToolError {
    ToolError::request(format!(
        "session: there's no open serial session {id:?}. Open one with open_serial."
    ))
}

impl Sessions {
    /// Opens a port. `device` and its port's `interface` let the session
    /// find the port again after the board is unplugged, and pause it while
    /// program_board writes that board.
    pub fn open(
        &self,
        name: &str,
        baud: u32,
        device: Option<DeviceKey>,
        interface: Option<u8>,
    ) -> Result<(String, String), ToolError> {
        if let Some(other) = self
            .sessions
            .lock()
            .unwrap()
            .values()
            .find(|s| *s.port_name.lock().unwrap() == name)
        {
            return Err(ToolError::request(format!(
                "port: {name} is open already, in session {}. Read it there, or close it first.",
                other.id
            )));
        }
        let port = open_port(name, baud).map_err(|e| open_error(name, &e))?;
        self.start(name, port, baud, device, interface)
    }

    /// Starts a session on a port already open (open; tests give a
    /// pseudo-terminal).
    fn start(
        &self,
        name: &str,
        port: Box<dyn serialport::SerialPort>,
        baud: u32,
        device: Option<DeviceKey>,
        interface: Option<u8>,
    ) -> Result<(String, String), ToolError> {
        // The writer's own handle (read_loop says why).
        let writer = port
            .try_clone()
            .map_err(|e| ToolError::engine(format!("Can't use the serial port {name}: {e}")))?;
        let id = format!("serial-{}", self.next.fetch_add(1, Ordering::SeqCst));
        let session = Arc::new(Session {
            id: id.clone(),
            baud,
            port_name: Mutex::new(name.to_string()),
            device,
            interface,
            port: Mutex::new(Some(writer)),
            inbox: Mutex::new(Inbox::default()),
            arrived: tokio::sync::Notify::new(),
            pauses: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
        });
        let session_for_reader = session.clone();
        std::thread::Builder::new()
            .name(format!("serial {id}"))
            .spawn(move || read_loop(session_for_reader, Some(port)))
            .map_err(|e| ToolError::engine(format!("Can't start reading {name}: {e}")))?;
        self.sessions.lock().unwrap().insert(id.clone(), session);
        log::info!("Serial session {id}: opened {name} at {baud} baud");
        Ok((id, name.to_string()))
    }

    fn get(&self, id: &str) -> Result<Arc<Session>, ToolError> {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| no_session(id))
    }

    /// What arrived since the last read, exactly. Waits up to `wait` for
    /// data or an event (or, with `until`, for those bytes), then returns
    /// what there is.
    pub async fn read(
        &self,
        id: &str,
        wait: Duration,
        until: Option<&[u8]>,
    ) -> Result<ReadResult, ToolError> {
        let s = self.get(id)?;
        let deadline = tokio::time::Instant::now() + wait;
        let mut received = Vec::new();
        let mut events = Vec::new();
        loop {
            // Registered before taking the inbox, so data that arrives in
            // between still wakes the wait below.
            let arrived = s.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            let (bytes, new_events) = {
                let mut inbox = s.inbox.lock().unwrap();
                inbox.overflowed = false;
                (
                    std::mem::take(&mut inbox.bytes),
                    std::mem::take(&mut inbox.events),
                )
            };
            received.extend_from_slice(&bytes);
            events.extend(new_events);
            let done = match until {
                Some(u) => received.windows(u.len()).any(|w| w == u),
                None => !received.is_empty() || !events.is_empty(),
            };
            if done || tokio::time::Instant::now() >= deadline || s.closed.load(Ordering::SeqCst) {
                break;
            }
            if tokio::time::timeout_at(deadline, arrived).await.is_err() {
                continue; // Take what came at the last moment, then stop.
            }
            // Data has started to arrive: give the rest of a line a moment.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(ReadResult {
            bytes: received,
            events,
            open: s.is_open(),
            port: s.port_name.lock().unwrap().clone(),
        })
    }

    /// Sends bytes to the port, exactly as given.
    pub fn write(&self, id: &str, data: &[u8]) -> Result<usize, ToolError> {
        let s = self.get(id)?;
        let mut port = s.port.lock().unwrap();
        let Some(p) = port.as_mut() else {
            let why = if s.pauses.load(Ordering::SeqCst) > 0 {
                "while the board is programmed"
            } else {
                "since the board went away"
            };
            return Err(ToolError::setup(format!(
                "The serial port {} is closed {why}: nothing was sent.",
                s.port_name.lock().unwrap()
            )));
        };
        // No flush: it waits until every byte has left (tcdrain), which a
        // monitor doesn't need.
        p.write_all(data).map_err(|e| {
            ToolError::setup(format!(
                "Can't send to {}: {e}",
                s.port_name.lock().unwrap()
            ))
        })?;
        Ok(data.len())
    }

    /// Closes a session and its port.
    pub fn close(&self, id: &str) -> Result<(), ToolError> {
        let s = self
            .sessions
            .lock()
            .unwrap()
            .remove(id)
            .ok_or_else(|| no_session(id))?;
        s.closed.store(true, Ordering::SeqCst);
        s.port.lock().unwrap().take();
        s.arrived.notify_waiters();
        log::info!("Serial session {id}: closed");
        Ok(())
    }

    /// Pauses the sessions on a device (its key), until the guard is
    /// dropped: program_board, while it writes the board. Waits briefly for
    /// their ports to close.
    pub fn pause(&self, device: &Device) -> PauseGuard {
        let key = DeviceKey::of(device);
        let paused: Vec<Arc<Session>> = self
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.device.is_some() && s.device == key)
            .cloned()
            .collect();
        for s in &paused {
            s.pauses.fetch_add(1, Ordering::SeqCst);
        }
        let start = std::time::Instant::now();
        while paused.iter().any(|s| s.is_open()) && start.elapsed() < Duration::from_millis(500) {
            std::thread::sleep(Duration::from_millis(5));
        }
        PauseGuard { paused }
    }
}

/// Ends a pause when dropped.
pub struct PauseGuard {
    paused: Vec<Arc<Session>>,
}

impl Drop for PauseGuard {
    fn drop(&mut self) {
        for s in &self.paused {
            s.pauses.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn open_error(name: &str, e: &serialport::Error) -> ToolError {
    match e.kind() {
        serialport::ErrorKind::NoDevice => ToolError::setup(format!(
            "Can't open the serial port {name}: it isn't there. Is the board plugged in?"
        ))
        .with_action(
            ActionCode::ConnectBoard,
            "Connect the board by USB, list the devices again and choose its serial port.",
        ),
        serialport::ErrorKind::Io(std::io::ErrorKind::PermissionDenied) if cfg!(target_os = "linux") => {
            ToolError::setup(format!(
                "Can't open the serial port {name}: permission denied. Another program may have it open, or the board's udev rule is missing or older than its serial ports (added 2026-11-19)."
            ))
            .with_action(
                ActionCode::FixDevicePermissions,
                "Close other programs using the port; else add the board's udev rule (install_driver), removing an older one first (uninstall_driver).",
            )
        }
        _ => ToolError::setup(format!(
            "Can't open the serial port {name}: {e}. Another program may have it open: close it, then try again."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::tests::device;

    fn key(vid: u16, pid: u16, serial: Option<&str>) -> DeviceKey {
        DeviceKey {
            vid,
            pid,
            serial_number: serial.map(Into::into),
        }
    }

    fn port(name: &str, interface: Option<u8>) -> DevicePort {
        DevicePort {
            port: name.into(),
            interface,
            role: None,
            description: String::new(),
        }
    }

    #[test]
    fn ports_go_to_their_device() {
        let alhambra = device("0403", "6010", Some("Alhambra II"), Some("A1"), 3);
        let other = device("1a86", "7523", Some("USB Serial"), None, 4);
        let ports = vec![
            (
                key(0x0403, 0x6010, Some("A1")),
                port("/dev/ttyUSB1", Some(1)),
            ),
            (
                key(0x0403, 0x6010, Some("A1")),
                port("/dev/ttyUSB0", Some(0)),
            ),
            (
                key(0x0403, 0x6010, Some("B2")),
                port("/dev/ttyUSB2", Some(1)),
            ),
            (key(0x1a86, 0x7523, None), port("/dev/ttyUSB3", Some(0))),
        ];
        let found = ports_of_devices(&[alhambra, other], &ports);
        assert_eq!(
            found[0],
            vec![port("/dev/ttyUSB0", Some(0)), port("/dev/ttyUSB1", Some(1))]
        );
        assert_eq!(found[1], vec![port("/dev/ttyUSB3", Some(0))]);
    }

    #[test]
    fn devices_without_serial_numbers_that_look_the_same_get_none() {
        let a = device("1a86", "7523", None, None, 4);
        let b = device("1a86", "7523", None, None, 5);
        let ports = vec![(key(0x1a86, 0x7523, None), port("/dev/ttyUSB3", Some(0)))];
        assert_eq!(ports_of_devices(&[a, b], &ports), vec![vec![], vec![]]);
    }

    #[test]
    fn port_descriptions() {
        assert_eq!(
            port_description(Some(PortRole::Uart), "the Alhambra II"),
            "The Alhambra II's serial port"
        );
        assert_eq!(
            port_description(Some(PortRole::Programmer), "the Alhambra II"),
            "The Alhambra II's programming port"
        );
        assert_eq!(
            port_description(None, "USB device (1a86:7523)"),
            "A serial port of USB device (1a86:7523)"
        );
    }

    #[test]
    fn the_uarts_port() {
        let ports = vec![port("COM4", Some(0)), port("COM5", Some(1))];
        assert_eq!(uart_port(&ports, Some(1)).as_deref(), Some("COM5"));
        assert_eq!(uart_port(&ports, None), None, "two ports: can't tell");
        assert_eq!(uart_port(&ports, Some(2)), None);
        let one = vec![port("/dev/cu.usbmodem1", None)];
        assert_eq!(uart_port(&one, None).as_deref(), Some("/dev/cu.usbmodem1"));
    }

    #[test]
    fn unknown_sessions_are_request_errors() {
        let err = sessions().write("serial-999", b"x").unwrap_err();
        assert_eq!(err.category, crate::errors::Category::Request);
        assert!(sessions().close("serial-999").is_err());
    }

    /// A real session on a pseudo-terminal pair: what the other end writes
    /// is read, and what's written reaches the other end.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_session_reads_and_writes() {
        use serialport::SerialPort;
        let (mut board, mut computer) = serialport::TTYPort::pair().expect("a pty pair");
        let name = computer.name().expect("the pty's name");
        computer.set_timeout(READ_SLICE).unwrap();
        let s = sessions();
        let (id, _) = s
            .start(&name, Box::new(computer), 115_200, None, None)
            .unwrap();
        // The board talks: read it, waiting for the whole line.
        board.write_all(b"Hello from your FPGA board\r\n").unwrap();
        let r = s
            .read(&id, Duration::from_secs(5), Some(b"\r\n"))
            .await
            .unwrap();
        assert_eq!(r.bytes, b"Hello from your FPGA board\r\n");
        assert!(r.open);
        // The computer talks: the board reads it.
        assert_eq!(s.write(&id, b"A").unwrap(), 1);
        let mut got = [0u8; 1];
        board.set_timeout(Duration::from_secs(5)).unwrap();
        board.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"A");
        // Any byte goes out and comes in exactly, text or not.
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(s.write(&id, &all).unwrap(), 256);
        let mut echoed = vec![0u8; 256];
        board.read_exact(&mut echoed).unwrap();
        assert_eq!(echoed, all);
        board.write_all(&all).unwrap();
        let mut got = Vec::new();
        while got.len() < 256 {
            let r = s.read(&id, Duration::from_secs(5), None).await.unwrap();
            assert!(!r.bytes.is_empty(), "timed out after {} bytes", got.len());
            got.extend(r.bytes);
        }
        assert_eq!(got, all);
        // Nothing more: an empty read after the wait.
        let r = s.read(&id, Duration::from_millis(50), None).await.unwrap();
        assert!(r.bytes.is_empty());
        // Writing never waits for the reader: keys typed one by one go out
        // at once (each took a second or more when the two shared a lock).
        for _ in 0..20 {
            let started = std::time::Instant::now();
            s.write(&id, b"k").unwrap();
            assert!(
                started.elapsed() < Duration::from_millis(100),
                "{:?}",
                started.elapsed()
            );
        }
        let mut keys = [0u8; 20];
        board.read_exact(&mut keys).unwrap();
        assert_eq!(&keys, b"kkkkkkkkkkkkkkkkkkkk");
        // The same port can't be opened twice.
        assert!(s.open(&name, 115_200, None, None).is_err());
        s.close(&id).unwrap();
        assert!(s.read(&id, Duration::ZERO, None).await.is_err());
    }
}
