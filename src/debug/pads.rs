//! Nets the debugger can't reach: those wired to a cell port that may
//! connect only to the FPGA's own pins, such as an I/O buffer's pad or the
//! outputs of the iCE40 UltraPlus RGB LED driver (`SB_RGBA_DRV`). Watching
//! one gives it a second load inside the FPGA, which the place and route
//! refuses ("SB_RGB_DRV/SB_RGBA_DRV port connected to more than just
//! package pin"), and driving one would cut it from its pin. So `list_nets`
//! marks them, and the debug build refuses them with this reason before
//! synthesis.
//!
//! The list holds the pad ports of the cells in the cell libraries the
//! engine reads (yosys's `cells_sim.v` for each toolchain) that designs
//! instantiate by hand; yosys adds its own I/O buffers only after the
//! debugger is in, so a plain top-level port is never one of them.

use serde_json::Value;

/// Each cell type's ports that may connect only to the FPGA's pins.
const PAD_PORTS: &[(&str, &[&str])] = &[
    // iCE40.
    ("SB_IO", &["PACKAGE_PIN"]),
    ("SB_GB_IO", &["PACKAGE_PIN"]),
    ("SB_IO_OD", &["PACKAGEPIN"]),
    ("SB_IO_I3C", &["PACKAGE_PIN"]),
    ("SB_RGBA_DRV", &["RGB0", "RGB1", "RGB2"]),
    ("SB_RGB_DRV", &["RGB0", "RGB1", "RGB2"]),
    // A hard wire to SB_RGB_DRV's RGBPU, not to a pin, but as fixed.
    ("SB_LED_DRV_CUR", &["LEDPU"]),
    // ECP5.
    ("TRELLIS_IO", &["B"]),
    // Gowin and Xilinx.
    ("IBUF", &["I"]),
    ("IBUFG", &["I"]),
    ("OBUF", &["O"]),
    ("OBUFT", &["O"]),
    ("TBUF", &["O"]),
    ("IOBUF", &["IO"]),
    ("ELVDS_OBUF", &["O", "OB"]),
    ("TLVDS_OBUF", &["O", "OB"]),
];

/// Why the debugger can't watch or drive a net with these bits (a netname's
/// `bits` in yosys's JSON of the flattened top module), or None when it can.
/// `top` names the module in the reason's net names.
pub(crate) fn unreachable(module: &Value, bits: &Value, top: &str) -> Option<String> {
    let wanted: Vec<&Value> = bits.as_array()?.iter().filter(|b| b.is_number()).collect();
    for cell in module["cells"].as_object()?.values() {
        let Some(kind) = cell["type"].as_str() else {
            continue;
        };
        let Some((_, ports)) = PAD_PORTS.iter().find(|(t, _)| *t == kind) else {
            continue;
        };
        for port in *ports {
            let hit = cell["connections"][*port]
                .as_array()
                .is_some_and(|list| list.iter().any(|b| wanted.contains(&b)));
            if !hit {
                continue;
            }
            // The RGB LED drivers: their inputs are what the design sets.
            if kind.ends_with("RGBA_DRV") || kind.ends_with("RGB_DRV") {
                let input = format!("{port}PWM");
                let by = net_named(module, &cell["connections"][&input])
                    .map(|n| format!(" ({top}.{n})"))
                    .unwrap_or_default();
                return Some(format!(
                    "It is wired to {kind}'s {port}, the RGB LED driver's output, which only its pin may take: watch or drive the driver's input, {input}{by}, instead."
                ));
            }
            return Some(format!(
                "It is wired to {kind}'s {port}, which only the FPGA's pin may take: watch or drive the net on the design's side of the {kind} instead."
            ));
        }
    }
    None
}

/// A named net with exactly these bits, if there is one.
fn net_named<'a>(module: &'a Value, bits: &Value) -> Option<&'a str> {
    if !bits
        .as_array()
        .is_some_and(|b| b.iter().all(Value::is_number))
    {
        return None;
    }
    module["netnames"]
        .as_object()?
        .iter()
        .filter(|(name, n)| n["hide_name"].as_u64() != Some(1) && !name.starts_with('$'))
        .find(|(_, n)| &n["bits"] == bits)
        .map(|(name, _)| name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rgb() -> Value {
        json!({
            "cells": {"rgb": {"type": "SB_RGBA_DRV", "connections": {
                "RGB0PWM": [5], "RGB1PWM": [6], "RGB2PWM": [7],
                "RGB0": [2], "RGB1": [3], "RGB2": [4]
            }}},
            "netnames": {
                "rgb_led": {"hide_name": 0, "bits": [2, 3, 4]},
                "green_on": {"hide_name": 0, "bits": [5]},
                "count": {"hide_name": 0, "bits": [5, 6, 7]}
            }
        })
    }

    #[test]
    fn the_rgb_drivers_outputs_are_unreachable() {
        let m = rgb();
        let why = unreachable(&m, &json!([2, 3, 4]), "main").unwrap();
        assert!(why.contains("SB_RGBA_DRV's RGB0"), "{why}");
        assert!(why.contains("RGB0PWM (main.green_on)"), "{why}");
        // One bit of a bus is enough.
        assert!(unreachable(&m, &json!([4]), "main").is_some());
    }

    #[test]
    fn the_drivers_inputs_and_other_nets_are_reachable() {
        let m = rgb();
        assert_eq!(unreachable(&m, &json!([5]), "main"), None);
        assert_eq!(unreachable(&m, &json!([5, 6, 7]), "main"), None);
        assert_eq!(unreachable(&m, &json!(["0"]), "main"), None);
    }

    #[test]
    fn io_buffer_pads_are_unreachable() {
        let m = json!({
            "cells": {"io": {"type": "SB_IO", "connections": {
                "PACKAGE_PIN": [9], "D_IN_0": [10]
            }}},
            "netnames": {}
        });
        let why = unreachable(&m, &json!([9]), "main").unwrap();
        assert!(why.contains("SB_IO's PACKAGE_PIN"), "{why}");
        assert_eq!(unreachable(&m, &json!([10]), "main"), None);
    }
}
