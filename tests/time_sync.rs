use rs_ctrl_os::TimeSynchronizer;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn starts_not_synced() {
    let ts = TimeSynchronizer::new();
    assert!(!ts.is_synced());
}

#[test]
fn sync_sets_offset() {
    let ts = TimeSynchronizer::new();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    // Master is 500ms ahead of us (master_ts > local_ts)
    let master_ts = now_ms + 500;
    ts.update_from_master("master", master_ts);
    assert!(ts.is_synced());
}

#[test]
fn corrected_time_roughly_matches_master() {
    let ts = TimeSynchronizer::new();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    // Master clock is 200ms ahead of local.
    let master_ts = now_ms + 200;
    ts.update_from_master("master", master_ts);
    let corrected = ts.now_corrected_ms();
    // corrected time should be close to master (allowing a small window for execution)
    let diff = if corrected >= master_ts {
        corrected - master_ts
    } else {
        master_ts - corrected
    };
    assert!(diff < 100, "corrected={} master_ts={} diff={}", corrected, master_ts, diff);
}

#[test]
fn low_pass_filter_damps_jumps() {
    let ts = TimeSynchronizer::new();
    let base = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    // First sync: offset -1000 (master is 1000ms ahead)
    ts.update_from_master("m", base + 1000);
    let first = ts.now_corrected_ms();

    // Sudden jump: master now 5000ms ahead
    ts.update_from_master("m", base + 5000);
    let second = ts.now_corrected_ms();

    let diff = (second as i128 - first as i128).abs() as u64;
    // A raw jump would be ~4000ms; the low-pass filter limits it to ~400ms (10% weight)
    assert!(diff < 1000, "filter should damp the jump, got diff={}", diff);
}
