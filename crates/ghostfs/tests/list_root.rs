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
