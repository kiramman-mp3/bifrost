//! Bridge: Windows <-> Android communication layer, shared directories,
//! clipboard synchronization, notifications, and authenticated local IPC.

pub mod adb;
pub mod client;
pub mod clipboard;
pub mod files;
pub mod ipc;
pub mod notifications;
pub mod toast;

pub use adb::{
    online_devices_deduplicated, quote_android_shell_arg, AdbClient, AdbDevice, AdbEndpointState,
    GuestHealth,
};
pub use client::HeimdallClient;
pub use clipboard::{
    read_host_clipboard_text, sync_clipboard_to_android, validate_clipboard_text,
    write_host_clipboard_text, ANDROID_SET_CLIPBOARD_FROM_STDIN, MAX_CLIPBOARD_BYTES,
};
pub use files::{validate_guest_relative_path, SharedDirectoryBridge, SharedFileEntry};
pub use ipc::{
    constant_time_eq, default_pipe_name, generate_auth_token, read_frame, token_file_path,
    write_frame, DaemonHealthReport, IpcRequest, IpcRequestEnvelope, IpcResponseEnvelope,
    IPC_PROTOCOL_VERSION, MAX_IPC_FRAME_BYTES,
};
pub use notifications::{
    build_windows_toast_xml, dispatch_windows_toast, list_active_notifications,
    parse_dumpsys_notifications, AndroidNotification,
};
pub use toast::{ensure_toast_identity, show_toast_xml, BIFROST_AUMID};
