//! The serial tools (docs/tool-api.md, Serial ports): open_serial,
//! read_serial, write_serial and close_serial, over the sessions in
//! crate::serial.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::errors::ToolError;
use crate::serial::{DeviceKey, SerialEvent, sessions};

/// The longest read_serial waits.
const MAX_WAIT_MS: u32 = 60_000;

pub struct OpenSerial;

impl EngineTool for OpenSerial {
    const NAME: &'static str = "open_serial";
    const DESCRIPTION: &'static str = "Opens a serial port, such as a board's UART over its USB \
        cable, for a Serial Monitor or an AI agent checking what a design prints, and returns a \
        session: read it with read_serial, send with write_serial, end with close_serial. Take \
        the port from list_devices: a device's serial_port is its board's UART (else choose from \
        its serial_ports). Give device too (its id): then the session pauses while program_board \
        writes that board and reopens after, and reopens the port when the board is plugged in \
        again, even under another name. The format is 8 data bits, no parity, 1 stop bit; baud \
        defaults to 115200. The session stays open until close_serial or the engine exits.";
    const READ_ONLY: bool = false;
    type Input = OpenSerialInput;
    type Output = OpenSerialOutput;
}

/// The port to open.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenSerialInput {
    /// The port, such as COM5, /dev/cu.usbserial-A10 or /dev/ttyUSB1: a
    /// device's serial_port or one of its serial_ports from list_devices.
    pub port: String,
    /// Bits per second. Defaults to 115200.
    #[serde(default)]
    pub baud: Option<u32>,
    /// The device the port belongs to, by its id from list_devices, so the
    /// session pauses while program_board writes it and finds the port
    /// again after it's unplugged. Omitted means the port alone.
    #[serde(default)]
    pub device: Option<String>,
}

/// The new session.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenSerialOutput {
    /// The session's ID, for the other serial tools.
    pub session: String,
    /// The port opened.
    pub port: String,
    /// Its speed, bits per second.
    pub baud: u32,
}

pub fn open(input: OpenSerialInput) -> Result<OpenSerialOutput, ToolError> {
    let baud = input.baud.unwrap_or(115_200);
    if !(50..=4_000_000).contains(&baud) {
        return Err(ToolError::request(format!(
            "baud: {baud} isn't a serial speed: use 50 to 4000000, such as 115200."
        )));
    }
    let device = match &input.device {
        Some(id) => Some(device_key(id)?),
        None => None,
    };
    // The port's own key and USB interface, as the serial port list
    // reports them, to find it again later (its serial number may differ
    // from the device ID's: serial::ports_of_devices).
    let owner = device.and_then(|device| {
        crate::serial::usb_ports()
            .into_iter()
            .find(|(k, p)| k.vid == device.vid && k.pid == device.pid && p.port == input.port)
            .map(|(port_key, p)| crate::serial::PortOwner {
                device,
                port_key,
                interface: p.interface,
            })
    });
    let (session, port) = sessions().open(&input.port, baud, owner)?;
    Ok(OpenSerialOutput {
        session,
        port,
        baud,
    })
}

/// A device ID's USB IDs and serial number (the engine's own format,
/// usb:<location>:<vid>:<pid>:<serial>).
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

pub struct ReadSerial;

impl EngineTool for ReadSerial {
    const NAME: &'static str = "read_serial";
    const DESCRIPTION: &'static str = "Returns the bytes a serial session received since the \
        last read, exactly, as hex (two digits per byte), and what happened to its port meanwhile (paused for program_board, \
        resumed, disconnected, reconnected). With wait_ms it waits up to that long for \
        something to arrive (with until, for those bytes, such as 0d0a, a line ending); without, it returns at once. A Serial Monitor calls it in a loop with a wait, \
        such as 1000 ms. Up to 1 MB is kept unread; beyond, the oldest is dropped (an overflow \
        event).";
    const READ_ONLY: bool = true;
    type Input = ReadSerialInput;
    type Output = ReadSerialOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadSerialInput {
    /// The session, from open_serial.
    pub session: String,
    /// How long to wait for data, in milliseconds, at most 60000. Defaults
    /// to 0: return what there is now.
    #[serde(default)]
    pub wait_ms: Option<u32>,
    /// Bytes to wait for, as hex, such as "0d0a" (a line ending): wait
    /// until what's received contains them (or wait_ms ends), instead of
    /// returning at the first data.
    #[serde(default)]
    pub until: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadSerialOutput {
    /// What arrived, exactly: two lowercase hex digits per byte, such as
    /// "48690d0a" for "Hi" and a carriage return and line feed. Empty when
    /// nothing did. The engine passes bytes through unchanged, as a UART
    /// does: decoding them (such as UTF-8 text) is the client's.
    pub hex: String,
    /// What happened to the port, in order. Omitted when nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<SerialEvent>,
    /// Whether the port is open now (false while paused or unplugged).
    pub open: bool,
    /// The port's name now: it may change when the board comes back.
    pub port: String,
}

pub async fn read(input: ReadSerialInput) -> Result<ReadSerialOutput, ToolError> {
    let wait = input.wait_ms.unwrap_or(0);
    if wait > MAX_WAIT_MS {
        return Err(ToolError::request(format!(
            "wait_ms: {wait} is too long: at most {MAX_WAIT_MS}."
        )));
    }
    let until = match input
        .until
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        Some(u) => Some(hex_bytes("until", u)?),
        None => None,
    };
    let r = sessions()
        .read(
            &input.session,
            Duration::from_millis(wait.into()),
            until.as_deref(),
        )
        .await?;
    Ok(ReadSerialOutput {
        hex: hex::encode(&r.bytes),
        events: r.events,
        open: r.open,
        port: r.port,
    })
}

pub struct WriteSerial;

impl EngineTool for WriteSerial {
    const NAME: &'static str = "write_serial";
    const DESCRIPTION: &'static str = "Sends bytes to a serial session's port, exactly as \
        given, as hex (two digits per byte, such as 48690a for \"Hi\" and a line feed): any \
        byte, text or not. Encoding text (such as UTF-8, and the line ending the device \
        expects) is the client's. While the port is closed (paused or unplugged), nothing is \
        sent and it's an error.";
    const READ_ONLY: bool = false;
    type Input = WriteSerialInput;
    type Output = WriteSerialOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteSerialInput {
    /// The session, from open_serial.
    pub session: String,
    /// The bytes to send, two hex digits each, such as "48690a".
    pub hex: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteSerialOutput {
    /// How many bytes were sent.
    pub bytes: usize,
}

pub fn write(input: WriteSerialInput) -> Result<WriteSerialOutput, ToolError> {
    let data = hex_bytes("hex", input.hex.trim())?;
    Ok(WriteSerialOutput {
        bytes: sessions().write(&input.session, &data)?,
    })
}

/// Hex to bytes, or a request error naming the field.
fn hex_bytes(field: &str, text: &str) -> Result<Vec<u8>, ToolError> {
    hex::decode(text).map_err(|e| {
        ToolError::request(format!(
            "{field}: {e}: give two hex digits per byte, such as \"48690a\"."
        ))
    })
}

pub struct CloseSerial;

impl EngineTool for CloseSerial {
    const NAME: &'static str = "close_serial";
    const DESCRIPTION: &'static str = "Ends a serial session and closes its port, so other \
        programs can open it. What it received but wasn't read is dropped.";
    const READ_ONLY: bool = false;
    type Input = CloseSerialInput;
    type Output = CloseSerialOutput;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseSerialInput {
    /// The session, from open_serial.
    pub session: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseSerialOutput {}

pub fn close(input: CloseSerialInput) -> Result<CloseSerialOutput, ToolError> {
    sessions().close(&input.session)?;
    Ok(CloseSerialOutput {})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ids_give_their_usb_ids_and_serial_number() {
        let k = device_key("usb:1-2.3:0403:6010:A1").unwrap();
        assert_eq!(
            (k.vid, k.pid, k.serial_number.as_deref()),
            (0x0403, 0x6010, Some("A1"))
        );
        let k = device_key("usb:1-2:1a86:7523:").unwrap();
        assert_eq!(k.serial_number, None);
        assert!(device_key("COM5").is_err());
    }

    #[test]
    fn write_takes_hex() {
        let w = |hex: &str| {
            write(WriteSerialInput {
                session: "serial-999".into(),
                hex: hex.into(),
            })
            .unwrap_err()
        };
        assert!(w("6").message.starts_with("hex:"));
        assert!(w("zz").message.starts_with("hex:"));
        // Good input reaches the sessions: no such session.
        assert!(w("ff00").message.contains("no open serial session"));
    }

    #[test]
    fn bad_inputs_are_request_errors() {
        let e = open(OpenSerialInput {
            port: "COM5".into(),
            baud: Some(7),
            device: None,
        })
        .unwrap_err();
        assert!(e.message.contains("baud"), "{}", e.message);
    }
}
