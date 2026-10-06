//! # 董監事持股餘額明細（含設質）
//!
//! 公開資訊觀測站每月彙整的內部人（董事、監察人、經理人、大股東）持股與設質，
//! 由交易所開放資料提供，一次回傳全市場最新一個月份，不需要查詢參數：
//!
//! | 函式 | 來源 |
//! |------|------|
//! | [`visit_listed`] | 證交所 OpenAPI `opendata/t187ap11_L`（上市） |
//! | [`visit_otc`] | 櫃買中心 OpenAPI `mopsfin_t187ap11_O`（上櫃） |
//!
//! ## 已知限制與陷阱
//!
//! - 只有**最新一個月份**，沒有歷史；要看變動得自己保存前一期。
//! - 每月約 18 日出表（例如 2026-09-18 出 8 月資料）。
//! - 解壓後上市約 10 MiB、上櫃約 7 MiB，超過 HTTP helper 預設的 8 MiB 上限，
//!   因此用 [`util::http::get_json_with_limit`] 放寬到 [`MAX_BODY_BYTES`]。
//! - 同一人有多個職稱時（例如「董事長本人」與「總經理本人」）會拆成多列、持股數字相同。
//! - 上市的「選任時持股」欄名尾端多一個空白，本模組不用這個欄位。
//!
//! BigGo 財經也有董監持股（`/stock/major-shareholder`），但 2026-10 實測上市只到 7 月、
//! 上櫃停在 2025-11，因此改用官方來源。

use anyhow::Result;
use chrono::NaiveDate;
use serde::Deserialize;

use crate::core::util::{self, datetime, text};

/// 上市公司董監事持股餘額明細。
const LISTED_URL: &str = "https://openapi.twse.com.tw/v1/opendata/t187ap11_L";

/// 上櫃公司董監事持股餘額明細。
const OTC_URL: &str = "https://www.tpex.org.tw/openapi/v1/mopsfin_t187ap11_O";

/// 回應 body 上限（32 MiB）；目前最大的上市資料解壓後約 10 MiB。
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// 開放資料的原始資料列；只宣告會用到的欄位。
#[derive(Deserialize, Debug, Clone)]
struct InsiderHoldingRaw {
    /// 資料年月（民國，例如 `11508`）。
    #[serde(rename = "資料年月")]
    data_month: String,
    /// 公司代號。
    #[serde(rename = "公司代號")]
    code: String,
    /// 職稱（例如 `董事長本人`、`董事之法人代表人`）。
    #[serde(rename = "職稱")]
    title: String,
    /// 姓名（法人董事為公司名稱）。
    #[serde(rename = "姓名")]
    name: String,
    /// 目前持股（股）。
    #[serde(rename = "目前持股")]
    shares: String,
    /// 設質股數（股）。
    #[serde(rename = "設質股數")]
    pledged: String,
    /// 內部人關係人（配偶、未成年子女等）設質股數（股）。
    #[serde(rename = "內部人關係人設質股數")]
    related_pledged: String,
}

/// 單一內部人在單一職稱下的持股與設質。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsiderHolding {
    /// 股票代號。
    pub stock_symbol: String,
    /// 資料月份（該月 1 日）。
    pub month: NaiveDate,
    /// 職稱。
    pub title: String,
    /// 姓名。
    pub name: String,
    /// 目前持股（股）。
    pub shares: i64,
    /// 設質股數（股）。
    pub pledged: i64,
    /// 內部人關係人設質股數（股）。
    pub related_pledged: i64,
}

/// 取得上市公司最新一個月份的董監事持股與設質。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit_listed() -> Result<Vec<InsiderHolding>> {
    visit(LISTED_URL).await
}

/// 取得上櫃公司最新一個月份的董監事持股與設質。
///
/// # Errors
///
/// HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit_otc() -> Result<Vec<InsiderHolding>> {
    visit(OTC_URL).await
}

async fn visit(url: &str) -> Result<Vec<InsiderHolding>> {
    let rows =
        util::http::get_json_with_limit::<Vec<InsiderHoldingRaw>>(url, MAX_BODY_BYTES).await?;
    Ok(parse_rows(rows))
}

/// 將原始資料列轉成 [`InsiderHolding`]。
///
/// 年月或股數無法解析的資料列整列丟棄並記錄警告——猜一個數字會讓上層誤判成持股或設質變動。
fn parse_rows(rows: Vec<InsiderHoldingRaw>) -> Vec<InsiderHolding> {
    rows.into_iter()
        .filter_map(|row| match parse_row(&row) {
            Some(holding) => Some(holding),
            None => {
                tracing::warn!("董監持股資料列無法解析，已略過: {row:?}");
                None
            }
        })
        .collect()
}

fn parse_row(row: &InsiderHoldingRaw) -> Option<InsiderHolding> {
    Some(InsiderHolding {
        stock_symbol: row.code.trim().to_string(),
        month: parse_month(&row.data_month)?,
        title: row.title.trim().to_string(),
        name: row.name.trim().to_string(),
        shares: text::parse_i64(&row.shares, None).ok()?,
        pledged: text::parse_i64(&row.pledged, None).ok()?,
        related_pledged: text::parse_i64(&row.related_pledged, None).ok()?,
    })
}

/// 解析民國年月（`11508` → 2026-08-01）。
fn parse_month(raw: &str) -> Option<NaiveDate> {
    let raw = raw.trim();
    let split = raw.len().checked_sub(2)?;
    let roc_year: i32 = raw.get(..split)?.parse().ok()?;
    let month: u32 = raw.get(split..)?.parse().ok()?;
    NaiveDate::from_ymd_opt(datetime::roc_year_to_gregorian_year(roc_year), month, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // include_str! 的路徑相對於本檔案（mops/insider_holding.rs）→ mops/testdata/。
    const LISTED_FIXTURE: &str = include_str!("testdata/insider_holding_t187ap11_L.json");
    const OTC_FIXTURE: &str = include_str!("testdata/insider_holding_t187ap11_O.json");

    fn parse_fixture(fixture: &str) -> Vec<InsiderHolding> {
        parse_rows(serde_json::from_str(fixture).expect("fixture 是合法 JSON"))
    }

    /// 上市資料：同一人多個職稱拆成多列、數字相同；「選任時持股 」欄名尾端的空白不影響解析。
    #[test]
    fn parse_listed_fixture() {
        let rows = parse_fixture(LISTED_FIXTURE);
        assert_eq!(rows.len(), 7);

        let wang: Vec<_> = rows.iter().filter(|row| row.name == "王貴雲").collect();
        assert_eq!(wang.len(), 2);
        assert_eq!(wang[0].stock_symbol, "1303");
        assert_eq!(wang[0].month, NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());
        assert_eq!(wang[0].title, "董事本人");
        assert_eq!(wang[0].shares, 10_723_271);
        assert_eq!(wang[0].pledged, 8_300_000);
        assert_eq!(wang[0].related_pledged, 1_224_400);
        assert_eq!(wang[1].title, "副總經理本人");
        assert_eq!(wang[1].shares, wang[0].shares);
    }

    /// 上櫃資料欄位相同（「選任時持股」沒有尾端空白）。
    #[test]
    fn parse_otc_fixture() {
        let rows = parse_fixture(OTC_FIXTURE);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].stock_symbol, "6248");
        assert_eq!(rows[0].name, "台灣鋼鐵股份有限公司");
        assert_eq!(rows[0].pledged, 3_204_000);
    }

    /// 股數或年月無法解析的列整列丟棄，不把壞資料當成 0。
    #[test]
    fn parse_rows_drops_malformed_rows() {
        let mut raw: Vec<InsiderHoldingRaw> =
            serde_json::from_str(OTC_FIXTURE).expect("fixture 是合法 JSON");
        raw[0].pledged = "N/A".to_string();
        raw[1].data_month = "115".to_string();
        assert_eq!(parse_rows(raw).len(), 2);
    }

    #[test]
    fn parse_month_converts_roc_year_month() {
        assert_eq!(parse_month("11508"), NaiveDate::from_ymd_opt(2026, 8, 1));
        assert_eq!(parse_month(" 9912 "), NaiveDate::from_ymd_opt(2010, 12, 1));
        assert_eq!(parse_month("11513"), None);
        assert_eq!(parse_month("8"), None);
        assert_eq!(parse_month(""), None);
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        let listed = visit_listed().await.expect("抓取上市董監持股");
        let otc = visit_otc().await.expect("抓取上櫃董監持股");
        dbg!(listed.len(), otc.len(), listed.first(), otc.first());
        assert!(listed.len() > 10_000);
        assert!(otc.len() > 5_000);
    }
}
