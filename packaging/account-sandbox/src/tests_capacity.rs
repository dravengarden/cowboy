//! Elastic home capacity (`crate::capacity`) against the fake host.

use std::os::fd::AsFd as _;
use std::os::unix::fs::symlink;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use super::{Fixture, code, fixture, fixture_with};
use crate::capacity::{growth_target, needs_growth, prune_agy_logs};
use crate::config::Config;
use crate::host::FsUsage;
use crate::protocol::{Code, Mode};
use crate::safefs;

const GIB: u64 = 1024 * 1024 * 1024;

fn usage(size_bytes: u64, free_bytes: u64) -> FsUsage {
    FsUsage {
        size_bytes,
        free_bytes,
        inodes: 1000,
        free_inodes: 900,
    }
}

impl Fixture {
    fn image_len(&self) -> u64 {
        std::fs::metadata(self.daemon.paths(&self.key).image)
            .unwrap()
            .len()
    }

    /// The mounted home reports `usage` until it is grown.
    fn set_home_usage(&self, usage: FsUsage) {
        let home = self.daemon.paths(&self.key).home;
        self.host.state().usage.insert(home, usage);
    }

    fn grows(&self) -> usize {
        self.host.state().grown.len()
    }
}

#[test]
fn a_home_grows_below_a_quarter_free_or_an_eighth_of_its_inodes() {
    let config = Config::default();
    assert!(!needs_growth(usage(4 * GIB, GIB + 1), &config));
    assert!(needs_growth(usage(4 * GIB, GIB - 1), &config));
    assert!(!needs_growth(usage(16 * GIB, 4 * GIB), &config));
    assert!(needs_growth(usage(16 * GIB, 4 * GIB - 1), &config));
    let inodes = FsUsage {
        free_inodes: 124,
        ..usage(4 * GIB, 3 * GIB)
    };
    assert!(needs_growth(inodes, &config));
}

#[test]
fn growth_doubles_up_to_the_maximum_and_finishes_an_interrupted_grow() {
    let config = Config::default();
    // ext4 reports its size without a few percent of metadata.
    let reported = |file: u64| file / 100 * 97;
    assert_eq!(
        growth_target(usage(reported(4 * GIB), 0), 4 * GIB, &config),
        Some(8 * GIB)
    );
    assert_eq!(
        growth_target(usage(reported(24 * GIB), 0), 24 * GIB, &config),
        Some(32 * GIB)
    );
    assert_eq!(
        growth_target(usage(reported(32 * GIB), 0), 32 * GIB, &config),
        None
    );
    // The file was lengthened but the filesystem never resized.
    assert_eq!(
        growth_target(usage(reported(4 * GIB), 0), 8 * GIB, &config),
        Some(8 * GIB)
    );
}

#[test]
fn a_start_grows_a_home_running_low_before_its_unit_runs() {
    let f = fixture();
    assert_eq!(f.image_len(), 4 * GIB);
    f.set_home_usage(usage(4 * GIB / 100 * 97, GIB / 2));
    f.start(Mode::Probe, 0).unwrap();
    assert_eq!(f.image_len(), 8 * GIB);
    assert_eq!(f.grows(), 1);
    // Now roomy: the next start does not grow again.
    f.start(Mode::Probe, 0).unwrap();
    assert_eq!(f.grows(), 1);
}

#[test]
fn a_full_home_at_its_maximum_refuses_every_start_but_logout() {
    let f = fixture_with(|config| config.home_image_max_bytes = config.home_image_bytes);
    f.sign_in(1);
    f.set_home_usage(usage(4 * GIB / 100 * 97, 1024));
    let refused = f.start(Mode::Runtime, 1).unwrap_err();
    assert_eq!(refused.code, Code::InsufficientSpace);
    assert!(
        refused.message.contains("home storage is exhausted")
            && refused.message.contains("maximum size"),
        "{}",
        refused.message
    );
    assert_eq!(code(f.start(Mode::Probe, 0)), Code::InsufficientSpace);
    assert_eq!(f.grows(), 0);
    assert_eq!(f.image_len(), 4 * GIB);
    // Signing out must stay possible on a full disk.
    f.start(Mode::Logout, 1).unwrap();
}

#[test]
fn growth_keeps_the_host_reserve_and_a_roomy_enough_home_still_starts() {
    let f = fixture();
    let reserve = f.config.free_space_reserve_bytes;
    // Enough for starts, not for 4 GiB of growth beyond the reserve.
    f.host.free.store(reserve + GIB, Ordering::Relaxed);
    f.set_home_usage(usage(4 * GIB / 100 * 97, GIB / 2));
    f.start(Mode::Probe, 0).unwrap();
    assert_eq!((f.grows(), f.image_len()), (0, 4 * GIB));
    f.set_home_usage(usage(4 * GIB / 100 * 97, 1024));
    let refused = f.start(Mode::Probe, 0).unwrap_err();
    assert!(
        refused.message.contains("beyond the reserve"),
        "{}",
        refused.message
    );
    f.host.free.store(reserve + 5 * GIB, Ordering::Relaxed);
    f.start(Mode::Probe, 0).unwrap();
    assert_eq!((f.grows(), f.image_len()), (1, 8 * GIB));
}

#[test]
fn a_failed_resize_is_finished_by_the_next_check() {
    let f = fixture();
    f.set_home_usage(usage(4 * GIB / 100 * 97, 1024));
    f.host.state().grow_fails = true;
    let refused = f.start(Mode::Probe, 0).unwrap_err();
    assert!(
        refused.message.contains("resize2fs failed"),
        "{}",
        refused.message
    );
    // The file was lengthened before the resize failed.
    assert_eq!(f.image_len(), 8 * GIB);
    f.start(Mode::Probe, 0).unwrap();
    assert_eq!((f.grows(), f.image_len()), (1, 8 * GIB));
}

#[test]
fn the_sweep_grows_mounted_homes_while_their_units_run() {
    let f = fixture();
    f.set_home_usage(usage(4 * GIB / 100 * 97, GIB / 2));
    f.daemon.sweep_capacity(false);
    assert_eq!((f.grows(), f.image_len()), (1, 8 * GIB));
    // An unmounted home is left alone.
    let home = f.daemon.paths(&f.key).home;
    f.host.state().mounted.remove(&home);
    f.set_home_usage(usage(8 * GIB / 100 * 97, 1024));
    f.daemon.sweep_capacity(false);
    assert_eq!(f.grows(), 1);
}

#[test]
fn log_pruning_removes_only_old_regular_log_files() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join(".gemini/antigravity-cli/log");
    std::fs::create_dir_all(&log).unwrap();
    let now = SystemTime::now();
    let old = now - Duration::from_hours(20 * 24);
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
    let retention = Duration::from_hours(14 * 24);
    assert_eq!(prune_agy_logs(&home, retention, now).unwrap(), (1, 10));
    assert!(!log.join("old.log").exists());
    assert!(log.join("new.log").exists() && log.join("old-dir").is_dir());
    assert!(std::fs::symlink_metadata(log.join("link.log")).is_ok() && outside.exists());
    // A home without agy state has nothing to prune.
    let empty = tempfile::tempdir().unwrap();
    let home = safefs::open_root(empty.path()).unwrap();
    assert_eq!(prune_agy_logs(&home, retention, now).unwrap(), (0, 0));
    let _ = home.as_fd();
}
