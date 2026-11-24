//! Property-based fuzz test for the `adb forward --list` parser.

use bifrost_network::parse_adb_forward_list;
use proptest::prelude::*;

proptest! {
    #[test]
    fn forward_list_parser_never_panics(text in r"\PC{0,512}") {
        let _ = parse_adb_forward_list(&text);
    }

    #[test]
    fn forward_list_parser_handles_structured_noise(
        lines in proptest::collection::vec("[a-z0-9:. ]{0,48}", 0..20)
    ) {
        let _ = parse_adb_forward_list(&lines.join("\n"));
    }
}
