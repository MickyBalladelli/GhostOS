//! Boot GhostOS with a host disk image attached to AHCI, NVMe, or virtio-blk.
//!
//! Run with:
//! cargo run --release --example storage-bios -- \
//!   --controller ahci --kernel path/to/kernel.bin \
//!   --initrd path/to/initrd.img --disk path/to/disk.raw

use std::path::PathBuf;

use ghostos_vm::{DiskImage, FirmwareMode, Vm, VmConfig};

fn value(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{name} needs a value"))
}

fn main() -> Result<(), String> {
    let mut controller = String::from("ahci");
    let mut kernel = None;
    let mut initrd = None;
    let mut disk = None;
    let mut steps = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--controller" => controller = value(&mut args, "--controller")?,
            "--kernel" => kernel = Some(PathBuf::from(value(&mut args, "--kernel")?)),
            "--initrd" => initrd = Some(PathBuf::from(value(&mut args, "--initrd")?)),
            "--disk" => disk = Some(PathBuf::from(value(&mut args, "--disk")?)),
            "--steps" => {
                steps = Some(
                    value(&mut args, "--steps")?
                        .parse::<u64>()
                        .map_err(|_| "--steps must be an integer".to_string())?,
                )
            }
            "--help" | "-h" => {
                println!(
                    "usage: storage-bios --controller ahci|nvme|virtio-blk \\\n                     --kernel PATH --initrd PATH --disk PATH [--steps COUNT]"
                );
                return Ok(())
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }

    let kernel = kernel.ok_or_else(|| "--kernel is required".to_string())?;
    let initrd = initrd.ok_or_else(|| "--initrd is required".to_string())?;
    let disk = DiskImage::open(disk.ok_or_else(|| "--disk is required".to_string())?)
        .map_err(|error| format!("cannot open disk image: {error}"))?;

    let mut vm = Vm::with_config(VmConfig {
        firmware: FirmwareMode::Bios,
        kernel_path: Some(kernel),
        initrd_path: Some(initrd),
        boot_args: String::from("console=serial0"),
        max_steps: steps,
        ..VmConfig::default()
    });

    match controller.as_str() {
        "ahci" => {
            vm.ahci().borrow_mut().attach_disk(disk);
        }
        "nvme" => {
            vm.nvme().borrow_mut().attach_namespace(disk);
        }
        "virtio-blk" => {
            vm.virtio_blk().borrow_mut().attach_disk(disk);
        }
        other => return Err(format!("unknown controller `{other}`")),
    }

    match steps {
        Some(count) => {
            let report = vm
                .run_for_steps(count)
                .map_err(|error| format!("VM error: {error:?}"))?;
            println!(
                "VM stopped after {} steps at RIP 0x{:016x} (halted={})",
                report.steps, report.rip, report.halted
            );
        }
        None => vm.run().map_err(|error| format!("VM error: {error:?}"))?,
    }

    Ok(())
}
