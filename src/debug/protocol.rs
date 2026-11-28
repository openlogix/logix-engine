//! The debugger's protocol, version 1 (docs/debugger.md, Protocol), and the
//! debug map the build writes for it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;
pub const SYNC_HOST: u8 = 0xA5;
pub const SYNC_REPLY: u8 = 0x5A;

/// The protocol version and the build ID: 5 bytes.
pub const INFO: u8 = 0x01;
/// A snapshot taken in one clock: the read nets, the write values and the
/// enables, packed.
pub const READ: u8 = 0x02;
/// The write values and the enables, packed, all taking effect at once.
pub const WRITE: u8 = 0x03;
/// Flips one-bit nets for one clock: a mask of the enables' width.
pub const PULSE: u8 = 0x04;

/// The most bytes a frame's payload, or a reply's, can have.
pub const MAX_PAYLOAD: usize = 255;
/// The bytes of an INFO reply.
pub const INFO_BYTES: usize = 5;

/// CRC-8, polynomial 07, initial 00 (CRC-8/SMBUS), as the debugger has it.
pub fn crc8(data: &[u8]) -> u8 {
    let mut c = 0u8;
    for &d in data {
        c ^= d;
        for _ in 0..8 {
            c = if c & 0x80 != 0 {
                (c << 1) ^ 0x07
            } else {
                c << 1
            };
        }
    }
    c
}

/// A host frame: sync, command, length, payload, CRC.
pub fn frame(command: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= 255);
    let mut f = vec![SYNC_HOST, command, payload.len() as u8];
    f.extend_from_slice(payload);
    let crc = crc8(&f[1..]);
    f.push(crc);
    f
}

/// What a reply's status byte means.
pub fn status_text(status: u8) -> &'static str {
    match status {
        0 => "OK",
        1 => "the debugger got a bad CRC",
        2 => "the debugger doesn't know the command",
        3 => "the debugger refused the length or range",
        _ => "an unknown status",
    }
}

/// A reply's fields, once its CRC checked out.
#[derive(Debug, PartialEq)]
pub struct Reply {
    pub command: u8,
    pub status: u8,
    pub payload: Vec<u8>,
}

/// Finds a whole reply in `buf` (bytes before its sync byte are skipped).
/// Ok(None) means more bytes are needed.
pub fn parse_reply(buf: &[u8]) -> Result<Option<(Reply, usize)>, String> {
    let Some(start) = buf.iter().position(|&b| b == SYNC_REPLY) else {
        return Ok(None);
    };
    let b = &buf[start..];
    if b.len() < 5 {
        return Ok(None);
    }
    let len = b[3] as usize;
    let total = 5 + len;
    if b.len() < total {
        return Ok(None);
    }
    if crc8(&b[1..total - 1]) != b[total - 1] {
        return Err("The reply's CRC is wrong: the link dropped or changed bytes.".into());
    }
    Ok(Some((
        Reply {
            command: b[1],
            status: b[2],
            payload: b[4..4 + len].to_vec(),
        },
        start + total,
    )))
}

/// The debug map, debug_map.json in the build folder: what the debugger in
/// that build reads and drives, and where (docs/debugger.md, R7).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugMap {
    /// The map's format, logix-debug-map-v1.
    pub format: String,
    /// The protocol version the debugger speaks.
    pub protocol: u8,
    /// The build ID, 8 hex digits, which the debugger reports too.
    pub build_id: String,
    /// The board's ID.
    pub board: String,
    /// The design's top module (the debug build's top wraps it).
    pub top_module: String,
    /// The debugger's clock port and its frequency in MHz.
    pub clock: String,
    pub clock_mhz: f64,
    /// The board resource the UART is, when the debug file named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uplink_resource: Option<String>,
    /// The UART's speed and FPGA pins.
    pub baud: u32,
    pub tx_pin: String,
    pub rx_pin: String,
    /// Bits of the read nets, of the write values and of the override
    /// enables (one per driven entry). Each is at least 1.
    pub read_bits: usize,
    pub write_bits: usize,
    pub enables: usize,
    /// The nets the debugger drives, in write-space order.
    pub drive: Vec<MapDrive>,
    /// The nets the debugger reads, in read-space order.
    pub watch: Vec<MapWatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapWatch {
    /// The host's name for it.
    pub name: String,
    /// The net in the design; for a bus combined from a list, the list,
    /// as {main.a, pin.18}.
    pub net: String,
    /// A combined bus's members, most significant first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// Its width in bits.
    pub width: usize,
    /// Its first bit in the read space.
    pub offset: usize,
    /// The debug file's format for showing it (hex, dec or bin), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapDrive {
    pub name: String,
    /// The net, or a combined bus's list, as for watched nets.
    pub net: String,
    /// A combined bus's members, most significant first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    pub width: usize,
    /// Its first bit in the write space.
    pub offset: usize,
    /// The debug file's format for showing it (hex, dec or bin), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Its bit in the override enables.
    pub enable_bit: usize,
}

impl DebugMap {
    /// Bytes of a READ reply: the read nets, the write values and the
    /// enables, packed.
    pub fn read_reply_bytes(&self) -> usize {
        (self.read_bits + self.write_bits + self.enables).div_ceil(8)
    }
}

pub const MAP_FORMAT: &str = "logix-debug-map-v1";
pub const MAP_FILE: &str = "debug_map.json";

/// Bits `offset..offset+width` of a little-endian byte string.
pub fn get_bits(bytes: &[u8], offset: usize, width: usize) -> Vec<bool> {
    (offset..offset + width)
        .map(|i| bytes.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 == 1))
        .collect()
}

/// Bits packed low first into bytes, padded with zeros.
pub fn pack(bits: &[bool]) -> Vec<u8> {
    let mut bytes = vec![0u8; bits.len().div_ceil(8)];
    set_bits(&mut bytes, 0, bits);
    bytes
}

/// Sets bits `offset..` of a little-endian byte string to `bits`.
pub fn set_bits(bytes: &mut [u8], offset: usize, bits: &[bool]) {
    for (k, &v) in bits.iter().enumerate() {
        let i = offset + k;
        if v {
            bytes[i / 8] |= 1 << (i % 8);
        } else {
            bytes[i / 8] &= !(1 << (i % 8));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_is_crc8_smbus() {
        // The standard check value of CRC-8/SMBUS.
        assert_eq!(crc8(b"123456789"), 0xF4);
    }

    #[test]
    fn replies_round_trip() {
        let mut r = vec![0x00, SYNC_REPLY, READ, 0, 2, 0xDE, 0xBC];
        r.push(crc8(&r[2..]));
        let (reply, used) = parse_reply(&r).unwrap().unwrap();
        assert_eq!(used, r.len());
        assert_eq!(reply.payload, vec![0xDE, 0xBC]);
        assert!(parse_reply(&r[..r.len() - 1]).unwrap().is_none());
        let last = r.len() - 1;
        r[last] ^= 1;
        assert!(parse_reply(&r).is_err());
    }

    #[test]
    fn bits_pack_little_endian() {
        let mut b = vec![0u8; 3];
        set_bits(
            &mut b,
            4,
            &[true, false, true, true, true, true, true, true, true],
        );
        assert_eq!(b, vec![0xD0, 0x1F, 0x00]);
        assert_eq!(get_bits(&b, 4, 3), vec![true, false, true]);
        assert_eq!(
            pack(&[true, false, true, true, true, true, true, true, true]),
            vec![0xFD, 0x01]
        );
    }
}
