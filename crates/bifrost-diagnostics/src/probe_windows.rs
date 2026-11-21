//! Native Windows 11 host probes using Win32, DXGI, Registry, CPUID, and WHPX APIs.
//!
//! Every `unsafe` block is minimized and documented with explicit `// SAFETY:` invariants.

use std::ffi::{c_void, OsStr};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use tracing::debug;
use windows::core::{s, w, Interface, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD,
    REG_SZ, REG_VALUE_TYPE,
};
use windows::Win32::System::SystemInformation::{
    GetNativeSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
};
use windows::Win32::System::Threading::{IsProcessorFeaturePresent, PF_VIRT_FIRMWARE_ENABLED};

use crate::model::{
    CpuInfo, GpuAdapterInfo, MemoryInfo, OsInfo, StorageInfo, SystemSnapshot, WhpxInfo,
};

const MIN_WINDOWS_11_BUILD: u32 = 22_000;
const WHV_CAPABILITY_CODE_HYPERVISOR_PRESENT: u32 = 0;

/// Collects a full native Windows host snapshot.
pub(crate) fn probe_system(target_storage_path: &Path) -> SystemSnapshot {
    let os = probe_os();
    let cpu = probe_cpu();
    let whpx = probe_whpx();
    let memory = probe_memory();
    let storage = probe_storage(target_storage_path);
    let gpus = probe_gpus();

    debug!(
        build = os.build,
        edition = %os.edition_id,
        whpx_ready = whpx.api_available && whpx.hypervisor_present,
        gpu_count = gpus.len(),
        "completed native Windows host probe"
    );

    SystemSnapshot {
        os,
        cpu,
        whpx,
        memory,
        storage,
        gpus,
    }
}

#[repr(C)]
struct RtlOsVersionInfoW {
    dw_os_version_info_size: u32,
    dw_major_version: u32,
    dw_minor_version: u32,
    dw_build_number: u32,
    dw_platform_id: u32,
    sz_csd_version: [u16; 128],
}

fn probe_os() -> OsInfo {
    let (major, minor, build) = query_rtl_version().unwrap_or((0, 0, 0));
    let subkey = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let raw_product_name =
        read_reg_string(HKEY_LOCAL_MACHINE, subkey, w!("ProductName")).unwrap_or_default();
    let edition_id =
        read_reg_string(HKEY_LOCAL_MACHINE, subkey, w!("EditionID")).unwrap_or_default();
    let display_version =
        read_reg_string(HKEY_LOCAL_MACHINE, subkey, w!("DisplayVersion")).unwrap_or_default();
    let ubr = read_reg_dword(HKEY_LOCAL_MACHINE, subkey, w!("UBR")).unwrap_or(0);

    let is_windows_11 = major == 10 && build >= MIN_WINDOWS_11_BUILD;
    let product_name = if is_windows_11 && raw_product_name.starts_with("Windows 10") {
        raw_product_name.replacen("Windows 10", "Windows 11", 1)
    } else if raw_product_name.is_empty() {
        format!("Windows {major}.{minor}")
    } else {
        raw_product_name
    };

    OsInfo {
        product_name,
        edition_id,
        display_version,
        major,
        minor,
        build,
        ubr,
        is_windows_11,
    }
}

#[allow(unsafe_code)]
fn query_rtl_version() -> Option<(u32, u32, u32)> {
    type RtlGetVersionFn = unsafe extern "system" fn(*mut RtlOsVersionInfoW) -> i32;

    // SAFETY: `ntdll.dll` is always mapped in every Win32 process; `GetProcAddress`
    // returns either null or the standard `RtlGetVersion` function pointer.
    let func_ptr = unsafe {
        let hmod = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        GetProcAddress(hmod, s!("RtlGetVersion"))?
    };

    // SAFETY: `RtlGetVersion` matches `RtlGetVersionFn` ABI (`NTSTATUS RtlGetVersion(PRTL_OSVERSIONINFOW)`).
    let rtl_get_version: RtlGetVersionFn = unsafe { std::mem::transmute(func_ptr) };

    let mut info = RtlOsVersionInfoW {
        dw_os_version_info_size: size_of::<RtlOsVersionInfoW>() as u32,
        dw_major_version: 0,
        dw_minor_version: 0,
        dw_build_number: 0,
        dw_platform_id: 0,
        sz_csd_version: [0u16; 128],
    };

    // SAFETY: `info` is properly initialized with exact struct byte size and valid writable pointer.
    let status = unsafe { rtl_get_version(&mut info) };
    if status == 0 {
        Some((
            info.dw_major_version,
            info.dw_minor_version,
            info.dw_build_number,
        ))
    } else {
        None
    }
}

#[allow(unsafe_code)]
fn probe_cpu() -> CpuInfo {
    let mut sys_info = SYSTEM_INFO::default();
    // SAFETY: `GetNativeSystemInfo` writes into a valid mutable `SYSTEM_INFO` struct.
    unsafe { GetNativeSystemInfo(&mut sys_info) };

    let brand = read_reg_string(
        HKEY_LOCAL_MACHINE,
        w!(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"),
        w!("ProcessorNameString"),
    )
    .map(|s| s.trim().to_string())
    .unwrap_or_else(|| "Unknown CPU".to_string());

    // SAFETY: `IsProcessorFeaturePresent` is a pure read-only Win32 query.
    let firmware_virt_reported =
        unsafe { IsProcessorFeaturePresent(PF_VIRT_FIRMWARE_ENABLED) }.as_bool();

    let (vmx_or_svm_exposed, hypervisor_present, hypervisor_vendor) = probe_cpuid();

    CpuInfo {
        brand,
        architecture: std::env::consts::ARCH.to_string(),
        logical_cores: sys_info.dwNumberOfProcessors,
        vmx_or_svm_exposed,
        firmware_virt_reported,
        hypervisor_present,
        hypervisor_vendor,
    }
}

#[cfg(target_arch = "x86_64")]
fn probe_cpuid() -> (bool, bool, Option<String>) {
    use std::arch::x86_64::__cpuid;

    let leaf1 = __cpuid(1);
    let vmx = (leaf1.ecx & (1 << 5)) != 0;
    let hypervisor_present = (leaf1.ecx & (1 << 31)) != 0;

    let ext_max = __cpuid(0x8000_0000).eax;
    let svm = if ext_max >= 0x8000_0001 {
        let ext1 = __cpuid(0x8000_0001);
        (ext1.ecx & (1 << 2)) != 0
    } else {
        false
    };

    let vendor = if hypervisor_present {
        let hv = __cpuid(0x4000_0000);
        let mut bytes = [0u8; 12];
        bytes[0..4].copy_from_slice(&hv.ebx.to_le_bytes());
        bytes[4..8].copy_from_slice(&hv.ecx.to_le_bytes());
        bytes[8..12].copy_from_slice(&hv.edx.to_le_bytes());
        let text = String::from_utf8_lossy(&bytes)
            .trim_matches('\0')
            .trim()
            .to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    } else {
        None
    };

    (vmx || svm, hypervisor_present, vendor)
}

#[cfg(not(target_arch = "x86_64"))]
fn probe_cpuid() -> (bool, bool, Option<String>) {
    (false, false, None)
}

#[allow(unsafe_code)]
fn probe_whpx() -> WhpxInfo {
    type WhvGetCapabilityFn = unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;
    type WhvCreatePartitionFn = unsafe extern "system" fn(*mut *mut c_void) -> i32;
    type WhvDeletePartitionFn = unsafe extern "system" fn(*mut c_void) -> i32;

    // SAFETY: `LoadLibraryW` loads `WinHvPlatform.dll` if present on the Windows system path.
    let module: HMODULE = match unsafe { LoadLibraryW(w!("WinHvPlatform.dll")) } {
        Ok(m) if !m.is_invalid() => m,
        _ => {
            return WhpxInfo {
                dll_present: false,
                api_available: false,
                hypervisor_present: false,
                partition_create_supported: false,
                hresult: None,
                partition_hresult: None,
                feature_likely_disabled: true,
            };
        }
    };

    // SAFETY: `module` is a valid loaded DLL handle; every resolved export matches the
    // documented WinHvPlatform.h C ABI, any created partition is immediately deleted, and
    // `FreeLibrary(module)` is always called before returning.
    unsafe {
        let get_cap_proc = GetProcAddress(module, s!("WHvGetCapability"));
        let create_proc = GetProcAddress(module, s!("WHvCreatePartition"));
        let delete_proc = GetProcAddress(module, s!("WHvDeletePartition"));

        let (api_available, hypervisor_present, hresult) = if let Some(func_ptr) = get_cap_proc {
            let get_cap: WhvGetCapabilityFn = std::mem::transmute(func_ptr);
            let mut capability_value: u64 = 0;
            let mut written_size: u32 = 0;
            let hr = get_cap(
                WHV_CAPABILITY_CODE_HYPERVISOR_PRESENT,
                (&mut capability_value as *mut u64).cast::<c_void>(),
                size_of::<u64>() as u32,
                &mut written_size,
            );
            let hv_present = hr >= 0 && (capability_value & 0xFFFF_FFFF) != 0;
            (hr >= 0, hv_present, Some(hr))
        } else {
            (false, false, None)
        };

        let (partition_create_supported, partition_hresult) =
            if let (Some(create_ptr), Some(delete_ptr)) = (create_proc, delete_proc) {
                let create_part: WhvCreatePartitionFn = std::mem::transmute(create_ptr);
                let delete_part: WhvDeletePartitionFn = std::mem::transmute(delete_ptr);
                let mut handle: *mut c_void = std::ptr::null_mut();
                let hr = create_part(&mut handle);
                if hr >= 0 && !handle.is_null() {
                    let _ = delete_part(handle);
                    (true, Some(hr))
                } else {
                    (false, Some(hr))
                }
            } else {
                (false, None)
            };

        let _ = FreeLibrary(module);

        WhpxInfo {
            dll_present: true,
            api_available,
            hypervisor_present,
            partition_create_supported,
            hresult,
            partition_hresult,
            feature_likely_disabled: !(hypervisor_present && partition_create_supported),
        }
    }
}

#[allow(unsafe_code)]
fn probe_memory() -> MemoryInfo {
    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };

    // SAFETY: `status.dwLength` is initialized to the exact size of `MEMORYSTATUSEX`.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) }.is_ok();
    if ok {
        MemoryInfo {
            total_bytes: status.ullTotalPhys,
            available_bytes: status.ullAvailPhys,
        }
    } else {
        MemoryInfo {
            total_bytes: 0,
            available_bytes: 0,
        }
    }
}

#[allow(unsafe_code)]
fn probe_storage(target_path: &Path) -> StorageInfo {
    let probe_dir = nearest_existing_dir(target_path);
    let wide = to_wide_null(probe_dir.as_os_str());
    let mut free_available: u64 = 0;
    let mut total_bytes: u64 = 0;

    // SAFETY: `wide` is NUL-terminated UTF-16 and output pointers are valid writable locals.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            Some(&mut free_available),
            Some(&mut total_bytes),
            None,
        )
    }
    .is_ok();

    if ok {
        StorageInfo {
            target_path: probe_dir,
            total_bytes,
            available_bytes: free_available,
        }
    } else {
        StorageInfo {
            target_path: probe_dir,
            total_bytes: 0,
            available_bytes: 0,
        }
    }
}

fn nearest_existing_dir(path: &Path) -> PathBuf {
    let mut cursor = Some(path);
    while let Some(dir) = cursor {
        if dir.exists() {
            return dir.to_path_buf();
        }
        cursor = dir.parent();
    }
    PathBuf::from(r"C:\")
}

#[allow(unsafe_code)]
fn probe_gpus() -> Vec<GpuAdapterInfo> {
    let mut adapters = Vec::new();

    // SAFETY: Standard DXGI 1.1 factory creation and adapter enumeration with COM RAII wrappers.
    let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
        Ok(f) => f,
        Err(_) => return adapters,
    };

    let mut index: u32 = 0;
    loop {
        // SAFETY: `EnumAdapters1` returns `Err(DXGI_ERROR_NOT_FOUND)` once all adapters are visited.
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(a) => a,
            Err(_) => break,
        };
        index += 1;

        // SAFETY: `GetDesc1` populates a `DXGI_ADAPTER_DESC1` struct for this valid COM adapter.
        let desc = match unsafe { adapter.GetDesc1() } {
            Ok(d) => d,
            Err(_) => continue,
        };

        let nul_pos = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..nul_pos])
            .trim()
            .to_string();
        let is_software = (desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32)) != 0;

        // SAFETY: `CheckInterfaceSupport` queries the WDDM user-mode driver version packed as 4x16 bits.
        let driver_version = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }
            .ok()
            .map(decode_driver_version);

        adapters.push(GpuAdapterInfo {
            name,
            vendor_id: desc.VendorId,
            vendor_name: vendor_name_from_id(desc.VendorId).to_string(),
            dedicated_vram_bytes: desc.DedicatedVideoMemory as u64,
            shared_system_memory_bytes: desc.SharedSystemMemory as u64,
            driver_version,
            is_software,
        });
    }

    adapters
}

fn decode_driver_version(raw: i64) -> String {
    let bits = raw as u64;
    let major = (bits >> 48) & 0xFFFF;
    let minor = (bits >> 32) & 0xFFFF;
    let build = (bits >> 16) & 0xFFFF;
    let rev = bits & 0xFFFF;
    format!("{major}.{minor}.{build}.{rev}")
}

fn vendor_name_from_id(vendor_id: u32) -> &'static str {
    match vendor_id {
        0x8086 => "Intel",
        0x10DE => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x1414 => "Microsoft",
        0x5143 => "Qualcomm",
        _ => "Unknown Vendor",
    }
}

#[allow(unsafe_code)]
fn read_reg_string(root: HKEY, subkey: PCWSTR, value_name: PCWSTR) -> Option<String> {
    let mut hkey = HKEY::default();
    // SAFETY: Opening a read-only predefined HKLM registry key; closed before returning.
    if unsafe { RegOpenKeyExW(root, subkey, 0, KEY_READ, &mut hkey) }.is_err() {
        return None;
    }

    let mut reg_type = REG_VALUE_TYPE::default();
    let mut byte_len: u32 = 0;

    // SAFETY: First call queries required buffer byte length with null data pointer.
    let size_res = unsafe {
        RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&mut reg_type),
            None,
            Some(&mut byte_len),
        )
    };

    let output = if size_res.is_ok() && reg_type == REG_SZ && byte_len >= 2 {
        let mut buf = vec![0u8; byte_len as usize];
        // SAFETY: `buf` has exact byte capacity reported by `RegQueryValueExW`.
        let data_res = unsafe {
            RegQueryValueExW(
                hkey,
                value_name,
                None,
                Some(&mut reg_type),
                Some(buf.as_mut_ptr()),
                Some(&mut byte_len),
            )
        };
        if data_res.is_ok() {
            let u16s: Vec<u16> = buf
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                .collect();
            let nul_pos = u16s.iter().position(|&c| c == 0).unwrap_or(u16s.len());
            Some(String::from_utf16_lossy(&u16s[..nul_pos]))
        } else {
            None
        }
    } else {
        None
    };

    // SAFETY: `hkey` was successfully opened above and is closed exactly once.
    unsafe {
        let _ = RegCloseKey(hkey);
    }

    output
}

#[allow(unsafe_code)]
fn read_reg_dword(root: HKEY, subkey: PCWSTR, value_name: PCWSTR) -> Option<u32> {
    let mut hkey = HKEY::default();
    // SAFETY: Opening a read-only HKLM key; closed before returning.
    if unsafe { RegOpenKeyExW(root, subkey, 0, KEY_READ, &mut hkey) }.is_err() {
        return None;
    }

    let mut reg_type = REG_VALUE_TYPE::default();
    // SAFETY: Zero-initialized 4-byte buffer for `REG_DWORD`.
    let mut value: u32 = unsafe { zeroed() };
    let mut byte_len: u32 = size_of::<u32>() as u32;

    // SAFETY: `value` pointer has exact size `byte_len == 4`.
    let res = unsafe {
        RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&mut reg_type),
            Some((&mut value as *mut u32).cast::<u8>()),
            Some(&mut byte_len),
        )
    };

    // SAFETY: `hkey` was successfully opened above and is closed exactly once.
    unsafe {
        let _ = RegCloseKey(hkey);
    }

    if res.is_ok() && reg_type == REG_DWORD {
        Some(value)
    } else {
        None
    }
}

fn to_wide_null(input: &OsStr) -> Vec<u16> {
    input.encode_wide().chain(std::iter::once(0)).collect()
}
