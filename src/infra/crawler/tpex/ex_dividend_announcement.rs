//! # TPEx 上櫃除權除息預告表採集器
//!
//! 資料來源為櫃買中心 OpenAPI `openapi/v1/tpex_exright_prepost`，對應網頁上的
//! 「除權除息預告表」（`announce/market/ex/announce.html`）。
//!
//! ## 與 TWSE 的差異
//!
//! 欄位語意與 [`crate::infra::crawler::twse::ex_dividend_announcement`] 相同，
//! 只有欄位名稱與類別字串不同（TPEx 用 `除息`／`除權`／`除權息`，TWSE 用 `息`／`權`／`權息`），
//! 因此兩邊都轉譯成共用的 [`ExDividendAnnouncement`]，上層流程不必分辨市場。
//!
//! 另一個差別是空值表示法：TPEx 在「沒有配股」時給的是 `0.00000000` 而不是空字串，
//! 所以 0 在這裡是真正的 0；仍以空字串代表未公布。

use anyhow::Result;
use serde::Deserialize;

use crate::{
    core::{
        declare::StockExchangeMarket,
        util::{self, datetime},
    },
    infra::crawler::{
        share::{ExDividendAnnouncement, parse_ex_dividend_kind},
        tpex,
    },
};

/// TPEx OpenAPI `tpex_exright_prepost` 的原始資料列。
///
/// 欄位名稱沿用官方的拼字（含 `ExRrights` 這個官方 typo），
/// 以免因為「看起來像錯字」而改壞對應關係。
#[derive(Deserialize, Debug, Clone)]
struct TpexExRightPrepostRaw {
    /// 除權除息日期（民國 `YYYMMDD`）。
    #[serde(rename = "ExRrightsExDividendDate")]
    date: String,
    /// 股票代號。
    #[serde(rename = "SecuritiesCompanyCode")]
    code: String,
    /// 公司名稱。
    #[serde(rename = "CompanyName")]
    name: String,
    /// 除權息類別：`除息`、`除權`、`除權息`。
    #[serde(rename = "ExRrightsExDividend")]
    ex_dividend: String,
    /// 無償配股率（股/股）。
    #[serde(rename = "StockDividendRatio")]
    stock_dividend_ratio: String,
    /// 現金股利（元/股）。
    #[serde(rename = "CashDividend")]
    cash_dividend: String,
}

/// 取得上櫃股票的除權除息預告表。
///
/// 與上市版一樣是**全市場**的預告事件，一次請求即可比對出漏抓的事件。
///
/// # 錯誤
///
/// 當 HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit() -> Result<Vec<ExDividendAnnouncement>> {
    let url = format!(
        "https://{host}/openapi/v1/tpex_exright_prepost",
        host = tpex::HOST
    );
    let data = util::http::get_json::<Vec<TpexExRightPrepostRaw>>(&url).await?;

    Ok(parse_announcements(data))
}

/// 將 TPEx 原始資料列轉成共用的 [`ExDividendAnnouncement`]。
///
/// 這是純函式，可用 `testdata/ex_dividend_prepost.json` fixture 直接驗證。
fn parse_announcements(rows: Vec<TpexExRightPrepostRaw>) -> Vec<ExDividendAnnouncement> {
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
                market: StockExchangeMarket::OverTheCounter,
            })
        })
        .collect()
}

/// 解析可能為「未公布」的數值欄位；空字串或 `-` 視為未公布。
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
    /// 涵蓋：除息、除權息、純除權（現金增資，無償配股率為 0）。
    #[test]
    fn test_parse_announcements_with_fixture() {
        // include_str! 的路徑相對於本檔案（tpex/ex_dividend_announcement.rs）→ tpex/testdata/。
        const FIXTURE: &str = include_str!("testdata/ex_dividend_prepost.json");
        let rows: Vec<TpexExRightPrepostRaw> =
            serde_json::from_str(FIXTURE).expect("fixture should parse");
        let result = parse_announcements(rows);

        assert_eq!(result.len(), 4);

        let cash_only = &result[0];
        assert_eq!(cash_only.stock_symbol, "3287");
        assert_eq!(
            cash_only.ex_date,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 26).unwrap()
        );
        assert!(cash_only.is_cash);
        assert!(!cash_only.is_stock);
        assert_eq!(cash_only.cash_dividend, Some(dec!(1.5)));
        assert_eq!(cash_only.market, StockExchangeMarket::OverTheCounter);

        // 除權息：0.05999999 股/股 → 0.5999999 元。
        let both = result
            .iter()
            .find(|item| item.stock_symbol == "3128")
            .expect("3128 should exist");
        assert!(both.is_cash);
        assert!(both.is_stock);
        assert_eq!(both.cash_dividend, Some(dec!(0.4)));
        assert_eq!(both.stock_dividend(), Some(dec!(0.5999999)));

        // 純除權（現金增資）：無償配股率是真正的 0，不是未公布。
        let stock_only = result
            .iter()
            .find(|item| item.stock_symbol == "3234")
            .expect("3234 should exist");
        assert!(!stock_only.is_cash);
        assert!(stock_only.is_stock);
        assert_eq!(stock_only.stock_dividend_ratio, Some(dec!(0)));
    }

    #[test]
    fn test_parse_announcements_skips_invalid_date() {
        let rows = vec![TpexExRightPrepostRaw {
            date: "115082".to_string(),
            code: "6488".to_string(),
            name: "環球晶".to_string(),
            ex_dividend: "除息".to_string(),
            stock_dividend_ratio: "0.00000000".to_string(),
            cash_dividend: "10.00000000".to_string(),
        }];

        assert!(parse_announcements(rows).is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();

        match visit().await {
            Err(why) => println!("取得上櫃除權息預告表失敗: {:?}", why),
            Ok(result) => {
                println!("上櫃除權息預告 {} 筆", result.len());
                for item in result.iter().take(5) {
                    println!("{:?}", item);
                }
            }
        }
    }
}
