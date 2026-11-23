//! Property-based fuzz test for the Android `ps -A` output parser (guest-controlled text).

use bifrost_process::parse_android_ps_output;
use proptest::prelude::*;

proptest! {
    #[test]
    fn ps_parser_never_panics(lines in proptest::collection::vec("[ -~]{0,80}", 0..40)) {
        let mut text = String::from("PID USER RSS NAME\n");
        text.push_str(&lines.join("\n"));
        let _ = parse_android_ps_output(&text);
    }

    #[test]
    fn ps_parser_handles_arbitrary_text(text in r"\PC{0,512}") {
        let _ = parse_android_ps_output(&text);
    }
}
