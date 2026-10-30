//! 全市場區間突破通知的單元與整合測試。

use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;
use crate::domain::quote::technical::StockTechnicalSignal;

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 30).expect("有效日期")
}

fn ranked(symbol: &str, name: &str, industry: &str) -> RankedSignal {
    RankedSignal {
        stock_symbol: symbol.to_owned(),
        name: name.to_owned(),
        industry: industry.to_owned(),
        trade_value: 1e9,
        description: "收盤 10.3 帶量突破 30 日整理區間高點 10".to_owned(),
    }
}

fn outcome(samples: i64) -> SignalOutcome {
    SignalOutcome {
        indicator: RANGE_BREAKOUT_INDICATOR.to_owned(),
        kind: "breakout".to_owned(),
        samples,
        avg_return: 1.23,
        win_rate: 0.52,
    }
}

/// 回傳固定排行與歷史表現、記錄查詢條件的假倉儲。
#[derive(Default)]
struct FakeQuery {
    signals: Vec<RankedSignal>,
    total: i64,
    fail: bool,
    outcomes: Vec<SignalOutcome>,
    outcomes_fail: bool,
    got: Mutex<Option<(NaiveDate, String, SignalKind, i64)>>,
}

#[async_trait]
impl RankedSignalQuery for FakeQuery {
    async fn fetch_ranked_signals(
        &self,
        date: NaiveDate,
        indicator: &str,
        kind: SignalKind,
        limit: i64,
    ) -> Result<(Vec<RankedSignal>, i64)> {
        *self.got.lock().expect("測試鎖") = Some((date, indicator.to_owned(), kind, limit));
        if self.fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok((self.signals.clone(), self.total))
    }
}

#[async_trait]
impl TechnicalSignalQuery for FakeQuery {
    async fn fetch_signals(
        &self,
        _date: NaiveDate,
        _stock_symbols: &[String],
    ) -> Result<Vec<StockTechnicalSignal>> {
        Ok(Vec::new())
    }

    async fn fetch_signal_outcomes(
        &self,
        _from: NaiveDate,
        _to: NaiveDate,
        _indicators: &[&str],
    ) -> Result<Vec<SignalOutcome>> {
        if self.outcomes_fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok(self.outcomes.clone())
    }
}

/// 標題附總檔數與列出檔數；產業、持股標記與勝率都有；結尾附回測結論與免責。
#[test]
fn build_message_lists_signals_with_holding_mark_and_outcome() {
    let signals = [
        ranked("2330", "台積電", "半導體業"),
        ranked("9999", "測試", ""),
    ];
    let holdings = BTreeSet::from(["2330".to_owned()]);

    let message = build_message(date(), &signals, 37, Some(&outcome(120)), &holdings);

    for want in [
        "區間突破︰2026\\-10\\-30（全市場 37 檔，依成交值列前 2）",
        "2330 台積電（半導體業）【持股】",
        "9999 測試\n",
        "• 收盤 10\\.3 帶量突破 30 日整理區間高點 10",
        "勝率 52%、平均還原報酬 \\+1\\.2%（120 次）",
        "單獨依區間突破選股明顯輸 0050",
    ] {
        assert!(message.contains(want), "缺少 {want:?}：{message}");
    }
    assert!(!message.contains("9999 測試【持股】"), "{message}");
}

/// 沒有歷史表現時只附回測結論。
#[test]
fn build_message_without_outcome() {
    let message = build_message(
        date(),
        &[ranked("2330", "台積電", "")],
        1,
        None,
        &BTreeSet::new(),
    );

    assert!(!message.contains("勝率"), "{message}");
    assert!(message.contains("不構成投資建議"), "{message}");
}

/// 查詢條件：當天、range_breakout／breakout、前 15 檔；樣本不足或假突破的歷史表現不附。
#[tokio::test]
async fn run_queries_top_signals_and_skips_small_samples() {
    let repo = FakeQuery {
        signals: vec![ranked("2330", "台積電", "半導體業")],
        total: 5,
        outcomes: vec![
            outcome(10),
            SignalOutcome {
                kind: "failed".to_owned(),
                ..outcome(500)
            },
        ],
        ..Default::default()
    };

    let message = run(&repo, date(), &BTreeSet::new())
        .await
        .expect("查詢成功")
        .expect("有訊號");

    assert_eq!(
        repo.got.lock().expect("測試鎖").take(),
        Some((
            date(),
            "range_breakout".to_owned(),
            SignalKind::Breakout,
            15
        ))
    );
    assert!(!message.contains("勝率"), "{message}");
}

/// 沒有訊號（含休市日）不發；查詢失敗回錯誤；歷史表現讀取失敗照常通知。
#[tokio::test]
async fn run_handles_empty_failure_and_outcome_failure() {
    let empty = FakeQuery::default();
    assert_eq!(
        run(&empty, date(), &BTreeSet::new())
            .await
            .expect("查詢成功"),
        None
    );

    let failing = FakeQuery {
        fail: true,
        ..Default::default()
    };
    assert!(run(&failing, date(), &BTreeSet::new()).await.is_err());

    let outcome_failing = FakeQuery {
        signals: vec![ranked("2330", "台積電", "")],
        total: 1,
        outcomes_fail: true,
        ..Default::default()
    };
    let message = run(&outcome_failing, date(), &BTreeSet::new())
        .await
        .expect("照常通知")
        .expect("有訊號");
    assert!(message.contains("2330 台積電"), "{message}");
}

/// 連資料庫：依成交值排序、取前 N 檔、總檔數為當天全部；其他來源、其他種類（假突破）與其他日期不列入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn fetch_ranked_signals_orders_by_trade_value() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 fetch_ranked_signals_orders_by_trade_value：無資料庫連接");
        return;
    }
    let day = NaiveDate::from_ymd_opt(1990, 3, 1).expect("有效日期");
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979B_'"#,
            "DELETE FROM daily_technical_signal WHERE stock_symbol LIKE '79979B_'",
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979B_'",
        ] {
            sqlx::query(sql).execute(pool).await.expect("清理測試資料");
        }
    };
    cleanup().await;
    // 79979BD 成交值最大但只有假突破，不列入突破排行。
    for (symbol, trade_value, kind) in [
        ("79979BA", 100, "breakout"),
        ("79979BB", 300, "breakout"),
        ("79979BC", 200, "breakout"),
        ("79979BD", 900, "failed"),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, 2, 24)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("寫入測試股票");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradeValue") VALUES ($1, $2, 10, $3)"#,
        )
        .bind(day)
        .bind(symbol)
        .bind(trade_value)
        .execute(pool)
        .await
        .expect("寫入測試日線");
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1, $2, 'range_breakout', $3, $2), ($1, $2, 'kd', 'overbought', 'kd')",
        )
        .bind(day)
        .bind(symbol)
        .bind(kind)
        .execute(pool)
        .await
        .expect("寫入測試訊號");
    }

    let (signals, total) = PgQuoteRepository::new()
        .fetch_ranked_signals(day, RANGE_BREAKOUT_INDICATOR, SignalKind::Breakout, 2)
        .await
        .expect("查詢成功");

    assert_eq!(total, 3);
    let symbols: Vec<&str> = signals.iter().map(|s| s.stock_symbol.as_str()).collect();
    assert_eq!(symbols, ["79979BB", "79979BC"]);
    assert_eq!(signals[0].industry, "半導體業");
    assert_eq!(signals[0].trade_value, 300.0);
    cleanup().await;
}

/// 排程入口連資料庫走完：讀持股、查當天訊號；當天沒有訊號時不發送也不失敗。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn execute_runs_against_database() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 execute_runs_against_database：無資料庫連接");
        return;
    }

    execute().await.expect("沒有訊號時照常結束");
}
