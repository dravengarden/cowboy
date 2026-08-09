//! Guard the startup migration contract: unbounded data rewrites are explicit
//! maintenance work, never daemon-startup work.

#![warn(clippy::pedantic)]

use sha2::Digest as _;

/// Every migration that has reached a shared database is immutable. `SQLx`
/// enforces this at runtime; pinning the source digests here moves the same
/// failure into `cargo test`, before a Cowboy restart can take the UI down.
const PUBLISHED_MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_init.sql",
        "78a7b726befc410983f29a795ef3e280303022ec61ddb9582da6f7bfe25a9fe9",
    ),
    (
        "0002_agent_session_id.sql",
        "e1eeade77f4a900801f1d9ed937d3c691ccdbee1773046e2b96638693149c8f8",
    ),
    (
        "0003_pending.sql",
        "3310c2ba3f641dfe5872b0a1a457dd07426185e7ad37eba14bea2ac9e92d0cac",
    ),
    (
        "0004_session_position.sql",
        "e618956295fcbad126f5393dd96f6ff67e6d8bcca6f7f888d145a94ca1396009",
    ),
    (
        "0005_session_soft_delete.sql",
        "1ac9b195672655a9e1ba1c2a5bc46878d46cd767f8cdefa4067a3599d6e925d0",
    ),
    (
        "0006_auto_resume.sql",
        "9294c757e16de8293b0aec35d6f27eaebe8061062ab5cdf7da1cebceae59a4af",
    ),
    (
        "0007_inference.sql",
        "d645861f033be2b513882eb2763334248963ce6ee50ca00e68234dad876c33a3",
    ),
    (
        "0008_turn_verdict.sql",
        "7606eb18ac47b87c5511bd45657933da0d1cdcb5f5be7862d6c9693a5275abc8",
    ),
    (
        "0009_judge_runs.sql",
        "ab5bb69d9edce0c3b148894b6b3a1261e531cd7251175347f1516714e80129a1",
    ),
    (
        "0010_system_session.sql",
        "155000a02401c371d3bd02cca1362f7d6de93f92f9a665ba9523ed4d3f9ab9de",
    ),
    (
        "0011_scheduled_wakeups.sql",
        "a6ef2f7973ce3b69c3435f146c98bf1419fd667d21d81caacfa260d8d8a2b5ce",
    ),
    (
        "0012_compact_event_log.sql",
        "d8ac844edcc802728809f81fa309ee62317e4509f7c59ecd43b1c73e36cd6f18",
    ),
    (
        "0013_drop_inference.sql",
        "f0674ab068e55ba6aa5ccf1e7d0ef5f3ecbf28149b16d855fb5f3e8e8f009125",
    ),
    (
        "0014_provider_actions.sql",
        "b6f32dc1f36c05b9d8461a0ca2bcb08936eb0173540e9ed3343d211d65c1ceea",
    ),
    (
        "0015_provider_action_logs.sql",
        "47dea213814a7ad695f0dad94189342b3cbbee526cbafc611498144acf3d3780",
    ),
    (
        "0016_mobile_review_state.sql",
        "9f20a15c4b9a000209e1af070aadc86f7874c753bba86324292670b71ac8c7a8",
    ),
    (
        "0017_machines.sql",
        "334060e9171db9c5eda5188982ab0777d6dd8ea95d66e1582881ec762904c3d3",
    ),
    (
        "0018_machine_enrollment.sql",
        "b295ce695b06ef213144691956fd5d5e141a67e107426788f41ef26fcffcf491",
    ),
    (
        "0019_unify_local_machine.sql",
        "3b9ae1a0569d12b880165c9a8f1b9311be09e43cc8c638a6b9ea1b55c18bc993",
    ),
    (
        "0020_machine_reconnect_grace.sql",
        "f4993e561935bdd74ef706e84ab81ccbb809b754f6ec0c3f45c9abc8ae93bdc8",
    ),
    (
        "0021_runtime_incidents.sql",
        "6b80aa0284f9a113ac4ebde0f72e43565677aec5b0d66e2378fe207ff797db58",
    ),
    (
        "0022_provider_usage_ledger.sql",
        "3f5bc917df416351be2dc46fab91fe4e4e66510ad49c66f71fc88fc720c17d22",
    ),
    (
        "0023_session_workspace_identity.sql",
        "0ea62043bcb20ee9e994b4213739ad82243ef2fb1661b92dbeb75cac2610b6b2",
    ),
    (
        "0024_provider_usage_telemetry_v2.sql",
        "ea29fe4759ab4b77534a021eb22447c56be580d63f66822d527f44c5476448df",
    ),
    (
        "0025_provider_usage_telemetry_v3.sql",
        "d0011492f29a1159fa2dda5e8a052d0c3a4b8516b5e40290be8681b9bf6f426d",
    ),
    (
        "0026_session_config.sql",
        "64c6080585e93cfaec59156882786149cb85e0a69d30bdd2b4feab103ab78a09",
    ),
    (
        "0027_deepseek_cache_keepalive.sql",
        "f07d7821ab0c58fef0d97204500a092aef64b910ca67dd4a7236be7603b3dd48",
    ),
];

#[test]
fn published_migrations_are_immutable() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut seen = 0;
    for entry in std::fs::read_dir(directory).expect("read migrations") {
        let entry = entry.expect("migration entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().extension() != Some(std::ffi::OsStr::new("sql")) {
            continue;
        }
        let expected = PUBLISHED_MIGRATIONS
            .iter()
            .find_map(|(candidate, digest)| (*candidate == name).then_some(*digest))
            .unwrap_or_else(|| {
                panic!("{name} has no published checksum; register it before deploy")
            });
        let bytes = std::fs::read(entry.path()).expect("read migration");
        let actual = format!("{:x}", sha2::Sha256::digest(bytes));
        assert_eq!(actual, expected, "published migration {name} was modified");
        seen += 1;
    }
    assert_eq!(seen, PUBLISHED_MIGRATIONS.len());
}

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
