//! Texts for people about a device's driver (Windows) or udev rule (Linux),
//! so every frontend shows the same ones (docs/tool-api.md, Drivers on
//! Windows, udev rules on Linux): its state, a short tag, the questions to
//! ask before adding or removing it, and what to do for a device that
//! can't be opened. Moved here from Logix IDE, which built them itself (the
//! user's decision, 2026-11-18: Engine first). They never name a frontend,
//! its menus or its buttons: a frontend adds those around them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::drivers::DriverStatus;

/// What `needed` means for Linux's udev rules.
pub const UDEV_RULE: &str = "udev-rule";

/// A question to ask the user before a change (adding or removing a
/// driver or udev rule), and the lines to show around it: a frontend shows
/// it in its own dialog, with its own Cancel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Consent {
    /// The question's title, such as "Install the board's USB driver?".
    pub title: String,
    /// One sentence: what it's for, naming the board.
    pub lead: String,
    /// What happens, a short sentence each, such as Windows' or Linux's own
    /// permission prompt.
    pub steps: Vec<String>,
    /// An optional last line, such as that it can be undone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The label of the button that agrees, such as "Install Driver".
    pub confirm: String,
    /// A status line while the change runs, such as "Installing the
    /// board's driver…".
    pub busy: String,
    /// The line to show when the user declines, such as "Cancelled: the
    /// driver wasn't installed.".
    pub cancelled: String,
}

/// A Windows driver's service name for people, such as "FTDI's driver"
/// for ftdibus; other names as they are.
pub fn driver_name(service: &str) -> String {
    match service {
        "winusb" => "WinUSB",
        "libusbk" => "libusbK",
        "libusb0" => "libusb0",
        "ftdibus" => "FTDI's driver",
        "usbser" => "the serial port driver",
        other => other,
    }
    .to_string()
}

/// Fills a driver status's texts for people, for the device named `who`
/// (such as "the Alhambra II", or the device's label): its label, state
/// and tag, the current driver's name, whether it can be added or removed,
/// and the questions for each.
pub fn fill(status: &mut DriverStatus, who: &str) {
    let udev = status.needed == UDEV_RULE;
    status.current_label = if udev {
        None
    } else {
        status.current.as_deref().map(driver_name)
    };
    status.can_add = !status.ready;
    // Linux: only Logix Engine's own rule can be removed. Windows: a ready
    // device's libusb driver (uninstall_driver says when it found none
    // that libwdi installed).
    status.can_remove = if udev {
        status.current.is_some()
    } else {
        status.ready
    };
    if udev {
        status.label = "a udev rule".into();
        status.state = match (status.ready, &status.current) {
            (false, _) => {
                "Access: Linux needs a udev rule before the board can be programmed.".into()
            }
            (true, Some(file)) => {
                format!(
                    "Access: ready to program the board, with Logix Engine's udev rule ({file})."
                )
            }
            (true, None) => "Access: ready to program the board.".into(),
        };
        status.tag = (!status.ready).then(|| "no udev rule yet".into());
    } else {
        status.label = "the WinUSB driver".into();
        status.state = if status.ready {
            format!(
                "Driver: {}, ready to program the board.",
                status
                    .current_label
                    .clone()
                    .unwrap_or_else(|| driver_name(&status.needed))
            )
        } else {
            let now = match &status.current_label {
                Some(name) => format!("it has {name} now"),
                None => "it has no driver now".into(),
            };
            format!(
                "Driver: needs WinUSB on USB interface {} to program the board ({now}).",
                status.interface.unwrap_or(0)
            )
        };
        status.tag = (!status.ready).then(|| "no driver yet".into());
    }
    status.add = status.can_add.then(|| add_question(status, who));
    status.remove = status.can_remove.then(|| remove_question(status, who));
}

/// The question before installing WinUSB (Windows) or adding a udev rule
/// (Linux) for the device `who`. Plain words (the user's request,
/// 2026-11-14): what it's for, what happens, and that it can be undone.
fn add_question(status: &DriverStatus, who: &str) -> Consent {
    if status.needed == UDEV_RULE {
        return Consent {
            title: "Let your user account program the board?".into(),
            lead: format!(
                "To program {who}, Linux needs a udev rule that lets your user account use it."
            ),
            steps: vec![
                "Linux asks for your password. Enter it to allow the change.".into(),
                "The rule is only for this kind of board (its USB IDs): other connected boards of the same kind can be programmed too.".into(),
                "Nothing else on your computer changes.".into(),
            ],
            note: Some("You can remove the rule later.".into()),
            confirm: "Add Rule".into(),
            busy: "Adding the board's udev rule…".into(),
            cancelled: "Cancelled: the udev rule wasn't added.".into(),
        };
    }
    let now = match &status.current_label {
        Some(name) => format!(" Right now Windows uses {name} for it."),
        None => String::new(),
    };
    Consent {
        title: "Install the board's USB driver?".into(),
        lead: format!("To program {who}, Windows needs to use the WinUSB driver for it.{now}"),
        steps: vec![
            "Windows asks whether Logix Engine may make changes to your device. Choose Yes.".into(),
            "It takes a minute or two.".into(),
            "Only the part of the board that programs the FPGA changes. Its other parts, such as a serial port, keep working as before.".into(),
            "Other connected boards of the same kind get the driver too.".into(),
        ],
        note: Some("You can remove the driver later.".into()),
        confirm: "Install Driver".into(),
        busy: "Installing the board's driver…".into(),
        cancelled: "Cancelled: the driver wasn't installed.".into(),
    }
}

/// The question before removing WinUSB (Windows) or Logix Engine's udev
/// rule (Linux) from the device `who`.
fn remove_question(status: &DriverStatus, who: &str) -> Consent {
    if status.needed == UDEV_RULE {
        return Consent {
            title: "Remove the board's udev rule?".into(),
            lead: format!("This removes Logix Engine's udev rule for {who}."),
            steps: vec![
                "Linux asks for your password. Enter it to allow the change.".into(),
                "Then your user account can't program it, unless another rule allows it.".into(),
                "To program the board again, add the rule again.".into(),
            ],
            note: None,
            confirm: "Remove Rule".into(),
            busy: "Removing the board's udev rule…".into(),
            cancelled: "Cancelled: the udev rule wasn't removed.".into(),
        };
    }
    Consent {
        title: "Remove the board's USB driver?".into(),
        lead: format!(
            "This restores the original Windows driver for {who}, if it had one, so the board maker's own tools can use it again."
        ),
        steps: vec![
            "Windows asks whether Logix Engine may make changes to your device. Choose Yes.".into(),
            "Other connected boards of the same kind get their original driver back too.".into(),
            "To program the board again, install WinUSB again.".into(),
        ],
        note: None,
        confirm: "Remove Driver".into(),
        busy: "Removing the board's driver…".into(),
        cancelled: "Cancelled: the driver wasn't removed.".into(),
    }
}

/// What to do for a device the engine can't open, for people (the
/// device's `action` says it for AI agents).
pub fn fix() -> String {
    if cfg!(windows) {
        "It needs its driver first, so it can be programmed.".into()
    } else if cfg!(target_os = "linux") {
        "Add a udev rule for it, so it can be programmed and its name read.".into()
    } else {
        "Allow your user account to use it, so it can be programmed.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn udev(ready: bool, rule: Option<&str>) -> DriverStatus {
        let mut s = crate::drivers::udev_status(ready, rule.map(String::from));
        fill(&mut s, "the Alhambra II");
        s
    }

    fn winusb(current: Option<&str>) -> DriverStatus {
        let mut s = crate::drivers::status(0, &[current.map(String::from)]);
        fill(&mut s, "the Alhambra II");
        s
    }

    #[test]
    fn linux_without_a_rule_offers_one() {
        let s = udev(false, None);
        assert_eq!(s.label, "a udev rule");
        assert_eq!(s.tag.as_deref(), Some("no udev rule yet"));
        assert!(s.can_add && !s.can_remove);
        let add = s.add.unwrap();
        assert_eq!(
            add.lead,
            "To program the Alhambra II, Linux needs a udev rule that lets your user account use it."
        );
        assert_eq!(add.confirm, "Add Rule");
        assert_eq!(add.cancelled, "Cancelled: the udev rule wasn't added.");
        assert!(s.remove.is_none());
    }

    #[test]
    fn linux_with_the_engines_rule_offers_to_remove_it() {
        let s = udev(true, Some("70-logix-engine-0403-6010.rules"));
        assert_eq!(
            s.state,
            "Access: ready to program the board, with Logix Engine's udev rule (70-logix-engine-0403-6010.rules)."
        );
        assert!(s.tag.is_none());
        assert!(!s.can_add && s.can_remove);
        assert_eq!(
            s.remove.unwrap().lead,
            "This removes Logix Engine's udev rule for the Alhambra II."
        );
        // Ready through another rule: nothing to add or remove.
        let other = udev(true, None);
        assert_eq!(other.state, "Access: ready to program the board.");
        assert!(!other.can_add && !other.can_remove);
    }

    #[test]
    fn windows_names_the_drivers_for_people() {
        let s = winusb(Some("ftdibus"));
        assert_eq!(s.current_label.as_deref(), Some("FTDI's driver"));
        assert_eq!(
            s.state,
            "Driver: needs WinUSB on USB interface 0 to program the board (it has FTDI's driver now)."
        );
        assert_eq!(s.tag.as_deref(), Some("no driver yet"));
        assert_eq!(
            s.add.unwrap().lead,
            "To program the Alhambra II, Windows needs to use the WinUSB driver for it. Right now Windows uses FTDI's driver for it."
        );
        let ready = winusb(Some("winusb"));
        assert_eq!(ready.state, "Driver: WinUSB, ready to program the board.");
        assert!(ready.can_remove && !ready.can_add);
        assert_eq!(ready.remove.unwrap().confirm, "Remove Driver");
        assert_eq!(
            winusb(None).state,
            "Driver: needs WinUSB on USB interface 0 to program the board (it has no driver now)."
        );
    }

    #[test]
    fn texts_never_name_a_frontend() {
        for s in [
            udev(false, None),
            udev(true, Some("x.rules")),
            winusb(Some("ftdibus")),
            winusb(Some("winusb")),
        ] {
            let text = serde_json::to_string(&s).unwrap();
            assert!(!text.contains("Logix IDE"), "{text}");
            assert!(!text.contains("Tools >"), "{text}");
            assert!(!text.contains("Upload"), "{text}");
        }
    }
}
