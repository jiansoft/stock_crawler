//! 持股估值週報的測試；連外的測試標 #[ignore]。

use std::collections::HashMap;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 9).expect("有效日期") - chrono::Duration::days(offset)
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
    let symbols = holdings::common_stock_holdings().await.expect("取得持股");
    let flags = collect_flags(
        &PgQuoteRepository::new(),
        symbols.iter().map(String::as_str),
        Local::now().date_naive(),
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
