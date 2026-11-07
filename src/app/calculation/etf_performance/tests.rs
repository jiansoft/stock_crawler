//! ETF 含息報酬的測試。

use super::*;

/// 測試用：建立一段日線。
fn bars_from(start: NaiveDate, closes: &[f64]) -> Vec<PriceBar> {
    closes
        .iter()
        .enumerate()
        .map(|(i, close)| PriceBar {
            date: start + Days::new(i as u64),
            close: *close,
            high: *close,
            low: *close,
        })
        .collect()
}

#[test]
fn nav_returns_reinvest_cash_dividends_and_ignore_splits() {
    let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("日期合法");
    // 淨值 100 → 第 3 天除息 2 元後 99：配息以除息參考價 98 再投入，含息報酬 = 99 / 98 − 1。
    let mut navs = HashMap::new();
    navs.insert("0056".to_string(), bars_from(start, &[100.0, 100.0, 99.0]));
    let mut adjustments = HashMap::new();
    adjustments.insert(
        "0056".to_string(),
        vec![
            PriceAdjustment {
                date: start + Days::new(2),
                cash_dividend: 2.0,
                stock_dividend: 0.0,
                share_ratio: 1.0,
            },
            // 分割：淨值歷史已調整過，不可再套。
            PriceAdjustment {
                date: start + Days::new(1),
                cash_dividend: 0.0,
                stock_dividend: 0.0,
                share_ratio: 4.0,
            },
        ],
    );
    let mut performances = vec![
        EtfPerformance {
            stock_symbol: "0056".to_string(),
            ..Default::default()
        },
        EtfPerformance {
            stock_symbol: "00999".to_string(),
            ..Default::default()
        },
    ];
    attach_nav_returns(&mut performances, &navs, &adjustments);
    let nav = performances[0].nav_returns.as_ref().expect("有淨值");
    assert_eq!(nav.calc_date, start + Days::new(2));
    let one_week = nav.ret_1w;
    assert_eq!(one_week, None, "不足一週");
    assert!(performances[1].nav_returns.is_none());

    // 配息在分割之後，金額不換算。
    let adjusted = adjust_bars(&navs["0056"], &cash_only(&adjustments["0056"]));
    let total = (adjusted[2].close / adjusted[0].close - 1.0) * 100.0;
    let expected = (99.0 / 98.0 - 1.0) * 100.0;
    assert!(
        (total - expected).abs() < 1e-9,
        "含息報酬應為 {expected}%，實際 {total}"
    );
}

#[test]
fn cash_before_split_is_scaled_to_current_units() {
    let day = |d| NaiveDate::from_ymd_opt(2025, 6, d).expect("日期合法");
    let event = |date, cash, ratio| PriceAdjustment {
        date,
        cash_dividend: cash,
        stock_dividend: 0.0,
        share_ratio: ratio,
    };
    let cash = cash_only(&[
        event(day(1), 4.0, 1.0),
        event(day(18), 0.0, 4.0),
        event(day(25), 1.0, 1.0),
    ]);
    let amounts: Vec<(NaiveDate, f64, f64)> = cash
        .iter()
        .map(|e| (e.date, e.cash_dividend, e.share_ratio))
        .collect();
    assert_eq!(amounts, vec![(day(1), 1.0, 1.0), (day(25), 1.0, 1.0)]);
}

#[test]
fn compute_all_skips_symbols_without_bars() {
    let start = NaiveDate::from_ymd_opt(2026, 10, 1).expect("日期合法");
    let mut bars = HashMap::new();
    bars.insert("0050".to_string(), bars_from(start, &[100.0, 101.0, 102.0]));
    bars.insert("00999".to_string(), Vec::new());
    let symbols = ["0050".to_string(), "00999".to_string(), "00998".to_string()];
    let result = compute_all(&symbols, &bars, &[]);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].stock_symbol, "0050");
    assert_eq!(result[0].calc_date, Some(start + Days::new(2)));
}

/// 以臨時資料庫跑完整的重算流程（假 ETF、固定歷史日期）。
mod database {
    use rust_decimal::Decimal;

    use super::*;
    use crate::infra::database;

    const SYMBOL: &str = "79979EP";

    fn date(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(1999, month, day).expect("合法日期")
    }

    async fn cleanup() {
        let pool = database::get_connection();
        for sql in [
            "DELETE FROM etf_performance WHERE stock_symbol = $1",
            "DELETE FROM etf_tracking_error WHERE stock_symbol = $1",
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            sqlx::query(sql)
                .bind(SYMBOL)
                .execute(pool)
                .await
                .expect("清除測試資料");
        }
    }

    /// 上市 ETF（產業 9001）以 `today` 為期末算出績效並寫入；期末是最後一個有成交的日子。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn run_writes_performance_for_listed_etf() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 run_writes_performance_for_listed_etf：無資料庫連接");
            return;
        }
        cleanup().await;
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '測試 ETF', $1, 2, 9001)"#,
        )
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("插入假 ETF");
        // 1999-03-01（一）起 10 個交易日，收盤 20.0、20.1…20.9。
        let days = [1, 2, 3, 4, 5, 8, 9, 10, 11, 12];
        for (i, day) in days.into_iter().enumerate() {
            let close = Decimal::new(200 + i64::try_from(i).expect("小數字"), 1);
            sqlx::query(
                r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "HighestPrice", "LowestPrice", "TradingVolume")
                   VALUES ($1, $2, $3, $3, $3, 1000)"#,
            )
            .bind(date(3, day))
            .bind(SYMBOL)
            .bind(close)
            .execute(pool)
            .await
            .expect("插入日線");
        }

        let written = run(date(3, 15)).await;
        let calc_date: Option<NaiveDate> =
            sqlx::query_scalar("SELECT calc_date FROM etf_performance WHERE stock_symbol = $1")
                .bind(SYMBOL)
                .fetch_optional(pool)
                .await
                .expect("讀回績效");
        cleanup().await;

        assert!(written.expect("重算成功") >= 1);
        assert_eq!(calc_date, Some(date(3, 12)));
    }
}
