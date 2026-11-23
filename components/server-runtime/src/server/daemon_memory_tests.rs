use super::parse_proc_status_memory;

#[test]
fn proc_status_reports_current_and_peak_bytes() {
    let memory = parse_proc_status_memory(
        "Name:\tcowboy\nVmPeak:\t 999999 kB\nVmHWM:\t 258180 kB\nVmRSS:\t  72372 kB\n",
    );
    assert_eq!(memory.rss_bytes, 72_372 * 1024);
    assert_eq!(memory.rss_peak_bytes, 258_180 * 1024);
}
