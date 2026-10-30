//! The "board specification" input type (docs/tool-api.md): a catalog ID, or
//! a full board definition whose FPGA is a catalog ID or a full definition.
//! It is resolved into a complete board, which the build record keeps.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::{self, BoardDefinition, FpgaDefinition, ProgrammingVariant};
use crate::errors::ToolError;

/// A board: a catalog ID, or a full definition.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum BoardSpec {
    /// A board's catalog ID, such as alhambra-ii. get_catalog lists them.
    Id(#[schemars(pattern(r"^[a-z0-9-]+$"))] String),
    /// A full board definition, for boards that aren't in the catalog.
    Definition(Box<BoardInputDefinition>),
}

/// A board definition in a request. Like a catalog entry, except that the
/// FPGA can be given either as fpga_id or as a full fpga definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoardInputDefinition {
    /// The board's name for people.
    pub description: String,
    /// The board's FPGA as a catalog ID, such as ice40hx4k-tq144-8k. Give
    /// either fpga_id or fpga.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga_id: Option<String>,
    /// The board's FPGA as a full definition. Give either fpga_id or fpga.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga: Option<FpgaDefinition>,
    /// The name of the programming variant used by default.
    pub default_variant: String,
    /// The ways to program the board, by variant name.
    pub variants: BTreeMap<String, ProgrammingVariant>,
}

// A clearer error than serde's "did not match any variant".
impl<'de> Deserialize<'de> for BoardSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::String(s) => Ok(BoardSpec::Id(s)),
            serde_json::Value::Object(_) => serde_json::from_value(v)
                .map(|b| BoardSpec::Definition(Box::new(b)))
                .map_err(|e| serde::de::Error::custom(format!("board definition: {e}"))),
            _ => Err(serde::de::Error::custom(
                "board must be a catalog ID string or a board definition object",
            )),
        }
    }
}

/// A board with everything spelled out: what a build used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedBoard {
    /// The board's catalog ID, when it came from the catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_id: Option<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga_id: Option<String>,
    pub fpga: FpgaDefinition,
    pub default_variant: String,
    pub variants: BTreeMap<String, ProgrammingVariant>,
}

impl ResolvedBoard {
    /// SHA-256 of the board as JSON, which is stable: struct fields and map
    /// keys have a fixed order.
    pub fn checksum(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        hex::encode(Sha256::digest(json.as_bytes()))
    }
}

fn fpga_by_id(id: &str) -> Result<FpgaDefinition, ToolError> {
    catalog::builtin().fpgas.get(id).cloned().ok_or_else(|| {
        ToolError::request(format!(
            "{id:?} isn't an FPGA in the catalog. Use get_catalog to list them."
        ))
    })
}

/// Resolves a board input against the catalog. Unknown IDs and bad
/// definitions are request errors.
pub fn resolve(spec: &BoardSpec) -> Result<ResolvedBoard, ToolError> {
    match spec {
        BoardSpec::Id(id) => {
            let b = catalog::builtin().boards.get(id).ok_or_else(|| {
                ToolError::request(format!(
                    "{id:?} isn't a board in the catalog. Use get_catalog to list the boards, or give a full board definition."
                ))
            })?;
            Ok(ResolvedBoard {
                board_id: Some(id.clone()),
                description: b.description.clone(),
                fpga_id: Some(b.fpga_id.clone()),
                fpga: fpga_by_id(&b.fpga_id)?,
                default_variant: b.default_variant.clone(),
                variants: b.variants.clone(),
            })
        }
        BoardSpec::Definition(d) => {
            let (fpga_id, fpga) = match (&d.fpga_id, &d.fpga) {
                (Some(id), None) => (Some(id.clone()), fpga_by_id(id)?),
                (None, Some(f)) => (None, f.clone()),
                _ => {
                    return Err(ToolError::request(
                        "A board definition needs exactly one of fpga_id and fpga.",
                    ));
                }
            };
            // Reuse the catalog's checks for variants.
            let as_catalog = BoardDefinition {
                description: d.description.clone(),
                fpga_id: fpga_id.clone().unwrap_or_else(|| "custom".into()),
                default_variant: d.default_variant.clone(),
                variants: d.variants.clone(),
            };
            as_catalog
                .validate()
                .map_err(|e| ToolError::request(format!("Invalid board definition: {e}.")))?;
            Ok(ResolvedBoard {
                board_id: None,
                description: d.description.clone(),
                fpga_id,
                fpga,
                default_variant: d.default_variant.clone(),
                variants: d.variants.clone(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: serde_json::Value) -> Result<BoardSpec, String> {
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    #[test]
    fn catalog_id() {
        let b = resolve(&parse(json!("alhambra-ii")).unwrap()).unwrap();
        assert_eq!(b.board_id.as_deref(), Some("alhambra-ii"));
        assert_eq!(b.fpga.nextpnr_device, "hx8k");
        let err = resolve(&parse(json!("nope")).unwrap()).unwrap_err();
        assert!(err.message.contains("get_catalog"), "{}", err.message);
    }

    fn custom(fpga: serde_json::Value) -> serde_json::Value {
        let mut v = json!({
            "description": "My board",
            "default_variant": "flash",
            "variants": {"flash": {
                "programmer": "openfpgaloader",
                "args": "-b ice40_generic --vid ${VID} --pid ${PID} ${BITSTREAM}",
                "usb": {"vid": "0403", "pid": "6010"}
            }}
        });
        for (k, val) in fpga.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    }

    #[test]
    fn full_definitions() {
        let by_id = resolve(&parse(custom(json!({"fpga_id": "ice40up5k-sg48"}))).unwrap()).unwrap();
        assert_eq!(by_id.fpga.nextpnr_device, "up5k");
        let inline = resolve(
            &parse(custom(json!({"fpga": {
                "architecture": "ice40", "part_number": "X", "size": "1k",
                "nextpnr_device": "hx1k", "nextpnr_package": "tq144"
            }})))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(inline.fpga.nextpnr_package, "tq144");
        assert_ne!(by_id.checksum(), inline.checksum());
        // Neither or both.
        assert!(resolve(&parse(custom(json!({}))).unwrap()).is_err());
        // A bad variant.
        let mut bad = custom(json!({"fpga_id": "ice40up5k-sg48"}));
        bad["variants"]["flash"]["programmer"] = json!("tinyprog");
        assert!(resolve(&parse(bad).unwrap()).is_err());
    }

    #[test]
    fn clear_parse_errors() {
        assert!(parse(json!(5)).unwrap_err().contains("catalog ID string"));
        let err = parse(json!({"description": "x"})).unwrap_err();
        assert!(err.contains("board definition"), "{err}");
    }
}
