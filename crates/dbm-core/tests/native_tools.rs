use dbm_core::native::{select_runner, NativePgToolPaths, RunnerKind};
use std::path::PathBuf;

#[test]
fn native_tools_are_selected_when_all_required_binaries_exist() {
    let root = std::env::temp_dir().join(format!("dbm-native-tools-test-{}", std::process::id()));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for tool in ["pg_dump.exe", "pg_restore.exe", "psql.exe"] {
        std::fs::write(bin.join(tool), b"placeholder").unwrap();
    }
    let paths = NativePgToolPaths::from_bin_dir(bin.clone());
    assert_eq!(select_runner(&paths, 17), RunnerKind::Native);
}

#[test]
fn docker_is_selected_only_when_native_bundle_is_unavailable() {
    let paths = NativePgToolPaths::from_bin_dir(PathBuf::from("does-not-exist"));
    assert_eq!(select_runner(&paths, 17), RunnerKind::Docker);
}

#[test]
fn native_tool_paths_are_never_derived_from_current_directory() {
    let paths = NativePgToolPaths::discover_from_executable(PathBuf::from(
        r"C:\Program Files\Database Backup Manager\DatabaseBackupManager.exe",
    ));
    assert_eq!(
        paths.bin_dir,
        PathBuf::from(r"C:\Program Files\Database Backup Manager\resources\postgresql\17\bin")
    );
}
