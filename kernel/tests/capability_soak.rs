//! Long-running capability lifetime and stale-handle campaign.

use std::fs;
use std::path::{Path, PathBuf};

use ghostos_kernel::{
    AddressSpaceId, CapabilityError, CapabilityHandle, CapabilityObject, CapabilitySpace, Rights,
};

const CAPACITY: usize = 8;
const DEFAULT_CYCLES: usize = 512;

#[derive(Clone, Copy, Debug, Default)]
struct CampaignStats {
    cycles_requested: usize,
    cycles_completed: usize,
    stale_rejections: usize,
    generation_reuses: usize,
    peak_live_capabilities: usize,
}

fn cycles() -> usize {
    std::env::var("GHOSTOS_CAPABILITY_SOAK_CYCLES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CYCLES)
}

fn report_path() -> PathBuf {
    std::env::var_os("GHOSTOS_CAPABILITY_SOAK_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("build/soak/capabilities/report.json"))
}

fn expect_stale(
    capabilities: &CapabilitySpace<CAPACITY>,
    owner: AddressSpaceId,
    handle: CapabilityHandle,
    object: CapabilityObject,
) -> Result<(), String> {
    if capabilities.inspect(owner, handle) != Err(CapabilityError::InvalidHandle) {
        return Err(format!("stale handle {:#x} was inspectable", handle.raw()));
    }
    if capabilities.authorize(owner, handle, object, Rights::READ)
        != Err(CapabilityError::InvalidHandle)
    {
        return Err(format!("stale handle {:#x} authorized a read", handle.raw()));
    }
    Ok(())
}

fn run_campaign(requested: usize) -> (CampaignStats, Option<String>) {
    let owner = AddressSpaceId::new(1).expect("valid capability soak owner");
    let borrower = AddressSpaceId::new(2).expect("valid capability soak borrower");
    let observer = AddressSpaceId::new(3).expect("valid capability soak observer");
    let object = CapabilityObject::AddressSpace(owner);
    let rights = Rights::READ.union(Rights::DELEGATE).union(Rights::REVOKE);
    let mut capabilities = CapabilitySpace::<CAPACITY>::new();
    let mut previous_handles: Option<[CapabilityHandle; 5]> = None;
    let mut stats = CampaignStats {
        cycles_requested: requested,
        ..CampaignStats::default()
    };

    for cycle in 1..=requested {
        if let Some(handles) = previous_handles {
            for handle in handles {
                if let Err(error) = expect_stale(&capabilities, owner, handle, object) {
                    return (stats, Some(format!("cycle {cycle}: {error}")));
                }
                stats.stale_rejections += 2;
            }
        }

        let root = match capabilities.mint_root(owner, object, rights) {
            Ok(handle) => handle,
            Err(error) => return (stats, Some(format!("cycle {cycle}: mint root: {error:?}"))),
        };
        let child = match capabilities.delegate(owner, root, borrower, rights) {
            Ok(handle) => handle,
            Err(error) => return (stats, Some(format!("cycle {cycle}: delegate child: {error:?}"))),
        };
        let grandchild = match capabilities.delegate(borrower, child, observer, Rights::READ) {
            Ok(handle) => handle,
            Err(error) => {
                return (
                    stats,
                    Some(format!("cycle {cycle}: delegate grandchild: {error:?}")),
                )
            }
        };
        stats.peak_live_capabilities = stats.peak_live_capabilities.max(capabilities.used());
        if let Err(error) = capabilities.check_invariants() {
            return (stats, Some(format!("cycle {cycle}: live invariant: {error:?}")));
        }

        let revoked = match capabilities.revoke(owner, root) {
            Ok(count) => count,
            Err(error) => return (stats, Some(format!("cycle {cycle}: revoke: {error:?}"))),
        };
        if revoked != 2 || capabilities.used() != 1 {
            return (
                stats,
                Some(format!(
                    "cycle {cycle}: revoke left {revoked} descendants and {} live capabilities",
                    capabilities.used()
                )),
            );
        }
        for handle in [child, grandchild] {
            if let Err(error) = expect_stale(&capabilities, owner, handle, object) {
                return (stats, Some(format!("cycle {cycle}: {error}")));
            }
            stats.stale_rejections += 2;
        }

        let replacement = match capabilities.mint_root(owner, object, rights) {
            Ok(handle) => handle,
            Err(error) => {
                return (
                    stats,
                    Some(format!("cycle {cycle}: mint replacement root: {error:?}")),
                )
            }
        };
        if replacement == child || replacement == grandchild {
            return (stats, Some(format!("cycle {cycle}: revoked handle was reused")));
        }
        stats.generation_reuses += 1;

        let replacement_child = match capabilities.delegate(owner, replacement, borrower, rights) {
            Ok(handle) => handle,
            Err(error) => {
                return (
                    stats,
                    Some(format!("cycle {cycle}: delegate replacement: {error:?}")),
                )
            }
        };
        stats.peak_live_capabilities = stats.peak_live_capabilities.max(capabilities.used());

        if let Err(error) = capabilities.delete(owner, replacement) {
            return (stats, Some(format!("cycle {cycle}: delete: {error:?}")));
        }
        if let Err(error) = capabilities.delete(owner, root) {
            return (stats, Some(format!("cycle {cycle}: delete authority: {error:?}")));
        }
        if capabilities.used() != 0 {
            return (
                stats,
                Some(format!(
                    "cycle {cycle}: capability leak left {} live descriptors",
                    capabilities.used()
                )),
            );
        }
        if let Err(error) = expect_stale(&capabilities, owner, root, object) {
            return (stats, Some(format!("cycle {cycle}: {error}")));
        }
        stats.stale_rejections += 2;
        if let Err(error) = capabilities.check_invariants() {
            return (stats, Some(format!("cycle {cycle}: empty invariant: {error:?}")));
        }

        previous_handles = Some([root, child, grandchild, replacement, replacement_child]);
        stats.cycles_completed = cycle;
    }

    (stats, None)
}

fn write_report(path: &Path, stats: CampaignStats, failure: Option<&str>) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create capability soak report directory");
    }
    let state = if failure.is_none() && stats.cycles_completed == stats.cycles_requested {
        "passed"
    } else {
        "failed"
    };
    let failure_json = failure
        .map(|value| format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
        .unwrap_or_else(|| "null".to_string());
    let output = format!(
        "{{\n  \"schema\": 1,\n  \"campaign\": \"capabilities\",\n  \"cycles_requested\": {},\n  \"cycles_completed\": {},\n  \"stale_rejections\": {},\n  \"generation_reuses\": {},\n  \"peak_live_capabilities\": {},\n  \"zero_live_capabilities_after_cycle\": {},\n  \"result_state\": \"{}\",\n  \"failure\": {}\n}}\n",
        stats.cycles_requested,
        stats.cycles_completed,
        stats.stale_rejections,
        stats.generation_reuses,
        stats.peak_live_capabilities,
        state == "passed",
        state,
        failure_json,
    );
    fs::write(path, output).expect("write capability soak report");
}

#[test]
fn capability_soak_reclaims_slots_and_rejects_stale_handles() {
    let requested = cycles();
    let path = report_path();
    let (stats, failure) = run_campaign(requested);
    write_report(&path, stats, failure.as_deref());
    assert!(
        failure.is_none(),
        "capability soak failed; retained report: {}",
        path.display()
    );
}
