//! The published copy of the tool schemas, schemas/tools.json, must match the
//! code, so schema changes show up in code review (docs/tool-api.md,
//! "Published copy"). To update it after a deliberate change, run
//! scripts/update-schemas.sh.

const PUBLISHED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/tools.json");

#[test]
fn published_schemas_match_the_code() {
    let tools = logix_engine::tools::all_definitions();
    let current = serde_json::to_string_pretty(&tools).unwrap() + "\n";
    if std::env::var_os("UPDATE_SCHEMAS").is_some() {
        std::fs::write(PUBLISHED, &current).unwrap();
        return;
    }
    let published = std::fs::read_to_string(PUBLISHED).unwrap_or_default();
    assert!(
        published == current,
        "schemas/tools.json is out of date. Run scripts/update-schemas.sh and review the change. \
         Once released, changes may only add tools or optional fields."
    );
}
