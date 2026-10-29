//! 區間突破的每日計算與歷史回補測試。

use super::*;
use crate::app::calculation::technical_signal::range_breakout::backfill_with;

/// 30 根整理（高 10、低 9、收 9.5、量 1000）之後第 `breakout_at` 根帶量突破（收 10.3、量 1600），
/// 其後回到整理；共 `len` 根，日期 `day(0)` 起連續。
fn breakout_series(len: usize, breakout_at: usize) -> (Vec<PriceBar>, Vec<VolumeBar>) {
    let bars: Vec<PriceBar> = (0..len)
        .map(|i| {
            let (close, high) = if i == breakout_at {
                (10.3, 10.5)
            } else {
                (9.5, 10.0)
            };
            PriceBar {
                date: day(i as u64),
                close,
                high,
                low: 9.0,
            }
        })
        .collect();
    let volumes = bars
        .iter()
        .enumerate()
        .map(|(i, bar)| VolumeBar {
            date: bar.date,
            volume: if i == breakout_at { 1600.0 } else { 1000.0 },
        })
        .collect();
    (bars, volumes)
}

/// 每日計算串上區間突破：讀最近 21 根成交量，最後一日帶量突破就寫入 `range_breakout`。
#[tokio::test]
async fn compute_and_save_includes_range_breakout() {
    let (bars, volumes) = breakout_series(31, 30);
    let repo = FakeRepo {
        symbols: vec!["A".into()],
        bars: HashMap::from([("A".to_owned(), bars)]),
        volumes: HashMap::from([("A".to_owned(), volumes)]),
        ..Default::default()
    };

    compute_and_save(&repo, day(30)).await.expect("計算成功");

    let (_, saved) = repo.saved.lock().expect("測試鎖").take().expect("有寫入");
    assert!(
        saved.iter().any(
            |s| s.signal.indicator == "range_breakout" && s.signal.kind == SignalKind::Breakout
        ),
        "{saved:?}"
    );
}

/// 回補只寫起訖期間內出現突破的日子，並以來源與起訖覆蓋。
#[tokio::test]
async fn backfill_writes_breakouts_within_range_only() {
    let (mut bars, mut volumes) = breakout_series(70, 32);
    // 第 32 根的突破在起日之前，不寫；第 66 根再突破一次，在期間內。B 沒有成交量，不判斷。
    bars[66].close = 10.3;
    bars[66].high = 10.5;
    volumes[66].volume = 1600.0;
    let repo = FakeRepo {
        symbols: vec!["A".into(), "B".into()],
        bars: HashMap::from([
            ("A".to_owned(), bars.clone()),
            ("B".to_owned(), bars[..40].to_vec()),
        ]),
        volumes: HashMap::from([("A".to_owned(), volumes)]),
        ..Default::default()
    };

    let count = backfill_with(&repo, day(40), day(69))
        .await
        .expect("回補成功");

    let (indicator, from, to, signals) = repo
        .replaced
        .lock()
        .expect("測試鎖")
        .take()
        .expect("有寫入");
    assert_eq!(
        (indicator.as_str(), from, to),
        ("range_breakout", day(40), day(69))
    );
    assert_eq!(count, 1);
    assert_eq!(signals.len(), 1);
    assert_eq!(
        (signals[0].stock_symbol.as_str(), signals[0].date),
        ("A", day(66))
    );
    assert!(
        repo.limits
            .lock()
            .expect("測試鎖")
            .iter()
            .all(|&limit| limit == 30 + 31),
        "讀取根數要涵蓋起訖天數加 31 根"
    );
}

/// 起日晚於迄日、或任一檔讀取失敗時不寫入。
#[tokio::test]
async fn backfill_rejects_bad_range_and_keeps_table_on_fetch_failure() {
    let repo = FakeRepo {
        symbols: vec!["MISSING".into()],
        ..Default::default()
    };

    assert!(backfill_with(&repo, day(2), day(1)).await.is_err());
    let error = backfill_with(&repo, day(1), day(2))
        .await
        .expect_err("讀取失敗應回錯誤");
    assert!(
        format!("{error:#}").contains("讀取 MISSING 日線失敗"),
        "{error:#}"
    );
    assert!(repo.replaced.lock().expect("測試鎖").is_none());
}

/// 連資料庫的回補流程：成交量查詢、期間股票與只覆蓋 `range_breakout` 來源。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn backfill_range_breakout_writes_table() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 backfill_range_breakout_writes_table：無資料庫連接");
        return;
    }
    let symbol = "79979RB";
    let first = NaiveDate::from_ymd_opt(1990, 1, 1).expect("有效日期");
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
            "DELETE FROM stocks WHERE stock_symbol = $1",
            "DELETE FROM daily_technical_signal WHERE stock_symbol = $1",
        ] {
            sqlx::query(sql)
                .bind(symbol)
                .execute(pool)
                .await
                .expect("清理測試資料");
        }
    };
    cleanup().await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
           VALUES ($1, $1, $1, 2, 0)"#,
    )
    .bind(symbol)
    .execute(pool)
    .await
    .expect("寫入測試股票");
    // 第 30 根（1990-01-31）帶量突破，其餘整理。
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "HighestPrice", "LowestPrice", "TradingVolume")
           SELECT $2::date + n, $1,
                  CASE WHEN n = 30 THEN 10.3 ELSE 9.5 END,
                  CASE WHEN n = 30 THEN 10.5 ELSE 10 END,
                  9,
                  CASE WHEN n = 30 THEN 1600 ELSE 1000 END
           FROM generate_series(0, 40) AS n"#,
    )
    .bind(symbol)
    .bind(first)
    .execute(pool)
    .await
    .expect("寫入測試日線");
    let breakout_day = first + chrono::Days::new(30);
    // 其他來源的訊號不能被回補刪掉。
    sqlx::query(
        "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
         VALUES ($1, $2, 'kd', 'overbought', '測試')",
    )
    .bind(breakout_day)
    .bind(symbol)
    .execute(pool)
    .await
    .expect("寫入其他來源訊號");

    let from = first + chrono::Days::new(20);
    let to = first + chrono::Days::new(40);
    backfill_range_breakout(from, to).await.expect("回補成功");

    let rows: Vec<(NaiveDate, String, String)> = sqlx::query_as(
        "SELECT date, indicator, kind FROM daily_technical_signal WHERE stock_symbol = $1 ORDER BY indicator",
    )
    .bind(symbol)
    .fetch_all(pool)
    .await
    .expect("讀回訊號");
    assert_eq!(
        rows,
        vec![
            (breakout_day, "kd".to_owned(), "overbought".to_owned()),
            (
                breakout_day,
                "range_breakout".to_owned(),
                "breakout".to_owned()
            ),
        ]
    );
    cleanup().await;
}
