//! 站點健康狀態的單元測試。

use super::*;

const NAMES: [&str; 4] = ["A", "B", "C", "D"];

/// 記錄一筆耗時 `elapsed_ms`、在 `now` 回來的結果。
fn record_ms(
    book: &mut HealthBook,
    name: &'static str,
    ok: bool,
    elapsed_ms: u64,
    now: Instant,
) -> Option<Transition> {
    let started_at = now
        .checked_sub(Duration::from_millis(elapsed_ms))
        .unwrap_or(now);
    book.record(name, ok, started_at, now)
}

fn fail_times(book: &mut HealthBook, name: &'static str, times: u32, now: Instant) {
    for _ in 0..times {
        record_ms(book, name, false, 100, now);
    }
}

/// 沒有任何紀錄時就是單純從 start 開始的輪詢，每站各一次。
#[test]
fn plan_without_history_is_plain_rotation() {
    let book = HealthBook::default();
    assert_eq!(book.plan(&NAMES, 2, Instant::now(), true), vec![2, 3, 0, 1]);
}

/// 全域紀錄：連續失敗達門檻那一次回報熔斷；每站每天第一次熔斷記 WARN，之後記 INFO。
///
/// 站點名稱只給這個測試用，避免與其他測試共用全域健康狀態；第二個站點預先登記為今天已熔斷過，
/// 走 INFO 分支（同一站再次熔斷要等冷卻結束）。
#[test]
fn record_site_outcome_reports_trips() {
    // 開訂閱者讓日誌欄位實際被求值。
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let started_at = Instant::now();
    for name in ["TestOutcomeFirstTrip", "TestOutcomeSecondTrip"] {
        if name == "TestOutcomeSecondTrip" {
            assert!(TRIPPED_TODAY.first_today(name, Local::now().date_naive()));
        }
        assert!(!record_site_outcome(name, true, started_at));
        for _ in 1..FAILURE_THRESHOLD {
            assert!(!record_site_outcome(name, false, started_at));
        }
        assert!(record_site_outcome(name, false, Instant::now()), "{name}");
    }
}

/// 連續失敗達門檻才熔斷；中間有一次成功就重新計算。
#[test]
fn site_trips_after_consecutive_failures() {
    let mut book = HealthBook::default();
    let now = Instant::now();

    fail_times(&mut book, "B", FAILURE_THRESHOLD - 1, now);
    record_ms(&mut book, "B", true, 100, now);
    fail_times(&mut book, "B", FAILURE_THRESHOLD - 1, now);
    assert_eq!(book.plan(&NAMES, 0, now, true), vec![0, 1, 2, 3]);

    assert_eq!(
        record_ms(&mut book, "B", false, 100, now),
        Some(Transition::Tripped(BASE_COOLDOWN))
    );
    assert_eq!(book.plan(&NAMES, 0, now, true), vec![0, 2, 3]);
}

/// 冷卻結束後第一次就失敗會立刻再熔斷且冷卻加倍；成功則恢復。
#[test]
fn half_open_failure_doubles_cooldown_and_success_recovers() {
    let mut book = HealthBook::default();
    let now = Instant::now();
    fail_times(&mut book, "C", FAILURE_THRESHOLD, now);

    let after_first = now + BASE_COOLDOWN + Duration::from_secs(1);
    assert_eq!(book.plan(&NAMES, 0, after_first, true), vec![0, 1, 2, 3]);
    assert_eq!(
        record_ms(&mut book, "C", false, 100, after_first),
        Some(Transition::Tripped(BASE_COOLDOWN * 2))
    );
    assert_eq!(book.plan(&NAMES, 0, after_first, true), vec![0, 1, 3]);

    let after_second = after_first + BASE_COOLDOWN * 2 + Duration::from_secs(1);
    assert_eq!(
        record_ms(&mut book, "C", true, 100, after_second),
        Some(Transition::Recovered)
    );
    assert_eq!(book.plan(&NAMES, 0, after_second, true), vec![0, 1, 2, 3]);
}

/// 冷卻時間逐次加倍並封頂。
#[test]
fn cooldown_is_capped() {
    assert_eq!(cooldown_for(1), Duration::from_secs(30));
    assert_eq!(cooldown_for(2), Duration::from_secs(60));
    assert_eq!(cooldown_for(4), Duration::from_secs(240));
    assert_eq!(cooldown_for(5), MAX_COOLDOWN);
    assert_eq!(cooldown_for(40), MAX_COOLDOWN);
}

/// 全部熔斷時仍回傳完整輪詢，不讓報價整個中斷。
#[test]
fn all_open_falls_back_to_full_rotation() {
    let mut book = HealthBook::default();
    let now = Instant::now();
    for name in NAMES {
        fail_times(&mut book, name, FAILURE_THRESHOLD, now);
    }
    assert_eq!(book.plan(&NAMES, 1, now, true), vec![1, 2, 3, 0]);
}

/// 明顯較慢的站點排到最後；探測輪次不降級；整體都快時不降級。
#[test]
fn slow_site_is_demoted_except_on_probe() {
    let mut book = HealthBook::default();
    let now = Instant::now();
    for name in ["A", "B", "C"] {
        record_ms(&mut book, name, true, 900, now);
    }
    record_ms(&mut book, "D", true, 2_000, now);
    record_ms(&mut book, "A", true, 950, now);

    assert_eq!(book.plan(&NAMES, 3, now, true), vec![0, 1, 2, 3]);
    assert_eq!(book.plan(&NAMES, 3, now, false), vec![3, 0, 1, 2]);

    let mut quick = HealthBook::default();
    for name in ["A", "B", "C"] {
        record_ms(&mut quick, name, true, 100, now);
    }
    record_ms(&mut quick, "D", true, 500, now);
    assert_eq!(quick.plan(&NAMES, 3, now, true), vec![3, 0, 1, 2]);
}

/// EWMA 會往新樣本靠近，慢站變快後不再降級。
#[test]
fn slow_site_recovers_when_latency_drops() {
    let mut book = HealthBook::default();
    let now = Instant::now();
    for name in ["A", "B", "C"] {
        record_ms(&mut book, name, true, 900, now);
    }
    record_ms(&mut book, "D", true, 3_000, now);
    assert!(book.is_slow(&NAMES, 3));

    for _ in 0..20 {
        record_ms(&mut book, "D", true, 900, now);
    }
    assert!(!book.is_slow(&NAMES, 3));
}

/// 熔斷前就送出的請求晚回來：成功不解除熔斷、失敗不再加倍冷卻，只更新耗時。
#[test]
fn results_started_before_trip_do_not_change_breaker() {
    let mut book = HealthBook::default();
    let sent = Instant::now();
    let tripped = sent + Duration::from_millis(500);
    fail_times(&mut book, "A", FAILURE_THRESHOLD, tripped);
    assert_eq!(book.plan(&NAMES, 0, tripped, true), vec![1, 2, 3]);

    let back = tripped + Duration::from_millis(20);
    assert_eq!(book.record("A", true, sent, back), None);
    assert_eq!(book.record("A", false, sent, back), None);
    assert_eq!(book.plan(&NAMES, 0, back, true), vec![1, 2, 3]);
    let ewma = book
        .sites
        .get("A")
        .and_then(|h| h.ewma_ms)
        .unwrap_or_default();
    assert!(ewma > 100.0, "晚回來的結果仍要計入耗時：{ewma}");

    // 冷卻結束後才送出的請求成功，才算真正恢復。
    let probe = tripped + BASE_COOLDOWN + Duration::from_secs(1);
    assert_eq!(
        book.record("A", true, probe, probe + Duration::from_millis(50)),
        Some(Transition::Recovered)
    );
    assert_eq!(book.plan(&NAMES, 0, probe, true), vec![0, 1, 2, 3]);
}

/// 全部熔斷時仍會照常輪詢；這時送出的請求成功就是有效的恢復證據。
#[test]
fn success_sent_while_open_recovers() {
    let mut book = HealthBook::default();
    let now = Instant::now();
    fail_times(&mut book, "B", FAILURE_THRESHOLD, now);
    let later = now + Duration::from_secs(1);
    assert_eq!(
        book.record("B", true, later, later + Duration::from_millis(30)),
        Some(Transition::Recovered)
    );
}
