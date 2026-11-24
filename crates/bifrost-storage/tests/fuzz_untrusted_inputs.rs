//! Property-based fuzz tests for Midgard `manifest.json` parsing and validation, which
//! comes from downloaded (untrusted until verified) image bundles.

use bifrost_storage::{evaluate_payload_trust, MidgardImageManifest};
use proptest::prelude::*;

fn jsonish() -> impl Strategy<Value = String> {
    let fragment = prop_oneof![
        Just("{".to_string()),
        Just("}".to_string()),
        Just("\"schema_version\":1,".to_string()),
        Just("\"arch\":\"x86_64\",".to_string()),
        Just("\"boot\":{\"mode\":\"direct_kernel\",".to_string()),
        Just("\"kernel\":{\"path\":\"../../evil\",\"sha256\":\"".to_string()),
        Just("\"path\":\"C:\\Windows\\x\",".to_string()),
        Just("\"sha256\":\"".to_string()),
        Just("\"disks\":[".to_string()),
        Just("]".to_string()),
        Just("\",".to_string()),
        "[0-9a-f]{0,64}",
        "[ -~]{0,12}",
    ];
    proptest::collection::vec(fragment, 0..40).prop_map(|p| p.concat())
}

proptest! {
    #[test]
    fn manifest_parser_never_panics(input in jsonish()) {
        if let Ok(manifest) = MidgardImageManifest::from_json_str(&input) {
            let _ = manifest.validate();
        }
    }

    #[test]
    fn manifest_parser_handles_arbitrary_text(input in r"\PC{0,256}") {
        let _ = MidgardImageManifest::from_json_str(&input);
    }

    #[test]
    /// A present but invalid signature is rejected even with `allow_unsigned` (an empty
    /// signature means "unsigned" and is covered by the unit tests).
    fn trust_policy_never_accepts_garbage_signatures(
        payload in proptest::collection::vec(any::<u8>(), 0..128),
        sig in "[0-9a-f]{1,140}",
    ) {
        let key = "ab".repeat(32);
        let result = evaluate_payload_trust(&payload, Some(&sig), &[key], true);
        prop_assert!(result.is_err());
    }
}
