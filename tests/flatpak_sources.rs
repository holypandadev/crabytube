//! The Flatpak build compiles offline from the crates listed in its cargo sources, so every registry
//! package in Cargo.lock must have an archive there. A dependency added without regenerating the list
//! breaks that build, and CI does not build the Flatpak, so this test is what notices.

use std::collections::HashSet;
use std::path::Path;

/// The `name version` of every registry package in `lock` that has no archive in `sources`.
fn missing_registry_packages(lock: &str, sources: &str) -> Vec<String> {
    let entries: Vec<serde_json::Value> =
        serde_json::from_str(sources).expect("the cargo sources are a JSON array");
    let archives: HashSet<&str> = entries
        .iter()
        .filter_map(|entry| entry.get("url")?.as_str())
        .collect();
    lock.split("[[package]]")
        .skip(1)
        .filter_map(|block| {
            let field = |key: &str| {
                block.lines().find_map(|line| {
                    line.strip_prefix(key)?
                        .strip_prefix(" = \"")?
                        .strip_suffix('"')
                })
            };
            if !field("source")?.starts_with("registry+") {
                return None;
            }
            let (name, version) = (field("name")?, field("version")?);
            let url = format!("https://static.crates.io/crates/{name}/{name}-{version}.crate");
            (!archives.contains(url.as_str())).then(|| format!("{name} {version}"))
        })
        .collect()
}

#[test]
fn every_registry_package_is_in_the_flatpak_sources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock is readable");
    let sources = std::fs::read_to_string(root.join("packaging/linux/flatpak/cargo-sources.json"))
        .expect("the Flatpak cargo sources are readable");
    let missing = missing_registry_packages(&lock, &sources);
    assert!(
        missing.is_empty(),
        "packages missing from packaging/linux/flatpak/cargo-sources.json: {missing:?}; \
         regenerate the file from Cargo.lock (see the manifest comment)"
    );
}

#[test]
fn a_registry_package_without_an_archive_is_reported_and_others_are_not() {
    let lock = r#"
[[package]]
name = "listed"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "00"

[[package]]
name = "unlisted"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "11"

[[package]]
name = "local-crate"
version = "0.1.0"
"#;
    let sources = r#"[
        {"type": "archive", "url": "https://static.crates.io/crates/listed/listed-1.0.0.crate"},
        {"type": "inline", "dest-filename": ".cargo-checksum.json"}
    ]"#;
    assert_eq!(
        missing_registry_packages(lock, sources),
        vec!["unlisted 2.0.0".to_owned()]
    );
}
