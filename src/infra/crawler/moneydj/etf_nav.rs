//! # MoneyDJ ETF 歷史淨值與市價
//!
//! 證交所 MIS 只給當天的預估淨值，`etf_daily_nav` 從上線那天才開始累積；
//! 歷史改由 MoneyDJ「淨值及折溢價」頁的走勢圖資料回補：
//! `https://www.moneydj.com/ETF/X/xdjbcd/Basic0003BCD.xdjbcd?etfid=0050.TW&b=20231001&c=20261021`。
//!
//! ## 格式
//!
//! 三段以空白分隔、各段以逗號分隔且等長：日期（`YYYYMMDD`）、淨值、市價。
//! 代號不存在時回傳空字串（可能帶 NUL 字元）。
//!
//! ## 已知限制與陷阱
//!
//! - 上市、上櫃 ETF 一律加 `.TW`（`.TWO` 查無資料）。
//! - 價格已依分割調整（0050 在 2025 年 1 拆 4，2016 年的值約為當時的四分之一），
//!   淨值與市價用同一個基準，折溢價不受影響。
//! - 當天的淨值在正式淨值公布前會沿用前一天的數字（債券、期貨型隔天才公布），
//!   因此回補只抓到前一天，當天交給 MIS。
//! - 實測 10-21 的淨值、市價與證交所回填的正式淨值、收盤價逐檔一致。

use anyhow::{Result, bail};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::{core::util, domain::etf::EtfNavHistoryPoint, infra::crawler::moneydj};

/// 抓取一檔 ETF 在 `from`～`to`（含）之間每個交易日的淨值與市價。查無資料時回傳空集合。
///
/// # Errors
///
/// HTTP 請求失敗，或回應格式不符（三段長度不同）時回傳錯誤。
pub async fn visit(
    symbol: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<EtfNavHistoryPoint>> {
    let url = format!(
        "https://www.{host}/ETF/X/xdjbcd/Basic0003BCD.xdjbcd?etfid={symbol}.TW&b={from}&c={to}",
        host = moneydj::WEB_HOST,
        from = from.format("%Y%m%d"),
        to = to.format("%Y%m%d"),
    );
    let text = util::http::get(&url, None).await?;
    parse(&text)
}

/// 解析走勢圖資料。淨值無法解析或不為正數的日期略過；市價無法解析或不為正數時為 `None`。
///
/// # Errors
///
/// 不是三段、或三段長度不同時回傳錯誤（多半是來源改版）。
pub(crate) fn parse(text: &str) -> Result<Vec<EtfNavHistoryPoint>> {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let sections: Vec<Vec<&str>> = trimmed
        .split_whitespace()
        .map(|section| section.split(',').collect())
        .collect();
    let [dates, navs, prices] = sections.as_slice() else {
        bail!("MoneyDJ 淨值資料應為三段，實際 {} 段", sections.len());
    };
    if dates.len() != navs.len() || dates.len() != prices.len() {
        bail!(
            "MoneyDJ 淨值資料長度不一致: dates={}, navs={}, prices={}",
            dates.len(),
            navs.len(),
            prices.len()
        );
    }
    Ok(dates
        .iter()
        .zip(navs)
        .zip(prices)
        .filter_map(|((date, nav), price)| {
            Some(EtfNavHistoryPoint {
                date: NaiveDate::parse_from_str(date.trim(), "%Y%m%d").ok()?,
                nav: positive(nav)?,
                market_price: positive(price),
            })
        })
        .collect())
}

fn positive(text: &str) -> Option<Decimal> {
    text.trim()
        .parse::<Decimal>()
        .ok()
        .filter(|value| value.is_sign_positive() && !value.is_zero())
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).expect("日期應合法")
    }

    #[test]
    fn parses_fixture() {
        let points = parse(include_str!("testdata/etf_nav_0050.txt")).expect("fixture 應可解析");
        assert_eq!(points.len(), 14);
        assert_eq!(
            points.first(),
            Some(&EtfNavHistoryPoint {
                date: day(10, 1),
                nav: dec!(112.8700),
                market_price: Some(dec!(112.9000)),
            })
        );
        assert_eq!(
            points.last(),
            Some(&EtfNavHistoryPoint {
                date: day(10, 21),
                nav: dec!(108.2700),
                market_price: Some(dec!(108.1500)),
            })
        );
    }

    #[test]
    fn empty_body_means_unknown_symbol() {
        assert!(parse("").expect("空字串").is_empty());
        assert!(parse("\0\n").expect("NUL").is_empty());
    }

    #[test]
    fn skips_bad_nav_and_keeps_missing_price() {
        let points = parse("20261020,20261021,x 0,108.27,1 107.2,0,5").expect("應可解析");
        assert_eq!(
            points,
            vec![EtfNavHistoryPoint {
                date: day(10, 21),
                nav: dec!(108.27),
                market_price: None,
            }]
        );
    }

    #[test]
    fn rejects_changed_format() {
        assert!(parse("20261020,20261021 1,2").is_err());
        assert!(parse("20261020,20261021 1,2 3").is_err());
    }
}
