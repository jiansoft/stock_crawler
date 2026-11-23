//! 持股估值週報的測試；連外的測試標 #[ignore]。

use crate::core::util::datetime::today;
use jiff::ToSpan;
use jiff::civil::Date;
use jiff::civil::date;
use std::collections::HashMap;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;

fn day(offset: i64) -> Date {
    date(2026, 10, 9) - (offset).days()
}

/// `days` 天的每日倍數（由新到舊），最新一天本益比為 `latest_pe`，之前依序為 1..days 倍；
/// 股價淨值比固定 2 倍（百分位 100，但所有樣本都等於目前值）。
fn history(days: usize, latest_pe: Decimal) -> Vec<DailyRatio> {
    (0..days)
        .map(|index| DailyRatio {
            date: day(index as i64),
            close: Decimal::from(100),
            price_earning_ratio: if index == 0 {
                latest_pe
            } else {
                Decimal::from(index)
            },
            price_to_book_ratio: Decimal::TWO,
        })
        .collect()
}

/// 假倉儲：依代號回傳固定資料，沒有的代號回錯誤。
struct FakeRepo(HashMap<&'static str, Vec<DailyRatio>>);

#[async_trait]
impl ValuationHistoryRepository for FakeRepo {
    async fn fetch_daily_ratios(&self, stock_symbol: &str, years: u8) -> Result<Vec<DailyRatio>> {
        assert_eq!(years, YEARS);
        self.0
            .get(stock_symbol)
            .cloned()
            .ok_or_else(|| anyhow!("查無 {stock_symbol}"))
    }
}

/// 對 `.env` 的資料庫產生一次週報並印出，不發送（唯讀）。
///
/// `cargo test app::event::taiwan_stock::valuation_alert::tests::print_message -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn print_message() {
    dotenvy::dotenv().ok();
    crate::infra::cache::SHARE.load().await;
    let symbols = holdings::crawl_common_stocks().await.expect("取得持股");
    let flags = collect_flags(
        &PgQuoteRepository::new(),
        symbols.iter().map(String::as_str),
        today(),
    )
    .await;
    println!("持股 {} 檔、標記 {} 筆", symbols.len(), flags.len());
    println!(
        "{}",
        build_message(&flags, holdings::stock_name).unwrap_or_default()
    );
}

/// 本益比在高檔與低檔各列一次；股價淨值比所有樣本相同、百分位 100 也算高檔。
#[test]
fn evaluate_flags_high_and_low_percentiles() {
    let today = day(0);
    let high = evaluate("2330", &history(300, Decimal::from(400)), today);
    assert_eq!(high.len(), 2);
    assert_eq!(high[0].ratio, Ratio::PriceEarning);
    assert!(high[0].high && high[0].percentile >= HIGH_PERCENTILE);
    assert_eq!(high[1].ratio, Ratio::PriceToBook);

    let low = evaluate("2330", &history(300, Decimal::ONE), today);
    let pe = low
        .iter()
        .find(|flag| flag.ratio == Ratio::PriceEarning)
        .expect("本益比低檔");
    assert!(!pe.high && pe.percentile <= LOW_PERCENTILE);

    let middle = evaluate("2330", &history(300, Decimal::from(150)), today);
    assert!(middle.iter().all(|flag| flag.ratio == Ratio::PriceToBook));
}

/// 樣本不足、報價過期、沒有資料、目前虧損都不列入。
#[test]
fn evaluate_skips_thin_stale_and_loss_making_history() {
    let today = day(0);
    assert!(evaluate("6488", &history(100, Decimal::from(400)), today).is_empty());
    assert!(evaluate("6488", &history(300, Decimal::from(400)), day(-11)).is_empty());
    assert!(evaluate("6488", &[], today).is_empty());
    let loss = evaluate("6488", &history(300, Decimal::ZERO), today);
    assert!(loss.iter().all(|flag| flag.ratio == Ratio::PriceToBook));
}

/// 逐檔查詢，失敗的代號略過；組出的週報偏高由高排起、偏低由低排起，沒有標記時不發送。
#[tokio::test]
async fn collect_flags_and_build_message() {
    let repo = FakeRepo(HashMap::from([
        ("2330", history(300, Decimal::from(400))),
        ("2317", history(300, Decimal::ONE)),
    ]));
    let flags = collect_flags(&repo, ["2330", "2317", "9999"], day(0)).await;
    assert_eq!(flags.len(), 4);

    let message = build_message(&flags, |symbol| match symbol {
        "2330" => "台積電".to_owned(),
        _ => "鴻海".to_owned(),
    })
    .expect("有標記就有訊息");
    assert!(
        message.starts_with("持股估值週報（近 5 年本益比／股價淨值比）\n"),
        "{message}"
    );
    assert!(message.contains("偏高（≥ 第 90 百分位）"), "{message}");
    assert!(
        message.contains("2330 台積電 本益比 400 倍，第 100 百分位（中位數 150\\.5 倍）"),
        "{message}"
    );
    assert!(message.contains("偏低（≤ 第 10 百分位）"), "{message}");
    assert!(
        message.contains("2317 鴻海 本益比 1 倍，第 0\\.7 百分位"),
        "{message}"
    );

    assert_eq!(build_message(&[], |_| String::new()), None);
    assert_eq!(round2(2.7432), 2.74);
    let only_low: Vec<Flag> = flags.into_iter().filter(|flag| !flag.high).collect();
    let message = build_message(&only_low, |_| String::new()).expect("有低檔");
    assert!(!message.contains("偏高"), "{message}");
}

/// 收件者範圍的估值通知以資料庫的持股為準：只列範圍內持有的普通股，沒有持股時不產生訊息。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn view_message_uses_holdings_in_scope() {
    use crate::domain::portfolio::view::HoldingView;
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 view_message_uses_holdings_in_scope：無資料庫連接");
        return;
    }
    const MEMBER: i64 = 979_904;
    let cleanup = || async {
        sqlx::query("DELETE FROM stock_ownership_details WHERE member_id = $1")
            .bind(MEMBER)
            .execute(pool)
            .await
            .expect("清除測試持股");
    };
    cleanup().await;
    sqlx::query(
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, created_time, date) \
         VALUES ($1, '2330', 1000, '1999-03-10 00:00:00+08', '1999-03-10')",
    )
    .bind(MEMBER)
    .execute(pool)
    .await
    .expect("插入測試持股");
    let flag = |symbol: &str| Flag {
        stock_symbol: symbol.to_string(),
        ratio: Ratio::PriceEarning,
        current: 30.0,
        percentile: 95.0,
        median: 20.0,
        high: true,
    };
    let flags = [flag("2330"), flag("1101")];
    let held = view_message(&flags, HoldingView::member(1, MEMBER)).await;
    let empty = view_message(&flags, HoldingView::member(1, MEMBER + 1)).await;
    cleanup().await;
    let message = held.expect("查詢持股").expect("持有 2330 應收到估值通知");
    assert!(!message.contains("1101"), "範圍外的持股不可出現：{message}");
    assert_eq!(empty.expect("查詢持股"), None);
}
