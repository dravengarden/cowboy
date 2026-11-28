//! Elastic home capacity (`crate::capacity`) against the fake host.

use std::os::fd::AsFd as _;
use std::os::unix::fs::symlink;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use super::{Fixture, code, fixture, fixture_with};
use crate::capacity::{growth_target, needs_growth, prune_agy_logs};
use crate::host::FsUsage;
use crate::protocol::{Code, Mode};
use crate::safefs;

const GIB: u64 = 1024 * 1024 * 1024;
const DAY: Duration = Duration::from_hours(24);

fn usage(size_bytes: u64, free_bytes: u64) -> FsUsage {
    FsUsage {
        size_bytes,
        free_bytes,
        inodes: 1000,
        free_inodes: 900,
    }
}

/// What ext4 reports for a 4 GiB image.
const REPORTED_4G: u64 = 4 * GIB / 100 * 97;

impl Fixture {
    fn image_len(&self) -> u64 {
        std::fs::metadata(self.daemon.paths(&self.key).image)
            .unwrap()
            .len()
    }

    /// The mounted home reports `usage` until it is grown.
    fn set_home_usage(&self, usage: FsUsage) {
        let home = safefs::open_root(&self.daemon.paths(&self.key).home).unwrap();
        let key = crate::fake::dir_key(home.as_fd()).unwrap();
        self.host.state().usage.insert(key, usage);
    }

    fn grows(&self) -> Vec<u64> {
        self.host.state().grown.clone()
    }
}

#[test]
fn a_home_grows_below_a_quarter_free_or_an_eighth_of_its_inodes() {
    assert!(!needs_growth(usage(4 * GIB, GIB + 1), GIB));
    assert!(needs_growth(usage(4 * GIB, GIB - 1), GIB));
    assert!(!needs_growth(usage(16 * GIB, 4 * GIB), GIB));
    assert!(needs_growth(usage(16 * GIB, 4 * GIB - 1), GIB));
    let inodes = FsUsage {
        free_inodes: 124,
        ..usage(4 * GIB, 3 * GIB)
    };
    assert!(needs_growth(inodes, GIB));
}

#[test]
fn growth_doubles_up_to_the_maximum() {
    assert_eq!(growth_target(4 * GIB, 32 * GIB), Some(8 * GIB));
    assert_eq!(growth_target(24 * GIB, 32 * GIB), Some(32 * GIB));
    assert_eq!(growth_target(32 * GIB, 32 * GIB), None);
    assert_eq!(growth_target(32 * GIB, 4 * GIB), None);
}

#[test]
fn the_sweep_grows_a_home_running_low() {
    let f = fixture();
    assert_eq!(f.image_len(), 4 * GIB);
    f.set_home_usage(usage(REPORTED_4G, GIB / 2));
    // A start above the minimum does not wait for growth.
    f.start(Mode::Probe, 0).unwrap();
    assert!(f.grows().is_empty());
    f.daemon.sweep_capacity(false);
    assert_eq!((f.grows(), f.image_len()), (vec![8 * GIB], 8 * GIB));
    // Now roomy: the next sweep does not grow again.
    f.daemon.sweep_capacity(false);
    assert_eq!(f.grows().len(), 1);
}

#[test]
fn a_start_into_a_nearly_full_home_grows_it_first() {
    let f = fixture();
    f.set_home_usage(usage(REPORTED_4G, 1024));
    f.start(Mode::Probe, 0).unwrap();
    assert_eq!((f.grows(), f.image_len()), (vec![8 * GIB], 8 * GIB));
}

#[test]
fn a_full_home_at_its_maximum_refuses_every_start_but_logout() {
    let f = fixture_with(|config| config.home_image_max_bytes = config.home_image_bytes);
    f.sign_in(1);
    f.set_home_usage(usage(REPORTED_4G, 1024));
    let refused = f.start(Mode::Runtime, 1).unwrap_err();
    assert_eq!(refused.code, Code::InsufficientSpace);
    assert!(
        refused.message.contains("home storage is exhausted")
            && refused.message.contains("maximum size"),
        "{}",
        refused.message
    );
    assert_eq!(code(f.start(Mode::Probe, 0)), Code::InsufficientSpace);
    assert!(f.grows().is_empty());
    assert_eq!(f.image_len(), 4 * GIB);
    // Signing out must stay possible on a full disk.
    f.start(Mode::Logout, 1).unwrap();
}

#[test]
fn growth_charges_the_space_every_image_may_still_claim() {
    let f = fixture();
    let reserve = f.config.free_space_reserve_bytes;
    // The sparse 4 GiB image may still claim about 4 GiB and growth adds 4.
    f.host.free.store(reserve + 6 * GIB, Ordering::Relaxed);
    f.set_home_usage(usage(REPORTED_4G, 1024));
    let refused = f.start(Mode::Probe, 0).unwrap_err();
    assert!(
        refused.message.contains("bytes free on the host"),
        "{}",
        refused.message
    );
    assert_eq!(f.image_len(), 4 * GIB);
    f.host.free.store(reserve + 9 * GIB, Ordering::Relaxed);
    f.start(Mode::Probe, 0).unwrap();
    assert_eq!(f.image_len(), 8 * GIB);
}

#[test]
fn a_failed_reconcile_is_retried_and_an_interrupted_grow_finished() {
    let f = fixture();
    f.set_home_usage(usage(REPORTED_4G, 1024));
    f.host.state().grow_fails = true;
    let refused = f.start(Mode::Probe, 0).unwrap_err();
    assert!(
        refused.message.contains("EXT4_IOC_RESIZE_FS failed"),
        "{}",
        refused.message
    );
    assert_eq!(f.image_len(), 4 * GIB);
    // The file was lengthened by something else and never resized: the
    // next check finishes it even without pressure.
    let image = f.daemon.paths(&f.key).image;
    std::fs::File::options()
        .write(true)
        .open(&image)
        .unwrap()
        .set_len(8 * GIB)
        .unwrap();
    f.set_home_usage(usage(REPORTED_4G, 3 * GIB));
    f.daemon.sweep_capacity(false);
    assert_eq!((f.grows(), f.image_len()), (vec![8 * GIB], 8 * GIB));
}

#[test]
fn an_unmounted_home_is_left_alone() {
    let f = fixture();
    f.set_home_usage(usage(REPORTED_4G, 1024));
    let home = f.daemon.paths(&f.key).home;
    f.host.state().mounted.remove(&home);
    f.daemon.sweep_capacity(false);
    assert!(f.grows().is_empty());
}

#[test]
fn logs_are_pruned_only_while_the_account_runs_no_unit() {
    let f = fixture();
    let log = f
        .daemon
        .paths(&f.key)
        .home
        .join(".gemini/antigravity-cli/log");
    std::fs::create_dir_all(&log).unwrap();
    let old = log.join("acp-server-old.log");
    let file = std::fs::File::create(&old).unwrap();
    file.set_modified(SystemTime::now() - 20 * DAY).unwrap();
    let unit = f.start(Mode::Probe, 0).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    f.daemon.sweep_capacity(true);
    assert!(old.exists());
    f.host.finish(&unit, Some(0));
    f.wait_for("the probe to settle", |meta| meta.units.is_empty());
    f.daemon.sweep_capacity(true);
    assert!(!old.exists());
}

#[test]
fn log_pruning_removes_only_old_regular_log_files() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join(".gemini/antigravity-cli/log");
    std::fs::create_dir_all(&log).unwrap();
    let now = SystemTime::now();
    let old = now - 20 * DAY;
    for (name, modified) in [("old.log", old), ("new.log", now)] {
        let file = std::fs::File::create(log.join(name)).unwrap();
        file.set_len(10).unwrap();
        file.set_modified(modified).unwrap();
    }
    let outside = temp.path().join("outside.log");
    std::fs::write(&outside, b"keep").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&outside)
        .unwrap()
        .set_modified(old)
        .unwrap();
    symlink(&outside, log.join("link.log")).unwrap();
    std::fs::create_dir(log.join("old-dir")).unwrap();
    let home = safefs::open_root(temp.path()).unwrap();
    let retention = 14 * DAY;
    assert_eq!(prune_agy_logs(&home, retention, now).unwrap(), (1, 10));
    assert!(!log.join("old.log").exists());
    assert!(log.join("new.log").exists() && log.join("old-dir").is_dir());
    assert!(std::fs::symlink_metadata(log.join("link.log")).is_ok() && outside.exists());
    // A home without agy state has nothing to prune.
    let empty = tempfile::tempdir().unwrap();
    let home = safefs::open_root(empty.path()).unwrap();
    assert_eq!(prune_agy_logs(&home, retention, now).unwrap(), (0, 0));
}
