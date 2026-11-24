//! Property-based fuzz tests for Valhalla parsers that consume untrusted data:
//! APK/APKM archives, binary AXML manifests, PNG icons, Android `pm`/`dumpsys`
//! output, and app labels that become Windows file names.
//!
//! Default runs use proptest's 256 cases per property; deep runs:
//! `PROPTEST_CASES=20000 cargo test -p valhalla --test fuzz_untrusted_inputs`.

use std::io::Write;

use proptest::prelude::*;
use valhalla::apk::{
    extract_manifest_metadata, extract_manifest_package, inspect_apk, wrap_png_as_ico,
};
use valhalla::package::{parse_dumpsys_package, parse_pm_list_packages, parse_resolved_activity};
use valhalla::sanitize_shortcut_name;

/// AXML chunk header prefix (RES_XML_TYPE, header size 8) so random tails reach the
/// string-pool and element parsers instead of failing on the first check.
const AXML_PREFIX: [u8; 4] = [0x03, 0x00, 0x08, 0x00];

fn axml_like() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..512),
        proptest::collection::vec(any::<u8>(), 0..2048).prop_map(|tail| {
            let mut v = AXML_PREFIX.to_vec();
            v.extend_from_slice(&(tail.len() as u32 + 8).to_le_bytes());
            v.extend(tail);
            v
        }),
    ]
}

/// Line-oriented text resembling Android shell output, with hostile fragments.
fn shellish_text() -> impl Strategy<Value = String> {
    let fragment = prop_oneof![
        Just("package:".to_string()),
        Just("=".to_string()),
        Just("versionName=".to_string()),
        Just("versionCode=".to_string()),
        Just("/data/app/".to_string()),
        Just("base.apk".to_string()),
        Just("Packages:".to_string()),
        Just("  Package [".to_string()),
        Just("]".to_string()),
        Just("\n".to_string()),
        Just("../".to_string()),
        "[ -~]{0,24}",
        "\\PC{0,8}",
    ];
    proptest::collection::vec(fragment, 0..48).prop_map(|parts| parts.concat())
}

proptest! {
    #[test]
    fn axml_parsers_never_panic(bytes in axml_like()) {
        let _ = extract_manifest_package(&bytes);
        let _ = extract_manifest_metadata(&bytes);
    }

    #[test]
    fn png_to_ico_never_panics_and_wraps_input(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
        if let Some(ico) = wrap_png_as_ico(&bytes) {
            // An ICO wrapper always embeds the original PNG bytes unchanged.
            prop_assert!(ico.ends_with(&bytes));
        }
    }

    #[test]
    fn pm_and_dumpsys_parsers_never_panic(text in shellish_text()) {
        let _ = parse_pm_list_packages(&text);
        let _ = parse_dumpsys_package("com.example.app", &text);
        let _ = parse_resolved_activity(&text);
    }

    #[test]
    fn sanitized_shortcut_names_are_creatable_windows_files(raw in "\\PC{0,96}") {
        if let Some(name) = sanitize_shortcut_name(&raw) {
            prop_assert!(!name.is_empty());
            prop_assert!(name.len() <= 72, "name too long: {} bytes", name.len());
            prop_assert!(!name.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*']));
            prop_assert!(!name.ends_with('.') && !name.ends_with(' '));
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(format!("{name}.lnk"));
            let created = std::fs::File::create(&path);
            prop_assert!(created.is_ok(), "cannot create '{}': {:?}", name, created.err());
            // The file must land inside the directory (no device or traversal names).
            prop_assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn random_bytes_as_apk_are_rejected_without_panicking(
        bytes in proptest::collection::vec(any::<u8>(), 1..1024),
        apkm in any::<bool>(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(if apkm { "fuzz.apkm" } else { "fuzz.apk" });
        std::fs::write(&path, &bytes).unwrap();
        let _ = inspect_apk(&path);
    }

    #[test]
    fn zip_slip_entries_are_always_rejected(
        names in proptest::collection::vec(entry_name(), 1..6),
        body in proptest::collection::vec(any::<u8>(), 0..64),
        apkm in any::<bool>(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(if apkm { "fuzz.apkm" } else { "fuzz.apk" });
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        let mut written = Vec::new();
        for name in &names {
            if written.contains(name) || zip.start_file(name.as_str(), opts).is_err() {
                continue;
            }
            zip.write_all(&body).unwrap();
            written.push(name.clone());
        }
        zip.finish().unwrap();

        let result = inspect_apk(&path);
        let has_traversal = written
            .iter()
            .any(|n| n.contains("..") || n.starts_with('/') || n.starts_with('\\'));
        if has_traversal {
            prop_assert!(result.is_err(), "traversal entries accepted: {:?}", written);
        }
    }
}

/// Windows reserved device names must never become shortcut file names: `NUL.lnk` or
/// `COM1.lnk` resolve to devices instead of files on many Windows versions.
#[test]
fn reserved_windows_device_names_are_not_used_as_shortcut_names() {
    for raw in [
        "CON", "con", "PRN", "AUX", "NUL", "nul ", "COM1", "com9", "LPT1", "LPT9", "NUL.",
        "CON.txt", "Con . ", "COM¹", "LPT²",
    ] {
        if let Some(name) = sanitize_shortcut_name(raw) {
            let stem = name
                .split('.')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_uppercase();
            let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.chars().count() == 4
                    && stem
                        .chars()
                        .nth(3)
                        .is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c)));
            assert!(
                !reserved,
                "'{raw}' sanitized to reserved device name '{name}'"
            );
        }
    }
}

fn entry_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("AndroidManifest.xml".to_string()),
        Just("classes.dex".to_string()),
        Just("lib/x86_64/libfoo.so".to_string()),
        Just("lib/arm64-v8a/libfoo.so".to_string()),
        Just("../../evil.dll".to_string()),
        Just("/abs/evil".to_string()),
        Just("\\windows\\evil".to_string()),
        Just("res/../../escape".to_string()),
        "[a-z/._-]{1,24}",
    ]
}
