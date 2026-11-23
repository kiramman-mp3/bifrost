//! Property-based fuzz tests for Bridge parsers fed by the Android guest or by local
//! IPC clients: IPC frames, `adb devices`, `dumpsys notification`, Toast XML rendering
//! and Android shell quoting.
//!
//! Deep run: `PROPTEST_CASES=20000 cargo test -p bifrost-bridge --test fuzz_untrusted_inputs`.

use bifrost_bridge::{
    adb::parse_adb_devices_output, build_windows_toast_xml, parse_dumpsys_notifications,
    quote_android_shell_arg, read_frame, AndroidNotification, IpcRequestEnvelope,
    MAX_IPC_FRAME_BYTES,
};
use proptest::prelude::*;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime")
}

/// Undoes POSIX single-quote shell quoting the way `sh` does for one word.
fn posix_unquote(word: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = word.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => loop {
                match chars.next()? {
                    '\'' => break,
                    other => out.push(other),
                }
            },
            '\\' => out.push(chars.next()?),
            // Any unquoted shell metacharacter or whitespace would split or expand.
            other if other.is_whitespace() || "|&;<>()$`\"*?[#~=%".contains(other) => return None,
            other => out.push(other),
        }
    }
    Some(out)
}

/// Number of `<` in the Toast template itself; escaped user text adds none.
fn template_tag_count() -> usize {
    build_windows_toast_xml(&AndroidNotification {
        key: String::new(),
        package_name: "p".to_string(),
        title: Some("t".to_string()),
        text: Some("b".to_string()),
    })
    .matches('<')
    .count()
}

fn notification_dump() -> impl Strategy<Value = String> {
    let fragment = prop_oneof![
        Just("NotificationRecord(".to_string()),
        Just(" pkg=".to_string()),
        Just(" key=".to_string()),
        Just("android.title=String (".to_string()),
        Just("android.text=String (".to_string()),
        Just(")".to_string()),
        Just("\n  ".to_string()),
        Just("<script>&\"'</script>".to_string()),
        "[ -~]{0,20}",
        "\\PC{0,6}",
    ];
    proptest::collection::vec(fragment, 0..64).prop_map(|p| p.concat())
}

proptest! {
    #[test]
    fn ipc_frames_from_arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let rt = runtime();
        let mut reader: &[u8] = &bytes;
        let _ = rt.block_on(read_frame::<_, IpcRequestEnvelope>(&mut reader));
    }

    #[test]
    fn oversized_or_empty_ipc_lengths_are_rejected(len in prop_oneof![Just(0u32), (MAX_IPC_FRAME_BYTES as u32 + 1)..u32::MAX]) {
        let rt = runtime();
        let mut frame = len.to_be_bytes().to_vec();
        frame.extend_from_slice(b"{}");
        let mut reader: &[u8] = &frame;
        let result = rt.block_on(read_frame::<_, IpcRequestEnvelope>(&mut reader));
        prop_assert!(result.is_err());
    }

    #[test]
    fn adb_device_listing_parser_never_panics(text in "\\PC{0,256}") {
        for device in parse_adb_devices_output(&text) {
            prop_assert!(!device.serial.is_empty());
        }
    }

    #[test]
    fn notification_dump_parser_and_toast_xml_are_safe(dump in notification_dump()) {
        for notif in parse_dumpsys_notifications(&dump) {
            let xml = build_windows_toast_xml(&notif);
            // Exactly the template's tags: injected '<' must always be escaped.
            prop_assert_eq!(xml.matches('<').count(), template_tag_count(), "unescaped markup in {}", xml);
        }
    }

    #[test]
    fn toast_xml_escapes_arbitrary_text(title in "\\PC{0,64}", text in "\\PC{0,64}", pkg in "[a-z.]{1,24}") {
        let xml = build_windows_toast_xml(&AndroidNotification {
            key: "k".to_string(),
            package_name: pkg,
            title: Some(title),
            text: Some(text),
        });
        prop_assert_eq!(xml.matches('<').count(), template_tag_count());
    }

    #[test]
    fn android_shell_quoting_round_trips_as_one_word(input in "\\PC{0,128}") {
        let quoted = quote_android_shell_arg(&input);
        prop_assert_eq!(posix_unquote(&quoted), Some(input));
    }
}
