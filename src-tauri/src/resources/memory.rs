//! What the machine has, and what this process is using.
//!
//! The probes sit behind a trait for one reason: everything the resource policy
//! decides is a function of these numbers, and a test that reads the real
//! machine passes or fails depending on whose laptop runs it. Production uses
//! [`OsProbe`]; tests and headless callers hand in a [`FixedProbe`].
use serde::Serialize;
use std::path::Path;

/// Physical memory, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMemory {
    /// Installed RAM the operating system can address.
    pub total_physical: u64,
    /// RAM that could be handed out right now without paging something out.
    pub available_physical: u64,
}

/// This process's own memory use, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMemory {
    /// Resident now.
    pub working_set: u64,
    /// The highest the working set has been since the process started.
    pub peak_working_set: u64,
    /// Committed private memory. Unlike the working set this counts pages that
    /// were allocated but have been paged out, so it is the closer match for
    /// "how much has PhotoForge asked for".
    pub private_bytes: u64,
}

/// A source of machine measurements.
pub trait MemoryProbe: Send + Sync {
    fn system(&self) -> Option<SystemMemory>;
    fn process(&self) -> Option<ProcessMemory>;
}

/// The real machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsProbe;

impl MemoryProbe for OsProbe {
    fn system(&self) -> Option<SystemMemory> {
        os_system()
    }

    fn process(&self) -> Option<ProcessMemory> {
        os_process()
    }
}

/// A stand-in with fixed answers, for tests and for headless runs that must be
/// reproducible.
#[derive(Debug, Clone, Copy)]
pub struct FixedProbe {
    pub system: Option<SystemMemory>,
    pub process: Option<ProcessMemory>,
}

impl FixedProbe {
    pub const fn machine(total_physical: u64, available_physical: u64) -> Self {
        Self {
            system: Some(SystemMemory {
                total_physical,
                available_physical,
            }),
            process: None,
        }
    }

    /// A machine that cannot be measured, which the policy has to survive.
    pub const fn unmeasurable() -> Self {
        Self {
            system: None,
            process: None,
        }
    }
}

impl MemoryProbe for FixedProbe {
    fn system(&self) -> Option<SystemMemory> {
        self.system
    }

    fn process(&self) -> Option<ProcessMemory> {
        self.process
    }
}

#[cfg(windows)]
fn os_system() -> Option<SystemMemory> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `dwLength` is set as the API requires, and the pointer is to a
    // live, exclusively borrowed struct of exactly that size.
    unsafe { GlobalMemoryStatusEx(&mut status).ok()? };
    // A zero total would make every ratio meaningless; treat it as unmeasurable
    // rather than as a machine with no memory.
    (status.ullTotalPhys > 0).then_some(SystemMemory {
        total_physical: status.ullTotalPhys,
        available_physical: status.ullAvailPhys.min(status.ullTotalPhys),
    })
}

#[cfg(not(windows))]
fn os_system() -> Option<SystemMemory> {
    None
}

#[cfg(windows)]
fn os_process() -> Option<ProcessMemory> {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: `PROCESS_MEMORY_COUNTERS_EX` begins with a `PROCESS_MEMORY_COUNTERS`,
    // which is the documented way to request the extended counters, and `cb`
    // tells the API the true size. The pseudo-handle from `GetCurrentProcess`
    // needs no closing.
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
        .ok()?;
    }
    Some(ProcessMemory {
        working_set: counters.WorkingSetSize as u64,
        peak_working_set: counters.PeakWorkingSetSize as u64,
        private_bytes: counters.PrivateUsage as u64,
    })
}

#[cfg(not(windows))]
fn os_process() -> Option<ProcessMemory> {
    None
}

/// Free space on the volume holding `path`, in bytes available to this user.
///
/// This is what bounds a disk cache or any out-of-core store, so it is measured
/// rather than assumed. `None` means the volume could not be queried, and a
/// caller must treat that as "unknown", not as "plenty".
#[cfg(windows)]
pub fn free_disk_bytes(path: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide = HSTRING::from(path.as_os_str());
    let mut available = 0u64;
    // SAFETY: `wide` outlives the call and is NUL-terminated by HSTRING; the
    // output pointer is to a live u64 and the other two are optional.
    unsafe { GetDiskFreeSpaceExW(&wide, Some(&mut available), None, None).ok()? };
    Some(available)
}

#[cfg(not(windows))]
pub fn free_disk_bytes(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real probe has to return something self-consistent on a Windows
    /// machine, or the policy built on it is built on nothing.
    #[test]
    #[cfg(windows)]
    fn the_os_probe_reports_a_plausible_machine() {
        let system = OsProbe.system().expect("GlobalMemoryStatusEx failed");
        assert!(
            system.total_physical >= 512 * 1024 * 1024,
            "implausibly little RAM: {}",
            system.total_physical
        );
        assert!(system.available_physical <= system.total_physical);
        assert!(system.available_physical > 0);
    }

    #[test]
    #[cfg(windows)]
    fn the_process_probe_sees_this_process() {
        let process = OsProbe.process().expect("GetProcessMemoryInfo failed");
        assert!(process.working_set > 0);
        assert!(process.peak_working_set >= process.working_set);
        assert!(process.private_bytes > 0);
    }

    /// Allocating and touching memory has to move the number, or the estimator
    /// calibration built on it would be measuring nothing.
    #[test]
    #[cfg(windows)]
    fn touching_memory_raises_the_working_set() {
        let before = OsProbe.process().unwrap().working_set;
        let mut block = vec![0u8; 256 * 1024 * 1024];
        // Writing a non-zero value into every page forces it resident; the
        // allocator would otherwise hand back lazily-zeroed pages.
        for page in block.chunks_mut(4096) {
            page[0] = 1;
        }
        let during = OsProbe.process().unwrap().working_set;
        std::hint::black_box(&block);
        assert!(
            during >= before + 200 * 1024 * 1024,
            "256 MiB touched, working set moved by only {}",
            during.saturating_sub(before)
        );
    }

    #[test]
    #[cfg(windows)]
    fn free_disk_space_is_reported_for_the_temp_directory() {
        let free = free_disk_bytes(&std::env::temp_dir()).expect("volume query failed");
        assert!(free > 0);
    }

    #[test]
    fn a_fixed_probe_returns_exactly_what_it_was_given() {
        let probe = FixedProbe::machine(128 << 30, 103 << 30);
        let system = probe.system().unwrap();
        assert_eq!(system.total_physical, 128 << 30);
        assert_eq!(system.available_physical, 103 << 30);
        assert!(FixedProbe::unmeasurable().system().is_none());
    }
}
