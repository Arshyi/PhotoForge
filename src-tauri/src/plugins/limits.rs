//! The sandbox's limits, as numbers, with the reasoning for each.
//!
//! These are constants a plugin cannot raise. A manifest may *ask* for memory
//! (`limits.memoryMib`) and gets no more than the host ceiling and no more than a
//! quarter of the memory budget; it cannot ask for time or work at all, because a
//! limit the plugin sets is a limit the plugin can remove.
//!
//! Nothing here claims to be "dynamic" beyond what it is: the memory a call may use
//! follows the machine through the resource budget, and the time and work allowed
//! follow the size of the image being processed.
use std::time::Duration;

pub const MIB: u64 = 1 << 20;

/// The most linear memory any one call may have, whatever the machine. A 32-bit
/// WebAssembly memory addresses 4 GiB; half of that leaves the `i32` pointers the
/// interface uses unambiguous and leaves headroom for the guest's own allocator.
pub const HOST_MEMORY_CEILING: u64 = 2048 * MIB;
/// What a plugin gets when its manifest does not say.
pub const DEFAULT_MEMORY: u64 = 256 * MIB;
/// A plugin may use at most this fraction of PhotoForge's memory budget at once,
/// because the document itself needs the rest.
pub const BUDGET_DIVISOR: u64 = 4;
/// Fixed memory a call needs beyond its pixels: the module's own stack and data.
pub const MEMORY_OVERHEAD: u64 = 8 * MIB;

/// Bytes of pixel data per pixel: four `f32` channels.
pub const BYTES_PER_PIXEL: u64 = 16;

/// Table elements a module may hold. Tables hold function references; a module with
/// more than this is either enormous or hostile.
pub const MAX_TABLE_ELEMENTS: u32 = 10_000;
pub const MAX_INSTANCES: usize = 1;
pub const MAX_MEMORIES: usize = 1;
pub const MAX_TABLES: usize = 4;

/// Work allowed to one call: a fixed part for start-up, and a part that grows with
/// the pixels the call processes. Wasmtime counts roughly one unit per executed
/// WebAssembly instruction, and a heavy per-pixel filter is a few thousand, so
/// 20,000 per pixel is generous for an honest plugin and still bounds a malicious one.
pub const FUEL_BASE: u64 = 10_000_000;
pub const FUEL_PER_PIXEL: u64 = 20_000;
pub const FUEL_CEILING: u64 = 500_000_000_000;

/// Wall-clock time for one call, which is what fuel cannot bound: a call that is
/// slow because the machine is, or that is stuck in the host. Cancellation and this
/// share one mechanism, an epoch check at every loop back-edge.
pub const TIME_BASE: Duration = Duration::from_secs(20);
pub const TIME_PER_MEGAPIXEL: Duration = Duration::from_millis(2500);
pub const TIME_CEILING: Duration = Duration::from_secs(15 * 60);
/// How often the epoch advances, so the resolution of every time limit and of
/// cancellation.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// Log output a call may produce: guest text is untrusted and ends up in a file and
/// on screen.
pub const LOG_LINES_PER_CALL: usize = 32;
pub const LOG_LINE_BYTES: usize = 240;

/// The tile edge for filters that can be tiled.
pub const TILE_EDGE: u32 = 256;

/// The limits for one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallLimits {
    pub memory_bytes: u64,
    pub fuel: u64,
    pub time: Duration,
}

/// Memory a manifest's request becomes: what it asked for (or the default), at most
/// the ceiling, at most a quarter of the budget.
pub fn memory_allowance(requested_mib: Option<u32>, budget_bytes: u64) -> u64 {
    let requested = requested_mib.map_or(DEFAULT_MEMORY, |mib| u64::from(mib) * MIB);
    requested
        .min(HOST_MEMORY_CEILING)
        .min((budget_bytes / BUDGET_DIVISOR).max(MEMORY_OVERHEAD * 2))
}

/// The limits for a call that processes `pixels` pixels.
pub fn call_limits(memory_bytes: u64, pixels: u64) -> CallLimits {
    let fuel = FUEL_BASE
        .saturating_add(pixels.saturating_mul(FUEL_PER_PIXEL))
        .min(FUEL_CEILING);
    let megapixels = pixels.div_ceil(1_000_000);
    let time = TIME_BASE
        .saturating_add(
            TIME_PER_MEGAPIXEL.saturating_mul(megapixels.min(u64::from(u32::MAX)) as u32),
        )
        .min(TIME_CEILING);
    CallLimits {
        memory_bytes,
        fuel,
        time,
    }
}

/// Guest memory a call needs for an input window and an output rectangle.
pub fn call_memory_required(input_pixels: u64, output_pixels: u64, parameter_count: usize) -> u64 {
    input_pixels
        .saturating_add(output_pixels)
        .saturating_mul(BYTES_PER_PIXEL)
        .saturating_add((parameter_count as u64) * 8)
        .saturating_add(MEMORY_OVERHEAD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_clamped_by_the_ceiling_and_by_the_budget() {
        let budget = 16 * 1024 * MIB;
        assert_eq!(memory_allowance(None, budget), DEFAULT_MEMORY);
        assert_eq!(memory_allowance(Some(64), budget), 64 * MIB);
        assert_eq!(memory_allowance(Some(2048), budget), HOST_MEMORY_CEILING);
        // A plugin cannot talk its way past a small machine.
        assert_eq!(memory_allowance(Some(2048), 2048 * MIB), 512 * MIB);
        assert_eq!(memory_allowance(Some(2048), 600 * MIB), 150 * MIB);
        // And a tiny budget still leaves something to run in.
        assert!(memory_allowance(Some(2048), 0) >= 2 * MEMORY_OVERHEAD);
    }

    #[test]
    fn work_and_time_grow_with_the_image_and_stop_at_a_ceiling() {
        let small = call_limits(MIB, 65_536);
        let large = call_limits(MIB, 45_000_000);
        assert!(large.fuel > small.fuel && large.time > small.time);
        assert_eq!(small.fuel, FUEL_BASE + 65_536 * FUEL_PER_PIXEL);
        let absurd = call_limits(MIB, u64::MAX);
        assert_eq!(absurd.fuel, FUEL_CEILING);
        assert_eq!(absurd.time, TIME_CEILING);
    }

    #[test]
    fn memory_required_counts_both_buffers_and_the_overhead() {
        assert_eq!(
            call_memory_required(100, 50, 3),
            150 * 16 + 24 + MEMORY_OVERHEAD
        );
        assert_eq!(call_memory_required(u64::MAX, u64::MAX, 0), u64::MAX);
    }
}
