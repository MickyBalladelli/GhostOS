// Inventory: coverage_59_5.rs (legacy roadmap section 59).
use ghostos_shell::filesystem::{split_version_selector, Path, ShellSession};
use ghostos_status::Status;

#[test]
fn shell_filesystem_resolution_preserves_versions_and_blocks_traversal() {
    let session = ShellSession::new();
    assert_eq!(session.resolve(Some("/data/./file;0")).unwrap().as_str(), "/data/file;0");
    assert_eq!(session.resolve(Some("/data/../file;4")).unwrap().as_str(), "/file;4");
    assert_eq!(session.resolve(Some("../../escape")), Err(Status::INVALID_PATH));
    assert_eq!(session.resolve(Some("/data//file")), Err(Status::INVALID_PATH));
    assert_eq!(split_version_selector("/data/file;7"), Ok(("/data/file", Some(7))));
    assert_eq!(split_version_selector("/data/file;latest"), Err(Status::INVALID_ARGUMENT));
    assert_eq!(Path::new("/"), Ok(Path::ROOT));
}
