use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;
use crate::domain::quote::technical::SignalKind;

fn day(offset: u64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).expect("有效日期") + chrono::Days::new(offset)
}

/// 連續 `len` 日、收盤依序為 `closes` 的日線，最後一日為 `day(len - 1)`。
fn bars(closes: &[f64]) -> Vec<PriceBar> {
    closes
        .iter()
        .enumerate()
        .map(|(i, &close)| PriceBar {
            date: day(i as u64),
            close,
            high: close,
            low: close,
        })
        .collect()
}

/// `replace_indicator_signals` 的引數：來源、起日、迄日與訊號。
type Replaced = (String, NaiveDate, NaiveDate, Vec<StockTechnicalSignal>);

/// 記錄寫入內容的假倉儲；`bars` 沒有的代號讀取時回錯誤，`adjustments_fail` 時讀事件回錯誤。
#[derive(Default)]
struct FakeRepo {
    symbols: Vec<String>,
    bars: HashMap<String, Vec<PriceBar>>,
    /// 沒有的代號讀成交量回空清單（不判斷區間突破）。
    volumes: HashMap<String, Vec<VolumeBar>>,
    adjustments: HashMap<String, Vec<PriceAdjustment>>,
    adjustments_fail: bool,
    limits: Mutex<Vec<i64>>,
    saved: Mutex<Option<(NaiveDate, Vec<StockTechnicalSignal>)>>,
    /// `replace_indicator_signals` 收到的來源、起訖與訊號。
    replaced: Mutex<Option<Replaced>>,
}

#[async_trait]
impl PriceBarRepository for FakeRepo {
    async fn fetch_price_bars(
        &self,
        stock_symbol: &str,
        _until: Option<NaiveDate>,
        limit: i64,
    ) -> Result<Vec<PriceBar>> {
        self.limits.lock().expect("測試鎖").push(limit);
        self.bars
            .get(stock_symbol)
            .cloned()
            .ok_or_else(|| anyhow!("連線中斷"))
    }

    async fn fetch_volumes(
        &self,
        stock_symbol: &str,
        _until: Option<NaiveDate>,
        _limit: i64,
    ) -> Result<Vec<VolumeBar>> {
        Ok(self.volumes.get(stock_symbol).cloned().unwrap_or_default())
    }

    async fn fetch_price_adjustments(
        &self,
        _stock_symbols: &[String],
    ) -> Result<HashMap<String, Vec<PriceAdjustment>>> {
        if self.adjustments_fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok(self.adjustments.clone())
    }
}

#[async_trait]
impl TechnicalSignalRepository for FakeRepo {
    async fn fetch_signal_symbols(&self, _date: NaiveDate) -> Result<Vec<String>> {
        Ok(self.symbols.clone())
    }

    async fn replace_signals(
        &self,
        date: NaiveDate,
        signals: &[StockTechnicalSignal],
    ) -> Result<()> {
        *self.saved.lock().expect("測試鎖") = Some((date, signals.to_vec()));
        Ok(())
    }

    async fn fetch_traded_symbols(&self, _from: NaiveDate, _to: NaiveDate) -> Result<Vec<String>> {
        Ok(self.symbols.clone())
    }

    async fn replace_indicator_signals(
        &self,
        indicator: &str,
        from: NaiveDate,
        to: NaiveDate,
        signals: &[StockTechnicalSignal],
    ) -> Result<()> {
        *self.replaced.lock().expect("測試鎖") =
            Some((indicator.to_owned(), from, to, signals.to_vec()));
        Ok(())
    }
}

/// 最後一日收盤由下往上穿越月線：產生「站上 20 日均線」訊號。
fn crossing_up() -> Vec<f64> {
    let mut closes = vec![10.0; 25];
    closes.push(9.0);
    closes.push(11.0);
    closes
}

/// 逐檔計算後整批寫入，並依代號排序；當日沒成交（最後一根不是當日）的不產生訊號。
#[tokio::test]
async fn compute_and_save_writes_signals_of_all_symbols() {
    let date = day(26);
    let repo = FakeRepo {
        symbols: vec!["B".into(), "A".into(), "C".into()],
        bars: HashMap::from([
            ("A".to_owned(), bars(&crossing_up())),
            ("B".to_owned(), bars(&crossing_up())),
            ("C".to_owned(), bars(&crossing_up()[..20])),
        ]),
        ..Default::default()
    };

    let count = compute_and_save(&repo, date).await.expect("計算成功");

    let (saved_date, saved) = repo.saved.lock().expect("測試鎖").take().expect("有寫入");
    assert_eq!(saved_date, date);
    assert_eq!(count, saved.len());
    let symbols: Vec<&str> = saved.iter().map(|s| s.stock_symbol.as_str()).collect();
    assert!(
        symbols.windows(2).all(|pair| pair[0] <= pair[1]),
        "{symbols:?}"
    );
    assert!(!symbols.contains(&"C"));
    let a: Vec<(&str, SignalKind)> = saved
        .iter()
        .filter(|s| s.stock_symbol == "A")
        .map(|s| (s.signal.indicator, s.signal.kind))
        .collect();
    assert!(a.contains(&("close_ma20", SignalKind::CrossAbove)), "{a:?}");
    assert!(saved.iter().all(|s| s.date == date));
    assert!(
        repo.limits
            .lock()
            .expect("測試鎖")
            .iter()
            .all(|&limit| limit == WARMUP_BARS as i64 + 1)
    );
}

/// 任一檔讀取失敗就整批不寫，保留原本的資料。
#[tokio::test]
async fn compute_and_save_keeps_table_when_a_fetch_fails() {
    let repo = FakeRepo {
        symbols: vec!["A".into(), "MISSING".into()],
        bars: HashMap::from([("A".to_owned(), bars(&crossing_up()))]),
        ..Default::default()
    };
    let error = compute_and_save(&repo, day(26))
        .await
        .expect_err("讀取失敗應回錯誤");
    assert!(
        format!("{error:#}").contains("讀取 MISSING 日線失敗"),
        "{error:#}"
    );
    assert!(repo.saved.lock().expect("測試鎖").is_none());
}

/// 連資料庫的完整流程：持續上漲 260 個交易日的股票在最後一日創 52 週新高、K 值超買。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn calculate_technical_signals_writes_table() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 calculate_technical_signals_writes_table：無資料庫連接");
        return;
    }
    let symbol = "79979SU";
    let first = NaiveDate::from_ymd_opt(1990, 1, 1).expect("有效日期");
    let last = first + chrono::Days::new(259);
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
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "HighestPrice", "LowestPrice")
           SELECT $2::date + n, $1, 100 + n, 101 + n, 99 + n FROM generate_series(0, 259) AS n"#,
    )
    .bind(symbol)
    .bind(first)
    .execute(pool)
    .await
    .expect("寫入測試日線");

    let count = calculate_technical_signals(last).await.expect("計算成功");

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT indicator, kind FROM daily_technical_signal WHERE date = $1 AND stock_symbol = $2",
    )
    .bind(last)
    .bind(symbol)
    .fetch_all(pool)
    .await
    .expect("讀回訊號");
    assert!(count >= rows.len());
    assert!(
        rows.contains(&("52_week".to_owned(), "new_high".to_owned())),
        "{rows:?}"
    );
    assert!(
        rows.contains(&("kd".to_owned(), "overbought".to_owned())),
        "{rows:?}"
    );
    cleanup().await;
}

/// 讀除權息事件失敗時整批不寫。
#[tokio::test]
async fn compute_and_save_fails_when_adjustments_fail() {
    let repo = FakeRepo {
        symbols: vec!["A".into()],
        bars: HashMap::from([("A".to_owned(), bars(&crossing_up()))]),
        adjustments_fail: true,
        ..Default::default()
    };
    let error = compute_and_save(&repo, day(26))
        .await
        .expect_err("讀取失敗應回錯誤");
    assert!(
        format!("{error:#}").contains("讀取除權息事件失敗"),
        "{error:#}"
    );
    assert!(repo.saved.lock().expect("測試鎖").is_none());
}

/// 除息跳空：未還原會跌破月線，還原後不算跌破。
#[tokio::test]
async fn compute_and_save_adjusts_bars_before_signals() {
    let mut closes = vec![10.0; 26];
    closes.push(9.0);
    let date = day(26);
    let ex_dividend = PriceAdjustment {
        date,
        cash_dividend: 1.0,
        stock_dividend: 0.0,
        share_ratio: 1.0,
    };
    let below = |repo: &FakeRepo| {
        repo.saved
            .lock()
            .expect("測試鎖")
            .take()
            .expect("有寫入")
            .1
            .iter()
            .any(|s| s.signal.indicator == "close_ma20" && s.signal.kind == SignalKind::CrossBelow)
    };

    let raw = FakeRepo {
        symbols: vec!["A".into()],
        bars: HashMap::from([("A".to_owned(), bars(&closes))]),
        ..Default::default()
    };
    compute_and_save(&raw, date).await.expect("計算成功");
    assert!(below(&raw));

    let adjusted = FakeRepo {
        symbols: vec!["A".into()],
        bars: HashMap::from([("A".to_owned(), bars(&closes))]),
        adjustments: HashMap::from([("A".to_owned(), vec![ex_dividend])]),
        ..Default::default()
    };
    compute_and_save(&adjusted, date).await.expect("計算成功");
    assert!(!below(&adjusted));
}

/// 沒有日線或只有一根時不產生交叉訊號，也不會失敗。
#[test]
fn signals_for_handles_short_history() {
    assert!(signals_for("A", day(0), &[], &[], &[]).is_empty());
    assert!(signals_for("A", day(0), &bars(&[10.0]), &[], &[]).is_empty());
}

mod range_breakout;
