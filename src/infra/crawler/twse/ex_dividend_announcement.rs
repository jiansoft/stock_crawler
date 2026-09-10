//! # TWSE 上市除權除息預告表採集器
//!
//! 資料來源為證交所 OpenAPI `exchangeReport/TWT48U_ALL`，對應網頁上的
//! 「除權除息預告表」（`announcement/ex-right/twt48u.html`）。
//!
//! ## 為什麼用 OpenAPI 而不是 rwd 版
//!
//! 同一份資料的 `rwd/zh/exRight/TWT48U` 端點會把「待公告實際收益分配金額」
//! 這類提示以 HTML 片段（`<p style=...>`）塞進現金股利欄位，解析前得先清一輪標籤；
//! OpenAPI 版在同樣情況下直接給空字串，語意乾淨且不必處理 HTML。
//!
//! ## 欄位注意事項
//!
//! - `StockDividendRatio` 是**無償配股率**；`SubscriptionRatio` 是現金增資認購配股率，
//!   兩者意義完全不同，後者不能併入股票股利，因此本模組不採集它。
//! - 空字串代表「未公布或不適用」，會轉成 `None`，不可視為 0。

use anyhow::Result;
use serde::Deserialize;

use crate::{
    core::{
        declare::StockExchangeMarket,
        util::{self, datetime},
    },
    infra::crawler::{
        share::{ExDividendAnnouncement, parse_ex_dividend_kind},
        twse,
    },
};

/// TWSE OpenAPI `TWT48U_ALL` 的原始資料列。
///
/// 只宣告本模組會用到的欄位；`serde` 預設忽略其餘欄位，
/// 交易所日後增欄不會讓反序列化失敗。
#[derive(Deserialize, Debug, Clone)]
struct Twt48uRaw {
    /// 除權除息日期（民國 `YYYMMDD`）。
    #[serde(rename = "Date")]
    date: String,
    /// 股票代號。
    #[serde(rename = "Code")]
    code: String,
    /// 股票名稱。
    #[serde(rename = "Name")]
    name: String,
    /// 除權息類別：`息`、`權`、`權息`。
    #[serde(rename = "Exdividend")]
    ex_dividend: String,
    /// 無償配股率（股/股）。
    #[serde(rename = "StockDividendRatio")]
    stock_dividend_ratio: String,
    /// 現金股利（元/股）。
    #[serde(rename = "CashDividend")]
    cash_dividend: String,
}

/// 取得上市股票的除權除息預告表。
///
/// 回傳的是**全市場**尚未除權息的預告事件（不需帶查詢參數），
/// 因此一次請求就能與資料庫比對出漏抓的事件。
///
/// # 錯誤
///
/// 當 HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit() -> Result<Vec<ExDividendAnnouncement>> {
    let url = format!(
        "https://openapi.{host}/v1/exchangeReport/TWT48U_ALL",
        host = twse::HOST
    );
    let data = util::http::get_json::<Vec<Twt48uRaw>>(&url).await?;

    Ok(parse_announcements(data))
}

/// 將 TWSE 原始資料列轉成共用的 [`ExDividendAnnouncement`]。
///
/// 這是純函式，可用 `testdata/ex_dividend_twt48u.json` fixture 直接驗證。
/// 日期無法解析（格式異常）的資料列會被丟棄，避免把壞資料帶進後續流程。
fn parse_announcements(rows: Vec<Twt48uRaw>) -> Vec<ExDividendAnnouncement> {
    rows.into_iter()
        .filter_map(|row| {
            let ex_date = datetime::parse_taiwan_date_short(row.date.trim())?;
            let (is_cash, is_stock) = parse_ex_dividend_kind(&row.ex_dividend);

            Some(ExDividendAnnouncement {
                stock_symbol: row.code.trim().to_string(),
                name: row.name.trim().to_string(),
                ex_date,
                is_cash,
                is_stock,
                cash_dividend: parse_optional_decimal(&row.cash_dividend),
                stock_dividend_ratio: parse_optional_decimal(&row.stock_dividend_ratio),
                market: StockExchangeMarket::Listed,
            })
        })
        .collect()
}

/// 解析可能為「未公布」的數值欄位。
///
/// 空字串、`-` 或無法解析的內容都視為尚未公布（`None`），
/// 這與「公布了但金額為 0」是不同語意，不可混為一談。
fn parse_optional_decimal(value: &str) -> Option<rust_decimal::Decimal> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }

    util::text::parse_decimal(trimmed, None).ok()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 以真實回應 fixture 驗證解析結果。
    ///
    /// 涵蓋：純除息、除權息、現金股利未公布（ETF 的「待公告」）、
    /// 民國日期轉西元、無償配股率換算成元。
    #[test]
    fn test_parse_announcements_with_fixture() {
        // include_str! 的路徑相對於本檔案（twse/ex_dividend_announcement.rs）→ twse/testdata/。
        const FIXTURE: &str = include_str!("testdata/ex_dividend_twt48u.json");
        let rows: Vec<Twt48uRaw> = serde_json::from_str(FIXTURE).expect("fixture should parse");
        let result = parse_announcements(rows);

        assert_eq!(result.len(), 5);

        let first = &result[0];
        assert_eq!(first.stock_symbol, "00400A");
        assert_eq!(
            first.ex_date,
            chrono::NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()
        );
        assert!(first.is_cash);
        assert!(!first.is_stock);
        assert_eq!(first.cash_dividend, Some(dec!(0.120000)));
        assert_eq!(first.market, StockExchangeMarket::Listed);

        // ETF 尚未公告實際收益分配金額 → None，而不是 0。
        let unannounced = result
            .iter()
            .find(|item| item.stock_symbol == "00401A")
            .expect("00401A should exist");
        assert!(unannounced.cash_dividend.is_none());

        // 權息同時發放：無償配股率 0.08 → 股票股利 0.8 元。
        let both = result
            .iter()
            .find(|item| item.stock_symbol == "1438")
            .expect("1438 should exist");
        assert!(both.is_cash);
        assert!(both.is_stock);
        assert_eq!(both.stock_dividend_ratio, Some(dec!(0.08000000)));
        assert_eq!(both.stock_dividend(), Some(dec!(0.8000000)));
    }

    #[test]
    fn test_parse_optional_decimal_treats_blank_as_unannounced() {
        assert_eq!(parse_optional_decimal(""), None);
        assert_eq!(parse_optional_decimal("  "), None);
        assert_eq!(parse_optional_decimal("-"), None);
        assert_eq!(parse_optional_decimal("0.000000"), Some(dec!(0)));
        assert_eq!(parse_optional_decimal("1.25"), Some(dec!(1.25)));
    }

    /// 日期格式異常的資料列必須被丟棄，而不是帶著錯誤日期往下走。
    #[test]
    fn test_parse_announcements_skips_invalid_date() {
        let rows = vec![Twt48uRaw {
            date: "abc".to_string(),
            code: "2330".to_string(),
            name: "台積電".to_string(),
            ex_dividend: "息".to_string(),
            stock_dividend_ratio: "".to_string(),
            cash_dividend: "5.0".to_string(),
        }];

        assert!(parse_announcements(rows).is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();

        match visit().await {
            Err(why) => println!("取得上市除權息預告表失敗: {:?}", why),
            Ok(result) => {
                println!("上市除權息預告 {} 筆", result.len());
                for item in result.iter().take(5) {
                    println!("{:?}", item);
                }
            }
        }
    }
}
