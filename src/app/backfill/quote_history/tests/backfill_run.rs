//! `execute_with` 的流程：逐代號逐月抓取、計數、單月失敗略過、倉儲失敗中止與 ACL 轉換。

use super::*;

/// 沒有代號就沒有工作：不打外部網站，也不碰倉儲。
#[tokio::test]
async fn execute_with_does_nothing_for_an_empty_symbol_list() {
    let fetcher = StubFetcher::default();
    let repository = CountingQuoteRepository::default();

    let summary = execute_with(
        &fetcher,
        &repository,
        &[],
        date(2015, 1, 1),
        date(2021, 12, 31),
    )
    .await
    .expect("空代號清單應成功");

    assert_eq!(summary, QuoteHistoryBackfillSummary::default());
    assert!(fetcher.requested().is_empty());
    assert_eq!(repository.insert_calls(), 0);
}

/// 區間反了要在送出任何請求前就失敗。
#[tokio::test]
async fn execute_with_rejects_a_reversed_range_before_any_request() {
    let fetcher = StubFetcher::default();
    let repository = CountingQuoteRepository::default();

    let err = execute_with(
        &fetcher,
        &repository,
        &["0050".to_string()],
        date(2022, 1, 1),
        date(2021, 12, 1),
    )
    .await
    .expect_err("反向區間應失敗");

    assert!(err.to_string().contains("起始月份"), "錯誤訊息：{err}");
    assert!(fetcher.requested().is_empty());
    assert_eq!(repository.insert_calls(), 0);
}

/// 逐一走過每個「代號 × 月份」，並把各項計數正確累加。
///
/// `start_paused` 讓 Tokio 的虛擬時鐘自動快轉，測試不必真的等每次請求
/// 之間的 1.2 秒間隔。
#[tokio::test(start_paused = true)]
async fn execute_with_walks_every_symbol_month_and_accumulates_counters() {
    let jan = date(2021, 1, 1);
    let feb = date(2021, 2, 1);
    // 0050 兩個月都有資料；0056 只有一月有，二月無成交。
    let fetcher = StubFetcher::default()
        .with_quotes("0050", jan, 3)
        .with_quotes("0050", feb, 2)
        .with_quotes("0056", jan, 1);
    let repository = CountingQuoteRepository::default();

    let summary = execute_with(
        &fetcher,
        &repository,
        &["0050".to_string(), "0056".to_string()],
        date(2021, 1, 15),
        date(2021, 2, 20),
    )
    .await
    .expect("回補應成功");

    assert_eq!(
        summary,
        QuoteHistoryBackfillSummary {
            months_requested: 4,
            months_with_data: 3,
            months_failed: 0,
            quotes_fetched: 6,
            rows_inserted: 6,
            rows_recalculated: 2,
        }
    );

    // 兩檔各兩個月，順序為「代號外層、月份內層」。
    assert_eq!(
        fetcher.requested(),
        vec![
            ("0050".to_string(), jan),
            ("0050".to_string(), feb),
            ("0056".to_string(), jan),
            ("0056".to_string(), feb),
        ]
    );
    // 無資料的月份不該產生寫入呼叫。
    assert_eq!(repository.insert_calls(), 3);
    assert_eq!(repository.inserted_len(), 6);
    // 有新增資料的兩檔，從區間第一個月起重算均線。
    assert_eq!(
        repository.recalculated(),
        vec![(vec!["0050".to_string(), "0056".to_string()], jan)]
    );
}

/// 單一月份抓取失敗只記錄並繼續，不讓整批回補中止。
///
/// 這是「補七年可以直接重跑」的前提：中間某個月暫時性失敗時，
/// 其餘月份仍須完成，否則每次重跑都會卡在同一個地方。
#[tokio::test(start_paused = true)]
async fn execute_with_skips_a_failed_month_and_keeps_going() {
    let jan = date(2021, 1, 1);
    let feb = date(2021, 2, 1);
    let mar = date(2021, 3, 1);
    let fetcher = StubFetcher::default()
        .with_quotes("0050", jan, 2)
        .with_failure("0050", feb)
        .with_quotes("0050", mar, 1);
    let repository = CountingQuoteRepository::default();

    let summary = execute_with(
        &fetcher,
        &repository,
        &["0050".to_string()],
        jan,
        date(2021, 3, 31),
    )
    .await
    .expect("單月失敗不應讓整批失敗");

    assert_eq!(
        summary,
        QuoteHistoryBackfillSummary {
            months_requested: 3,
            months_with_data: 2,
            months_failed: 1,
            quotes_fetched: 3,
            rows_inserted: 3,
            rows_recalculated: 1,
        }
    );
    // 失敗的二月之後，三月仍然被請求。
    assert_eq!(fetcher.requested().len(), 3);
    assert_eq!(repository.inserted_len(), 3);
}

/// 寫入失敗與抓取失敗不同：資料庫出問題必須整批中止並帶出代號與月份。
#[tokio::test(start_paused = true)]
async fn execute_with_aborts_when_the_repository_fails() {
    let jan = date(2021, 1, 1);
    let fetcher = StubFetcher::default()
        .with_quotes("0050", jan, 2)
        .with_quotes("0050", date(2021, 2, 1), 2);
    let repository = CountingQuoteRepository::failing();

    let err = execute_with(
        &fetcher,
        &repository,
        &["0050".to_string()],
        jan,
        date(2021, 2, 28),
    )
    .await
    .expect_err("倉儲失敗應上拋");

    let message = format!("{err:#}");
    assert!(
        message.contains("0050") && message.contains("2021-01"),
        "錯誤應指出是哪一檔的哪個月：{message}"
    );
    // 第一個月就中止，第二個月不該再被請求。
    assert_eq!(fetcher.requested().len(), 1);
}

/// 來源 DTO 會經過 ACL 轉成領域實體後才寫入，代號與日期不可在轉換中遺失。
#[tokio::test(start_paused = true)]
async fn execute_with_maps_dtos_through_the_acl_before_writing() {
    let jan = date(2021, 1, 1);
    let fetcher = StubFetcher::default().with_quotes("0050", jan, 2);
    let repository = CountingQuoteRepository::default();

    execute_with(&fetcher, &repository, &["0050".to_string()], jan, jan)
        .await
        .expect("回補應成功");

    assert_eq!(
        repository.inserted_keys(),
        vec![
            ("0050".to_string(), date(2021, 1, 1)),
            ("0050".to_string(), date(2021, 1, 2)),
        ]
    );
}
