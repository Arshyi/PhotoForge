//! What the admission planner decides, for each format and size, on machines of
//! different sizes. Nothing here decodes a file: it feeds the planner the header
//! facts a real probe would produce, and the budget each machine's automatic policy
//! gives, so the table is the planner's own answer and not a paraphrase of it.
//!
//! ```text
//! cargo run --release --example admission_matrix
//! ```
use photoforge_lib::raw::develop::DEVELOP_BYTES_PER_PHOTOSITE;
use photoforge_lib::resources::admission::{
    plan, AdmissionOption, DecodeCapabilities, ReducedDecode, RegionDecode, SourceKind,
    SourceProbe, Verdict,
};
use photoforge_lib::resources::memory::SystemMemory;
use photoforge_lib::resources::policy::{compute_budget, BudgetMode, ResourceLimits, GIB};
use photoforge_lib::source::dng::{self, DngInfo};

struct Machine {
    name: &'static str,
    total: u64,
    available: u64,
}

fn probe(kind: &str, megapixels: u64) -> SourceProbe {
    let pixels = megapixels * 1_000_000;
    // A 4:3 frame.
    let height = ((pixels as f64 * 3.0 / 4.0).sqrt()) as u64;
    let width = pixels / height;
    match kind {
        "PNG" => SourceProbe {
            kind: SourceKind::Png,
            width,
            height,
            native_bytes_per_pixel: 3,
            full_decode_bytes_per_pixel: 3,
            file_bytes: pixels * 3 / 2,
            capabilities: DecodeCapabilities {
                region: RegionDecode::Rows,
                reduced: ReducedDecode::Rows,
            },
        },
        "JPEG" => SourceProbe {
            kind: SourceKind::Jpeg,
            width,
            height,
            native_bytes_per_pixel: 3,
            full_decode_bytes_per_pixel: 3,
            file_bytes: pixels / 4,
            capabilities: DecodeCapabilities {
                region: RegionDecode::TransientFull { bytes_per_pixel: 3 },
                reduced: ReducedDecode::DctScaled,
            },
        },
        "WebP" => SourceProbe {
            kind: SourceKind::WebP,
            width,
            height,
            native_bytes_per_pixel: 3,
            full_decode_bytes_per_pixel: 7,
            file_bytes: pixels,
            capabilities: DecodeCapabilities {
                region: RegionDecode::TransientFull { bytes_per_pixel: 7 },
                reduced: ReducedDecode::TransientFull { bytes_per_pixel: 7 },
            },
        },
        _ => {
            // A tiled DNG, 256-photosite tiles.
            let info = DngInfo {
                width: width as u32,
                height: height as u32,
                bits: 14,
                tiled: true,
                segment_pixels: 256 * 256,
                segments: (width.div_ceil(256) * height.div_ceil(256)) as usize,
            };
            SourceProbe {
                kind: SourceKind::Dng,
                width,
                height,
                native_bytes_per_pixel: 4,
                full_decode_bytes_per_pixel: DEVELOP_BYTES_PER_PHOTOSITE,
                file_bytes: pixels * 2,
                capabilities: dng::capabilities(&info),
            }
        }
    }
}

fn cell(probe: &SourceProbe, machine: &Machine) -> String {
    let system = SystemMemory {
        total_physical: machine.total * GIB,
        available_physical: machine.available * GIB,
    };
    let budget = compute_budget(BudgetMode::Automatic, Some(system));
    let limits = ResourceLimits::from_budget(budget.bytes);
    let report = plan(probe, &budget, &limits, Some(system.available_physical), 0);
    let region = report.options.iter().find_map(|option| match option {
        AdmissionOption::OpenRegion {
            max_region_pixels, ..
        } => Some(*max_region_pixels),
        _ => None,
    });
    let reduced = report.options.iter().find_map(|option| match option {
        AdmissionOption::OpenReduced { max_scale, .. } => Some(*max_scale),
        _ => None,
    });
    let verdict = match report.verdict {
        Verdict::FullResolution => "whole".to_string(),
        Verdict::FullResolutionWithWarning {
            peak_percent_of_budget,
        } => format!("whole ({peak_percent_of_budget}% of budget)"),
        Verdict::RegionRequired => {
            format!("region ≤ {:.0} MP", region.unwrap_or(0) as f64 / 1e6)
        }
        Verdict::ReducedCopyRecommended => {
            format!("reduced ≤ {:.0}% per side", reduced.unwrap_or(0.0) * 100.0)
        }
        Verdict::InsufficientResources { .. } => "insufficient".to_string(),
        Verdict::Unsafe { .. } => "refused".to_string(),
    };
    verdict
}

fn main() {
    let machines = [
        Machine {
            name: "8 GiB laptop, 4 GiB free",
            total: 8,
            available: 4,
        },
        Machine {
            name: "16 GiB, 9 GiB free",
            total: 16,
            available: 9,
        },
        Machine {
            name: "32 GiB, 24 GiB free",
            total: 32,
            available: 24,
        },
        Machine {
            name: "128 GiB, 103 GiB free",
            total: 128,
            available: 103,
        },
    ];
    for machine in &machines {
        let system = SystemMemory {
            total_physical: machine.total * GIB,
            available_physical: machine.available * GIB,
        };
        let budget = compute_budget(BudgetMode::Automatic, Some(system));
        println!(
            "\n### {} — budget {:.1} GiB, largest image whole {:.0} MP\n",
            machine.name,
            budget.bytes as f64 / GIB as f64,
            ResourceLimits::from_budget(budget.bytes).working_pixels() as f64 / 1e6
        );
        println!("| Source | 12 MP | 24 MP | 45 MP | 100 MP | 200 MP |");
        println!("| --- | --- | --- | --- | --- | --- |");
        for kind in ["PNG", "JPEG", "WebP", "DNG"] {
            let row: Vec<String> = [12, 24, 45, 100, 200]
                .iter()
                .map(|mp| cell(&probe(kind, *mp), machine))
                .collect();
            println!("| {kind} | {} |", row.join(" | "));
        }
    }
}
