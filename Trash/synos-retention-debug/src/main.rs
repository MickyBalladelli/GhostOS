use synos_synfs::{SynfsPurged, SynFs, RmsMapHandle};

const BLOCKS: usize = 128;

fn main() {
    const CYCLES: usize = 48;
    const CHECKPOINT_INTERVAL: usize = 8;
    const CHECKPOINT_HOLD: usize = 3;
    const KEEP_LATEST: u32 = 2;

    let mut image = vec![0; SynFs::<BLOCKS>::volume_bytes()];
    SynFs::<BLOCKS>::format(&mut image).unwrap();
    let mut filesystem = Box::new(SynFs::<BLOCKS>::load(&image).unwrap());
    filesystem.create_directory("/data", true).unwrap();
    let mut purged = SynfsPurged::new();
    purged.add_rule("/data/long-run", KEEP_LATEST).unwrap();
    let mut checkpoint = None;

    for cycle in 0..CYCLES {
        let contents = [cycle as u8; 32];
        filesystem.write("/data/long-run", &contents).unwrap();
        if cycle % CHECKPOINT_INTERVAL == 0 {
            let info = filesystem.create_checkpoint().unwrap();
            checkpoint = Some((info, cycle as u8));
        }
        let report = purged.poll(&mut filesystem, 1).unwrap();
        let check = filesystem.check_consistency();
        println!("cycle={cycle} purged={} used={} check={check:?}", report.versions_purged, filesystem.used_blocks());
        if let Err(error) = check {
            return println!("FAILED at cycle {cycle}: {error:?}");
        }
        if cycle % CHECKPOINT_INTERVAL == CHECKPOINT_HOLD {
            let (info, expected) = checkpoint.take().unwrap();
            filesystem.flush(&mut image).unwrap();
            filesystem = Box::new(SynFs::<BLOCKS>::load(&image).unwrap());
            assert_eq!(filesystem.checkpoint_info(info.id), Ok(info));
            let snapshot = filesystem.checkpoint_snapshot(info.id, RmsMapHandle::from_capability((1 << 32) | 1).unwrap()).unwrap();
            let mut got = [0; 32];
            let mut copied = 0;
            snapshot.visit_file_pages("/data/long-run", |page| {
                got[copied..copied + page.bytes.len()].copy_from_slice(page.bytes);
                copied += page.bytes.len();
            }).unwrap();
            assert_eq!(got, [expected; 32]);
            filesystem.release_checkpoint(info.id).unwrap();
        }
    }
}
