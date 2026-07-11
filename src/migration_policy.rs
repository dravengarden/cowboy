//! Guard the startup migration contract: unbounded data rewrites are explicit
//! maintenance work, never daemon-startup work.

#[test]
fn new_migrations_do_not_rewrite_the_event_log() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    for entry in std::fs::read_dir(directory).expect("read migrations") {
        let entry = entry.expect("migration entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.as_str() <= "0012_compact_event_log.sql" {
            continue;
        }
        let sql = std::fs::read_to_string(entry.path())
            .expect("read migration")
            .to_uppercase();
        for forbidden in ["UPDATE EVENTS", "DELETE FROM EVENTS", "VACUUM FULL"] {
            assert!(
                !sql.contains(forbidden),
                "{name} contains startup-blocking operation {forbidden}; use expand/contract maintenance"
            );
        }
    }
}
