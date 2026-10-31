use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;
use crate::domain::dividend::fill::{DailyClose, ExRightEvent};
use crate::domain::dividend::fill_stats::FillSample;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("日期應合法")
}

/// 2330：7 月除息已填息、10 月除息仍差 5%；0056：9 月除息仍差 10%；
/// 1101：去年除息（超過回看期間，只計入近 5 年的歷年統計），第 2 個交易日填息。
struct FakeFills;

#[async_trait]
impl DividendFillQuery for FakeFills {
    async fn fetch_latest_quote_date(&self, _symbol: &str) -> Result<Option<NaiveDate>> {
        unreachable!("持股填息不需要最新收盤日")
    }

    async fn fetch_ex_right_events(&self, symbol: &str) -> Result<Vec<ExRightEvent>> {
        let event = |ex_date, cash| ExRightEvent {
            ex_date,
            cash_dividend: cash,
            stock_dividend: Decimal::ZERO,
        };
        Ok(match symbol {
            "2330" => vec![event(day(10, 14), dec!(5)), event(day(7, 10), dec!(5))],
            "0056" => vec![event(day(9, 15), dec!(1))],
            "1101" => vec![event(
                NaiveDate::from_ymd_opt(2025, 7, 1).expect("日期應合法"),
                dec!(1),
            )],
            _ => Vec::new(),
        })
    }

    async fn fetch_daily_closes(&self, symbol: &str, _since: NaiveDate) -> Result<Vec<DailyClose>> {
        let closes: Vec<(NaiveDate, Decimal)> = match symbol {
            "2330" => vec![
                (day(7, 9), dec!(100)),
                (day(7, 10), dec!(96)),
                (day(7, 20), dec!(101)),
                (day(10, 13), dec!(200)),
                (day(10, 14), dec!(190)),
                (day(10, 20), dec!(190)),
            ],
            "0056" => vec![
                (day(9, 12), dec!(40)),
                (day(9, 15), dec!(36)),
                (day(10, 20), dec!(36)),
            ],
            "1101" => {
                let y2025 = |m, d| NaiveDate::from_ymd_opt(2025, m, d).expect("日期應合法");
                vec![
                    (y2025(6, 30), dec!(20)),
                    (y2025(7, 1), dec!(19)),
                    (y2025(7, 2), dec!(20.5)),
                ]
            }
            _ => panic!("不應查 {symbol} 的日線"),
        };
        Ok(closes
            .into_iter()
            .map(|(date, close)| DailyClose { date, close })
            .collect())
    }
}

/// 只取期間內的除權息；未填息依缺口大到小在前，已填息在後；統計涵蓋全部事件。
#[tokio::test]
async fn response_lists_unfilled_first() {
    let holdings = [("0056", "元大高股息"), ("1101", "台泥"), ("2330", "台積電")]
        .map(|(symbol, name)| (symbol.to_owned(), name.to_owned()))
        .to_vec();
    let outcomes = collect(&FakeFills, &holdings, day(10, 20), 365)
        .await
        .expect("假倉儲不會失敗");
    let sample = |bucket, days| FillSample {
        bucket,
        trading_days_to_fill: Some(days),
        trading_days_observed: 120,
    };
    let peers = bucket_stats(&[
        sample(YieldBucket::Below3, 4),
        sample(YieldBucket::From5To7, 30),
    ]);
    let body = response(0, 365, outcomes, &peers);
    let order: Vec<(&str, &str)> = body
        .items
        .iter()
        .map(|item| (item.stock_symbol.as_str(), item.fill.ex_date.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ("0056", "2026-09-15"),
            ("2330", "2026-10-14"),
            ("2330", "2026-07-10"),
        ]
    );
    assert_eq!(body.items[0].name, "元大高股息");
    assert_eq!(body.items[0].fill.gap_percent, Some(10.0));
    assert!(body.items[2].fill.filled);
    assert_eq!((body.summary.events, body.summary.filled), (3, 1));
    assert_eq!(body.days, 365);
    // 歷年統計：同一檔每列相同；超過回看期間的 1101 不列入清單。
    let history = |symbol: &str| {
        let item = body
            .items
            .iter()
            .find(|item| item.stock_symbol == symbol)
            .unwrap_or_else(|| panic!("清單應有 {symbol}"));
        (
            item.history.events,
            item.history.filled,
            item.history.average_trading_days_to_fill,
        )
    };
    assert_eq!(history("2330"), (2, 1, Some(2.0)));
    assert_eq!(history("0056"), (1, 0, None));
    assert!(body.items.iter().all(|item| item.stock_symbol != "1101"));
    // 同組基準：0056 現金 1 ÷ 40 = 2.5%、2330 07-10 現金 5 ÷ 100 = 5%。
    let peer = |index: usize| {
        body.items[index]
            .peer
            .as_ref()
            .map(|peer| (peer.bucket.as_str(), peer.median_trading_days_to_fill))
    };
    assert_eq!(peer(0), Some(("below_3", Some(4.0))));
    assert_eq!(peer(2), Some(("from_5_to_7", Some(30.0))));

    let outcomes = collect(&FakeFills, &holdings, day(10, 20), 365)
        .await
        .expect("假倉儲不會失敗");
    let without_peers = response(0, 365, outcomes, &[]);
    assert!(
        without_peers.items.iter().all(|item| item.peer.is_none()),
        "基準查詢失敗時 peer 留空"
    );
}

/// 回看期間外、近 5 年內的除權息只計入歷年統計；超過 5 年的不計。
#[tokio::test]
async fn collect_counts_history_beyond_window() {
    let holdings = vec![("1101".to_owned(), "台泥".to_owned())];
    let within = collect(&FakeFills, &holdings, day(10, 20), 30)
        .await
        .expect("假倉儲不會失敗");
    assert!(within[0].recent.is_empty());
    assert_eq!((within[0].history.events, within[0].history.filled), (1, 1));
    assert_eq!(within[0].history.average_trading_days, Some(2.0));

    let far = NaiveDate::from_ymd_opt(2031, 7, 2).expect("日期應合法");
    let beyond = collect(&FakeFills, &holdings, far, 30)
        .await
        .expect("假倉儲不會失敗");
    assert_eq!(beyond[0].history.events, 0, "超過 5 年不計入");
}

/// 連資料庫：指定會員只取該會員未售出的持股，0 為全部會員合計；已售出的不列入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn fetch_holdings_filters_by_member() {
    const MEMBER_A: i64 = 979_793;
    const MEMBER_B: i64 = 979_794;
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 fetch_holdings_filters_by_member：無資料庫連接");
        return;
    }
    let cleanup = || async {
        sqlx::query("DELETE FROM stock_ownership_details WHERE member_id IN ($1, $2)")
            .bind(MEMBER_A)
            .bind(MEMBER_B)
            .execute(pool)
            .await
            .expect("清除測試持股");
    };
    cleanup().await;
    sqlx::query(
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold) \
         VALUES ($1, '79979FA', 1000, -1, false), ($1, '79979FS', 500, -1, true), ($2, '79979FB', 2000, -1, false)",
    )
    .bind(MEMBER_A)
    .bind(MEMBER_B)
    .execute(pool)
    .await
    .expect("種入測試持股");

    let symbols = |rows: Vec<(String, String)>| -> Vec<String> {
        rows.into_iter()
            .map(|(symbol, _)| symbol)
            .filter(|symbol| symbol.starts_with("79979F"))
            .collect()
    };
    let only_a = fetch_holdings(pool, MEMBER_A).await.expect("查詢會員 A");
    let only_b = fetch_holdings(pool, MEMBER_B).await.expect("查詢會員 B");
    let all = fetch_holdings(pool, 0).await.expect("查詢全部會員");
    cleanup().await;
    assert_eq!(symbols(only_a), ["79979FA"]);
    assert_eq!(symbols(only_b), ["79979FB"]);
    assert_eq!(symbols(all), ["79979FA", "79979FB"]);
}
