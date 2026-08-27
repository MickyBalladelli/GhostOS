use ghostos_ghostfs::{DirectoryEntry, FileType, SynFs};

#[test]
fn list_root_shows_immediate_child_after_nested_create() {
    let mut filesystem = SynFs::<32>::new();
    filesystem
        .create_directory("/system/security", true)
        .expect("directory");
    filesystem
        .write("/system/security/authorization", b"x")
        .expect("write");

    let mut entries = [DirectoryEntry::EMPTY; 16];
    let count = filesystem
        .list_directory("/", &mut entries)
        .expect("list root");
    assert_eq!(count, 1, "root listing was {count}");
    assert_eq!(entries[0].name.as_str(), "system");
    assert_eq!(entries[0].file_type, FileType::Directory);

    let count = filesystem
        .list_directory("/system", &mut entries)
        .expect("list system");
    assert_eq!(count, 1);
    assert_eq!(entries[0].name.as_str(), "security");
}

#[test]
fn list_directory_does_not_rescan_the_volume_per_entry() {
    let source = include_str!("../src/lib.rs");
    assert!(
        !source.contains("link_count: self.link_count_at(root, record.object_id)"),
        "LIST must use the cached record link count, not a nested volume walk"
    );
    assert!(source.contains("fn for_each_latest_record_at"));
    assert!(source.contains("link_count: record.link_count.max(1)"));
}

#[test]
fn list_directory_uses_cached_link_counts_for_hard_links() {
    let mut filesystem = SynFs::<32>::new();
    filesystem.create_directory("/data", true).expect("directory");
    filesystem.write("/data/source", b"x").expect("write");
    filesystem
        .link("/data/source", "/data/alias")
        .expect("hard link");

    let mut entries = [DirectoryEntry::EMPTY; 16];
    let count = filesystem
        .list_directory("/data", &mut entries)
        .expect("list data");
    assert_eq!(count, 2);
    let mut seen_source = false;
    let mut seen_alias = false;
    for entry in entries.iter().take(count) {
        if entry.name.as_str() == "source" {
            assert_eq!(
                entry.link_count, 1,
                "LIST must use the cached field, not a live recount"
            );
            seen_source = true;
        }
        if entry.name.as_str() == "alias" {
            assert_eq!(entry.link_count, 2);
            seen_alias = true;
        }
    }
    assert!(seen_source && seen_alias);
    assert_eq!(filesystem.lookup("/data/source").unwrap().link_count, 2);
    assert_eq!(filesystem.lookup("/data/alias").unwrap().link_count, 2);
}
