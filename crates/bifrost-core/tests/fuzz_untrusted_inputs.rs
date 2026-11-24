//! Property-based fuzz tests for `bifrost.toml` parsing and validation, which reads a
//! user-editable file that other tools (and the UI) may also write.

use bifrost_core::BifrostConfig;
use proptest::prelude::*;

fn tomlish() -> impl Strategy<Value = String> {
    let fragment = prop_oneof![
        Just("schema_version = 1\n".to_string()),
        Just("[runtime]\n".to_string()),
        Just("[network]\n".to_string()),
        Just("[images]\n".to_string()),
        Just("cpu_cores = ".to_string()),
        Just("memory_mb = ".to_string()),
        Just("adb_bind_address = \"".to_string()),
        Just("allow_non_loopback_adb = true\n".to_string()),
        Just("trusted_keys = [\"".to_string()),
        Just("data_dir = \"..\\..\\Windows\"\n".to_string()),
        Just("\"]\n".to_string()),
        Just("\"\n".to_string()),
        "[0-9]{1,12}\n",
        "[ -~]{0,16}",
    ];
    proptest::collection::vec(fragment, 0..32).prop_map(|p| p.concat())
}

proptest! {
    #[test]
    fn config_parser_never_panics(input in tomlish()) {
        if let Ok(cfg) = BifrostConfig::from_toml_str(&input) {
            let _ = cfg.validate();
        }
    }

    #[test]
    fn validated_configs_keep_adb_on_loopback(input in tomlish()) {
        if let Ok(cfg) = BifrostConfig::from_toml_str(&input) {
            if cfg.validate().is_ok() && !cfg.network.allow_non_loopback_adb {
                let ip: std::net::IpAddr = cfg.network.adb_bind_address.parse().unwrap();
                prop_assert!(ip.is_loopback());
            }
        }
    }
}
