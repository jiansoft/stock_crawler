//! Yahoo 股利政策頁的重試抓取的測試；連外的測試標 #[ignore]。

use super::*;

/// 重試延遲必須是秒級且有上限，總等待時間不可失控。
#[test]
fn retry_delays_are_bounded() {
    let delays: Vec<Duration> = retry_delays().collect();

    assert_eq!(
        delays,
        vec![
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
        ]
    );
    assert!(delays.iter().all(|delay| *delay <= MAX_RETRY_DELAY));
}

/// 已下市的 4987 回 404，必須立即失敗而不是進入重試。
#[tokio::test]
#[ignore]
async fn visit_with_retry_does_not_retry_page_not_found() {
    dotenvy::dotenv().ok();

    let started = std::time::Instant::now();
    let err = visit_with_retry("4987").await.expect_err("4987 應回 404");

    assert!(yahoo::dividend::is_page_not_found_error(&err));
    assert!(started.elapsed() < Duration::from_millis(400));
}
