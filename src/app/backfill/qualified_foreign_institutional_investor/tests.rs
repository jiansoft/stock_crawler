//! 外資持股回補的單元測試。

use rust_decimal_macros::dec;

use crate::domain::registry::entity::Stock;
use crate::infra::cache::SHARE;

use super::*;

#[test]
fn to_history_stamps_every_row_with_the_data_date() {
    let rows = vec![QfiiDto {
        stock_symbol: "2330".to_string(),
        issued_share: 25_932_370_067,
        shares_held: 17_951_023_616,
        share_holding_percentage: dec!(69.22),
    }];
    let date = NaiveDate::from_ymd_opt(2026, 8, 28).unwrap();

    let history = to_history(&rows, date);

    assert_eq!(
        history,
        vec![ForeignHolding {
            stock_symbol: "2330".to_string(),
            date,
            issued_share: 25_932_370_067,
            shares_held: 17_951_023_616,
            share_holding_percentage: dec!(69.22),
        }]
    );
}

fn dto(symbol: &str) -> QfiiDto {
    QfiiDto {
        stock_symbol: symbol.to_string(),
        issued_share: 1_000,
        shares_held: 300,
        share_holding_percentage: dec!(30),
    }
}

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

/// 週末略過；起訖同一天只有那一天。
#[test]
fn weekdays_between_skips_weekends() {
    // 2026-10-09（五）～10-13（二）：跳過 10-10、10-11。
    assert_eq!(
        weekdays_between(day(10, 9), day(10, 13)).unwrap(),
        vec![day(10, 9), day(10, 12), day(10, 13)]
    );
    assert_eq!(
        weekdays_between(day(10, 12), day(10, 12)).unwrap(),
        vec![day(10, 12)]
    );
    assert!(
        weekdays_between(day(10, 10), day(10, 11))
            .unwrap()
            .is_empty()
    );
}

/// 上櫃回應日期與查詢日相符才收；不符（休市回前一日）只留上市。
#[test]
fn day_history_keeps_otc_only_for_matching_date() {
    use twse::qualified_foreign_institutional_investor::over_the_counter::OtcQfiiSnapshot;
    let otc = OtcQfiiSnapshot {
        date: day(10, 12),
        rows: vec![dto("6488")],
    };
    let merged = day_history(&[dto("2330")], &otc, day(10, 12));
    assert_eq!(merged.len(), 2);
    assert!(merged.iter().all(|holding| holding.date == day(10, 12)));
    let stale = day_history(&[dto("2330")], &otc, day(10, 13));
    assert_eq!(stale.len(), 1);
}

/// 基準日：上市有資料取較晚者；上市休市用上櫃資料日。
#[test]
fn trend_as_of_picks_latest_available_date() {
    assert_eq!(trend_as_of(false, day(10, 12), day(10, 9)), day(10, 12));
    assert_eq!(trend_as_of(false, day(10, 12), day(10, 13)), day(10, 13));
    assert_eq!(trend_as_of(true, day(10, 12), day(10, 9)), day(10, 9));
}

/// 三個欄位都相同才不需要更新。
#[test]
fn qfii_changed_compares_all_fields() {
    let cmd = UpdateQfiiCommand {
        symbol: "2330".to_string(),
        shares_held: 300,
        share_holding_percentage: dec!(30),
        issued_share: 1_000,
    };
    assert!(!qfii_changed((1_000, 300, dec!(30)), &cmd));
    assert!(qfii_changed((1_000, 301, dec!(30)), &cmd));
    assert!(qfii_changed((1_000, 300, dec!(30.01)), &cmd));
    assert!(qfii_changed((999, 300, dec!(30)), &cmd));
}

/// 逐日迴圈：只抓平日、有資料才寫、每個平日後停頓、回傳最後一個有資料的交易日。
#[tokio::test]
async fn run_history_saves_trading_days_only() {
    let fetched = std::sync::Mutex::new(Vec::new());
    let pauses = std::sync::Mutex::new(0);
    let (summary, last) = run_history(
        day(10, 8),
        day(10, 12),
        |date| {
            fetched.lock().unwrap().push(date);
            async move {
                // 10-09 國慶補假休市：兩邊都沒有資料。
                Ok(if date == day(10, 9) {
                    Vec::new()
                } else {
                    to_history(&[dto("2330"), dto("6488")], date)
                })
            }
        },
        |history| async move { Ok(history.len() as u64) },
        || {
            *pauses.lock().unwrap() += 1;
            async {}
        },
    )
    .await
    .unwrap();
    assert_eq!(
        *fetched.lock().unwrap(),
        vec![day(10, 8), day(10, 9), day(10, 12)]
    );
    assert_eq!(*pauses.lock().unwrap(), 3);
    assert_eq!((summary.trading_days, summary.saved_rows), (2, 4));
    assert_eq!(last, Some(day(10, 12)));

    let failed = run_history(
        day(10, 12),
        day(10, 12),
        |_| async { Err(anyhow!("封鎖")) },
        |_| async { Ok(0) },
        || async {},
    )
    .await;
    assert!(failed.is_err());
}

#[tokio::test]
async fn backfill_history_rejects_reversed_range() {
    let start = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    assert!(backfill_history(start, end).await.is_err());
}

/// 驗證外資持股回補流程。
#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 execute");

    match execute().await {
        Ok(_) => {}
        Err(why) => {
            tracing::debug!("Failed to execute because {:?}", why);
        }
    }

    tracing::debug!("結束 execute");
}

/// 主檔有的股票欄位不同才更新（資料庫與快取一起），主檔沒有的略過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn update_saves_changed_stocks_and_skips_unknown() {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 update_saves_changed_stocks_and_skips_unknown：無資料庫連接");
        return;
    }
    let pool = crate::infra::database::get_connection();
    let cleanup = || async {
        let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
            .bind(vec!["79979QF".to_string(), "79979QZ".to_string()])
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM company_index WHERE security_code = '79979QF'")
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM company_word WHERE word = '79979QF'")
            .execute(pool)
            .await;
        if let Ok(mut cache) = SHARE.stocks.write() {
            cache.remove("79979QF");
        }
    };
    cleanup().await;
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing", issued_share, qfii_shares_held, qfii_share_holding_percentage)
           VALUES ('79979QF', 'Q', '79979QF', 2, false, 1000, 100, 10)"#,
    )
    .execute(pool)
    .await
    .expect("種入股票主檔");

    let cmd = |symbol: &str| UpdateQfiiCommand {
        symbol: symbol.to_string(),
        shares_held: 500,
        share_holding_percentage: dec!(25),
        issued_share: 2000,
    };
    let result = update(vec![cmd("79979QF"), cmd("79979QZ")]).await;
    let saved: (i64, i64, rust_decimal::Decimal) = sqlx::query_as(
        "SELECT issued_share, qfii_shares_held, qfii_share_holding_percentage FROM stocks WHERE stock_symbol = '79979QF'",
    )
    .fetch_one(pool)
    .await
    .expect("讀取主檔");
    let cached = SHARE
        .stocks
        .read()
        .ok()
        .and_then(|cache| cache.get("79979QF").map(|stock| stock.qfii_shares_held()));
    let unknown: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stocks WHERE stock_symbol = '79979QZ'")
            .fetch_one(pool)
            .await
            .expect("計數");
    cleanup().await;

    result.expect("更新成功");
    assert_eq!(saved, (2000, 500, dec!(25)));
    assert_eq!(cached, Some(500), "快取跟著更新");
    assert_eq!(unknown, 0, "主檔沒有的股票不新增");
}

/// 記憶體主檔：`stocks` 是現有股票，`fail_batch` 時批次寫回失敗，`saved` 記下批次內容。
#[derive(Default)]
struct FakeQfiiStocks {
    stocks: Vec<Stock>,
    fail_batch: bool,
    saved: std::sync::Mutex<Vec<Vec<String>>>,
}

#[async_trait::async_trait]
impl StockRepository for FakeQfiiStocks {
    async fn find_by_symbol(&self, symbol: &str) -> Result<Option<Stock>> {
        Ok(self
            .stocks
            .iter()
            .find(|stock| stock.symbol().0 == symbol)
            .cloned())
    }

    async fn save(&self, _stock: &Stock) -> Result<()> {
        unreachable!("外資持股改走批次寫回")
    }

    async fn save_qfii_batch(&self, stocks: &[Stock]) -> Result<u64> {
        if self.fail_batch {
            anyhow::bail!("批次寫回失敗");
        }
        let symbols: Vec<String> = stocks
            .iter()
            .map(|stock| stock.symbol().0.clone())
            .collect();
        let count = symbols.len() as u64;
        self.saved.lock().expect("saved 鎖不會中毒").push(symbols);
        Ok(count)
    }

    async fn fetch_all_active(&self) -> Result<Vec<Stock>> {
        unreachable!("外資持股不查全部主檔")
    }

    async fn update_eps_and_roe(&self) -> Result<()> {
        unreachable!("外資持股不更新 EPS")
    }

    async fn fetch_net_asset_value_per_share_is_zero(&self) -> Result<Vec<Stock>> {
        unreachable!("外資持股不查淨值")
    }

    async fn fetch_stocks_without_financial_statement(
        &self,
        _year: i32,
        _quarter: &str,
    ) -> Result<Vec<String>> {
        unreachable!("外資持股不查財報")
    }

    async fn zeroed_out_weights(&self) -> Result<()> {
        unreachable!("外資持股不改權值")
    }

    async fn update_weight(
        &self,
        _stock_symbol: &str,
        _weight: rust_decimal::Decimal,
    ) -> Result<()> {
        unreachable!("外資持股不改權值")
    }
}

/// 已發行 1,000 股、外資持有 300 股（30%）的主檔股票。
fn held_stock(symbol: &str) -> Stock {
    let mut stock = Stock::register(symbol.to_string(), symbol.to_string(), 2, 1);
    stock.update_issued_shares(1_000);
    stock.update_qfii(300, dec!(30));
    stock
}

fn qfii_cmd(symbol: &str, shares_held: i64) -> UpdateQfiiCommand {
    UpdateQfiiCommand {
        symbol: symbol.to_string(),
        shares_held,
        share_holding_percentage: dec!(30),
        issued_share: 1_000,
    }
}

/// 只有欄位變了的股票進批次；沒變的與主檔沒有的都不寫，而且整批只寫一次。
#[tokio::test]
async fn update_with_batches_only_changed_stocks() {
    let repo = FakeQfiiStocks {
        stocks: vec![held_stock("1101"), held_stock("1102")],
        ..Default::default()
    };

    update_with(
        &repo,
        vec![
            qfii_cmd("1101", 400),
            qfii_cmd("1102", 300),
            qfii_cmd("9999", 500),
        ],
    )
    .await
    .expect("比對與寫回成功");

    let saved = repo.saved.lock().expect("saved 鎖不會中毒").clone();
    assert_eq!(saved, vec![vec!["1101".to_string()]]);
}

/// 批次寫回失敗只記錄，不讓外資持股歷史的寫入跟著中斷。
#[tokio::test]
async fn update_with_logs_batch_failure_and_continues() {
    let repo = FakeQfiiStocks {
        stocks: vec![held_stock("1101")],
        fail_batch: true,
        ..Default::default()
    };

    update_with(&repo, vec![qfii_cmd("1101", 400)])
        .await
        .expect("寫回失敗不回傳錯誤");
}

/// 以本機 mock server 取代證交所與櫃買（G16）：抓取後以批次寫回股票主檔（P13）。
mod mock_sources {
    use chrono::TimeZone;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    /// 上市假代號：外資持股會從 100 股變成 400 股。
    const LISTED: &str = "79993QF";
    /// 上櫃假代號：外資持股會從 100 股變成 22,154,306 股。
    const OTC: &str = "79994QF";

    /// 上市 MI_QFIIS 的一列（12 欄：代號、名稱、ISIN、發行股數、尚可投資、持有股數、
    /// 尚可投資比率、持股比率、上限、陸資上限、異動原因、日期）。
    fn mi_qfiis() -> serde_json::Value {
        serde_json::json!({
            "stat": "OK", "date": "20260930", "selectType": "ALLBUT0999", "total": 1,
            "fields": [], "data": [[
                LISTED, "測試上市", "TW0000000000", "1,000", "100", "400",
                "10.00", "40.00", "100.00", "100.00", "", "1150930"
            ]]
        })
    }

    /// 櫃買 QFII 排行表，只有一檔；資料日取最外層的 `date`。
    fn tpex_qfii() -> serde_json::Value {
        serde_json::json!({
            "stat": "ok",
            "date": "20260930",
            "tables": [{
                "title": "櫃檯買賣股票僑外資及陸資持股比例排行表",
                "date": "115/09/30", "totalCount": 1,
                "fields": [
                    "排行", "代號", "名稱", "發行股數(A)", "僑外資及陸資尚可投資股數B=A*F-C",
                    "僑外資及陸資持有股數(C)", "僑外資及陸資尚可投資比率(D=B/A)",
                    "僑外資及陸資持股比率(E=C/A)", "法令投資上限比率(F)", "備註"
                ],
                "data": [[
                    "1", OTC, "測試上櫃", "25,219,056", "3,064,750", "22,154,306",
                    "12.15%", "87.84%", "100%", ""
                ]]
            }]
        })
    }

    /// 上市以 2026-09-30 15:00（台北）查詢，上櫃讀櫃買回應的日期；兩邊都寫回主檔與快取。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn listed_and_otc_fetch_then_batch_update_stocks() {
        dotenvy::dotenv().ok();
        if crate::infra::database::ping().await.is_err() {
            println!("跳過 listed_and_otc_fetch_then_batch_update_stocks：無資料庫連接");
            return;
        }
        let pool = crate::infra::database::get_connection();
        let symbols = vec![LISTED.to_string(), OTC.to_string()];
        let cleanup = || async {
            let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
                .bind(&symbols)
                .execute(pool)
                .await;
            if let Ok(mut cache) = SHARE.stocks.write() {
                cache.remove(LISTED);
                cache.remove(OTC);
            }
        };
        cleanup().await;
        for (symbol, market) in [(LISTED, 2), (OTC, 4)] {
            sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing", issued_share, qfii_shares_held, qfii_share_holding_percentage)
                   VALUES ($1, $1, $1, $2, false, 1000, 100, 10)"#,
            )
            .bind(symbol)
            .bind(market)
            .execute(pool)
            .await
            .expect("種入股票主檔");
        }

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rwd/zh/fund/MI_QFIIS"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mi_qfiis()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/www/zh-tw/insti/qfii"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tpex_qfii()))
            .mount(&server)
            .await;
        let taipei = FixedOffset::east_opt(8 * 3600).expect("台北時區");
        let date_time = taipei
            .with_ymd_and_hms(2026, 9, 30, 15, 0, 0)
            .single()
            .expect("有效時間");
        let uri = server.uri();
        let (listed_rows, otc_snapshot) = with_origins(
            &[
                ("https://www.twse.com.tw", &uri),
                ("https://www.tpex.org.tw", &uri),
            ],
            async { tokio::join!(listed(date_time), otc()) },
        )
        .await;
        let saved: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT stock_symbol, issued_share, qfii_shares_held FROM stocks WHERE stock_symbol = ANY($1) ORDER BY stock_symbol",
        )
        .bind(&symbols)
        .fetch_all(pool)
        .await
        .expect("讀回主檔");
        let cached = SHARE.stocks.read().ok().map(|cache| {
            (
                cache.get(LISTED).map(|stock| stock.qfii_shares_held()),
                cache.get(OTC).map(|stock| stock.qfii_shares_held()),
            )
        });
        cleanup().await;

        let listed_rows = listed_rows.expect("上市抓取成功");
        let otc_snapshot = otc_snapshot.expect("上櫃抓取成功");
        assert_eq!(listed_rows.len(), 1);
        assert_eq!(otc_snapshot.date, day(9, 30), "上櫃以櫃買回應的日期為準");
        assert_eq!(
            saved,
            vec![
                (LISTED.to_string(), 1_000, 400),
                (OTC.to_string(), 25_219_056, 22_154_306),
            ]
        );
        assert_eq!(cached, Some((Some(400), Some(22_154_306))), "快取跟著更新");
    }
}
