//! # 持股估值週報
//!
//! 每週六把持股的本益比、股價淨值比放回自己近 5 年的歷史裡比較，列出目前位於高檔
//! （≥ 第 90 百分位）或低檔（≤ 第 10 百分位）的持股。統計規則與 Data API 的
//! `/stocks/{symbol}/valuation-band` 相同（[`valuation_band::summarize`]）。
//!
//! - 只看持股中的普通股（[`holdings::common_stock_holdings`]）。
//! - 有效樣本不足 [`MIN_SAMPLES`] 天（新上市、長期虧損）或最新報價超過
//!   [`MAX_STALE_DAYS`] 天（停牌）的股票不列入，避免樣本太少的百分位誤導。
//! - 沒有任何持股落在高檔或低檔時不發送。

use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::quote::valuation_band::{self, DailyRatio, ValuationHistoryRepository},
    infra::database::repository::quote::PgQuoteRepository,
};

/// 回看幾年。
const YEARS: u8 = 5;

/// 百分位 ≥ 這個值算高檔。
const HIGH_PERCENTILE: f64 = 90.0;

/// 百分位 ≤ 這個值算低檔。
const LOW_PERCENTILE: f64 = 10.0;

/// 有效樣本至少幾天（約一年的交易日）。
const MIN_SAMPLES: usize = 250;

/// 最新報價距今超過幾天就不列入。
const MAX_STALE_DAYS: i64 = 10;

/// 估值倍數。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ratio {
    PriceEarning,
    PriceToBook,
}

impl Ratio {
    fn label(self) -> &'static str {
        match self {
            Self::PriceEarning => "本益比",
            Self::PriceToBook => "股價淨值比",
        }
    }

    fn pick(self, row: &DailyRatio) -> Decimal {
        match self {
            Self::PriceEarning => row.price_earning_ratio,
            Self::PriceToBook => row.price_to_book_ratio,
        }
    }
}

/// 落在高檔或低檔的一個倍數。
#[derive(Debug, Clone, PartialEq)]
struct Flag {
    stock_symbol: String,
    ratio: Ratio,
    current: f64,
    percentile: f64,
    median: f64,
    high: bool,
}

/// 排程入口：檢查持股估值並在有高檔或低檔持股時通知。
///
/// # Errors
///
/// 查詢持股失敗時回傳錯誤（交給排程告警）；單檔查詢失敗只記錄、不中斷。
pub async fn execute() -> Result<()> {
    let symbols = holdings::common_stock_holdings()
        .await
        .context("fetch active holdings for valuation alert failed")?;
    let flags = collect_flags(
        &PgQuoteRepository::new(),
        symbols.iter().map(String::as_str),
        Local::now().date_naive(),
    )
    .await;
    tracing::info!(
        "持股估值週報: holdings={} flagged={}",
        symbols.len(),
        flags.len()
    );
    if let Some(message) = build_message(&flags, holdings::stock_name) {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 逐檔取每日倍數並找出高檔與低檔；單檔失敗只記錄。
async fn collect_flags<'a>(
    repo: &impl ValuationHistoryRepository,
    symbols: impl IntoIterator<Item = &'a str>,
    today: NaiveDate,
) -> Vec<Flag> {
    let mut flags = Vec::new();
    for symbol in symbols {
        match repo.fetch_daily_ratios(symbol, YEARS).await {
            Ok(rows) => flags.extend(evaluate(symbol, &rows, today)),
            Err(why) => tracing::warn!("持股估值週報查詢 {symbol} 失敗: {why:#}"),
        }
    }
    flags
}

/// 單檔的兩個倍數各自判斷；`rows` 依日期由新到舊。
fn evaluate(symbol: &str, rows: &[DailyRatio], today: NaiveDate) -> Vec<Flag> {
    let Some(latest) = rows.first() else {
        return Vec::new();
    };
    if (today - latest.date).num_days() > MAX_STALE_DAYS {
        return Vec::new();
    }
    [Ratio::PriceEarning, Ratio::PriceToBook]
        .into_iter()
        .filter_map(|ratio| {
            let history: Vec<Decimal> = rows.iter().map(|row| ratio.pick(row)).collect();
            let band =
                valuation_band::summarize(&history, Some(ratio.pick(latest)), Some(latest.close))?;
            let (current, percentile) = band.current.zip(band.percentile)?;
            let high = percentile >= HIGH_PERCENTILE;
            (band.samples >= MIN_SAMPLES && (high || percentile <= LOW_PERCENTILE)).then(|| Flag {
                stock_symbol: symbol.to_owned(),
                ratio,
                current,
                percentile,
                median: band.median,
                high,
            })
        })
        .collect()
}

/// 組出週報（Telegram MarkdownV2）；沒有任何高檔或低檔時回 `None`。
fn build_message(flags: &[Flag], stock_name: impl Fn(&str) -> String) -> Option<String> {
    if flags.is_empty() {
        return None;
    }
    let mut message = String::with_capacity(256 + flags.len() * 96);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!("持股估值週報（近 {YEARS} 年本益比／股價淨值比）"))
    );
    for (high, heading) in [
        (true, format!("偏高（≥ 第 {HIGH_PERCENTILE} 百分位）")),
        (false, format!("偏低（≤ 第 {LOW_PERCENTILE} 百分位）")),
    ] {
        let mut group: Vec<&Flag> = flags.iter().filter(|flag| flag.high == high).collect();
        if group.is_empty() {
            continue;
        }
        // 偏高由最高排起、偏低由最低排起。
        group.sort_by(|a, b| {
            let order = a.percentile.total_cmp(&b.percentile);
            if high { order.reverse() } else { order }
        });
        let _ = write!(&mut message, "\n{}\n", text::escape_markdown_v2(heading));
        for flag in group {
            let line = format!(
                "{} {} {} {} 倍，第 {} 百分位（中位數 {} 倍）",
                flag.stock_symbol,
                stock_name(&flag.stock_symbol),
                flag.ratio.label(),
                round2(flag.current),
                flag.percentile,
                round2(flag.median)
            );
            let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
        }
    }
    Some(message)
}

/// 四捨五入到小數 2 位（倍數顯示用）。
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
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
        async fn fetch_daily_ratios(
            &self,
            stock_symbol: &str,
            years: u8,
        ) -> Result<Vec<DailyRatio>> {
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
}
