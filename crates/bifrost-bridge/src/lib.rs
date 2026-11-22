//! Bridge: Windows <-> Android communication layer and authenticated local IPC.

#![forbid(unsafe_code)]

pub mod adb;
pub mod client;
pub mod ipc;

pub use adb::{AdbClient, AdbDevice, GuestHealth};
pub use client::HeimdallClient;
pub use ipc::{
    constant_time_eq, default_pipe_name, generate_auth_token, read_frame, token_file_path,
    write_frame, DaemonHealthReport, IpcRequest, IpcRequestEnvelope, IpcResponseEnvelope,
    IPC_PROTOCOL_VERSION, MAX_IPC_FRAME_BYTES,
};
