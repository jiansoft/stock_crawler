//! # 公開資訊觀測站「上市公司股利分派情形」採集器
//!
//! 資料來源為證交所 OpenAPI `opendata/t187ap45_L`。
//!
//! ## 這支 API 補的是什麼
//!
//! 交易所的除權除息預告表只有「除權息日期 + 金額」，**沒有股利所屬期間**，
//! 而 `dividend` 資料表的唯一鍵是 `(security_code, year, quarter)`，
//! 少了所屬期間就無法決定一筆預告事件該落在哪一列。這支 API 補的正是這塊：
//!
//! - `股利所屬年(季)度`／`股利所屬期間` → `year_of_dividend` 與 `quarter`
//! - 盈餘／法定盈餘公積／資本公積的現金與配股六個欄位 → 現金與股票股利的來源拆分
//!
//! ## 已知限制
//!
//! 櫃買中心沒有對應的開放 API（`t187ap45_O` 的各種路徑都會轉址到網頁），
//! 因此**只有上市公司**能從這裡取得所屬期間，上櫃仍需仰賴既有來源。

use anyhow::Result;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::core::util::{self, datetime};

/// MOPS `t187ap45_L` 的原始資料列。
///
/// 只宣告會用到的欄位；`摘錄公司章程-股利分派部分` 這種長文字欄位一律忽略。
#[derive(Deserialize, Debug, Clone)]
struct DividendAllotmentRaw {
    /// 公司代號。
    #[serde(rename = "公司代號")]
    code: String,
    /// 決議（擬議）進度，例如「董事會決議」、「股東會確認」。
    #[serde(rename = "決議（擬議）進度")]
    progress: String,
    /// 股利年度（民國年）。
    #[serde(rename = "股利年度")]
    dividend_year: String,
    /// 股利所屬年(季)度：`年度`、`上半年`、`下半年`、`第1季`～`第4季`。
    #[serde(rename = "股利所屬年(季)度")]
    period_kind: String,
    /// 股利所屬期間，格式為 `1140101~1141231`（民國）。
    #[serde(rename = "股利所屬期間")]
    period: String,
    /// 盈餘分配之現金股利（元/股）。
    #[serde(rename = "股東配發-盈餘分配之現金股利(元/股)")]
    earnings_cash: String,
    /// 法定盈餘公積發放之現金（元/股）。
    #[serde(rename = "股東配發-法定盈餘公積發放之現金(元/股)")]
    legal_reserve_cash: String,
    /// 資本公積發放之現金（元/股）。
    #[serde(rename = "股東配發-資本公積發放之現金(元/股)")]
    capital_reserve_cash: String,
    /// 盈餘轉增資配股（元/股）。
    #[serde(rename = "股東配發-盈餘轉增資配股(元/股)")]
    earnings_stock: String,
    /// 法定盈餘公積轉增資配股（元/股）。
    #[serde(rename = "股東配發-法定盈餘公積轉增資配股(元/股)")]
    legal_reserve_stock: String,
    /// 資本公積轉增資配股（元/股）。
    #[serde(rename = "股東配發-資本公積轉增資配股(元/股)")]
    capital_reserve_stock: String,
}

/// 單一公司單一期別的股利分派情形。
///
/// 現金與配股都依「盈餘」與「公積」兩類彙整：`dividend` 資料表只有
/// `earnings_*` 與 `capital_reserve_*` 兩個欄位，而 MOPS 把公積再細分成
/// 法定盈餘公積與資本公積，因此本結構在此把兩種公積合併。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DividendAllotment {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股利所屬年度（西元）。
    pub year_of_dividend: i32,
    /// 所屬季度：空字串代表年度，`Q1`～`Q4` 為單季，`H1`／`H2` 為上下半年。
    pub quarter: String,
    /// 股利所屬期間起日。
    pub period_start: Option<NaiveDate>,
    /// 股利所屬期間迄日。
    pub period_end: Option<NaiveDate>,
    /// 盈餘現金股利（元/股）。
    pub earnings_cash: Decimal,
    /// 公積現金股利（元/股），含法定盈餘公積與資本公積。
    pub capital_reserve_cash: Decimal,
    /// 盈餘股票股利（元/股）。
    pub earnings_stock: Decimal,
    /// 公積股票股利（元/股），含法定盈餘公積與資本公積。
    pub capital_reserve_stock: Decimal,
    /// 決議（擬議）進度原文。
    pub progress: String,
}

impl DividendAllotment {
    /// 現金股利合計（元/股）。
    pub fn cash_dividend(&self) -> Decimal {
        self.earnings_cash + self.capital_reserve_cash
    }

    /// 股票股利合計（元/股）。
    pub fn stock_dividend(&self) -> Decimal {
        self.earnings_stock + self.capital_reserve_stock
    }
}

/// 取得上市公司股利分派情形。
///
/// 這支 API 一次回傳全部上市公司的最新分派資料（近千筆），不需要查詢參數。
///
/// # 錯誤
///
/// 當 HTTP 請求或 JSON 反序列化失敗時回傳錯誤。
pub async fn visit() -> Result<Vec<DividendAllotment>> {
    let url = "https://openapi.twse.com.tw/v1/opendata/t187ap45_L";
    let data = util::http::get_json::<Vec<DividendAllotmentRaw>>(url).await?;

    Ok(parse_allotments(data))
}

/// 將 MOPS 原始資料列轉成 [`DividendAllotment`]。
///
/// 這是純函式，可用 `testdata/dividend_allotment_t187ap45_L.json` fixture 直接驗證。
/// 股利年度無法解析時整列丟棄——沒有年度就無法對應到 `dividend` 的任何一列。
fn parse_allotments(rows: Vec<DividendAllotmentRaw>) -> Vec<DividendAllotment> {
    rows.into_iter()
        .filter_map(|row| {
            let roc_year = util::text::parse_i32(row.dividend_year.trim(), None).ok()?;
            let year_of_dividend = datetime::roc_year_to_gregorian_year(roc_year);
            let (period_start, period_end) = parse_period(&row.period);
            // 期別判不出來就整列丟棄。默認成「年度」會讓上層把它當成已確定的年配，
            // 進而寫錯資料列；丟掉之後該事件會落到 Yahoo 逐檔補救那條路。
            let Some(quarter) = resolve_quarter(&row.period_kind, period_start, period_end) else {
                tracing::warn!(
                    "無法判定 {} 的股利期別，已略過: 期別={:?} 期間={:?}",
                    row.code.trim(),
                    row.period_kind,
                    row.period
                );
                return None;
            };

            Some(DividendAllotment {
                stock_symbol: row.code.trim().to_string(),
                year_of_dividend,
                quarter,
                period_start,
                period_end,
                earnings_cash: parse_amount(&row.earnings_cash),
                capital_reserve_cash: parse_amount(&row.legal_reserve_cash)
                    + parse_amount(&row.capital_reserve_cash),
                earnings_stock: parse_amount(&row.earnings_stock),
                capital_reserve_stock: parse_amount(&row.legal_reserve_stock)
                    + parse_amount(&row.capital_reserve_stock),
                progress: row.progress.trim().to_string(),
            })
        })
        .collect()
}

/// 解析 `1140101~1141231` 形式的所屬期間。
///
/// 任一端解析失敗時該端為 `None`，不影響另一端，也不會讓整列被丟棄——
/// 期間只是季別判定的備援，主要判定仍以「股利所屬年(季)度」文字為準。
fn parse_period(period: &str) -> (Option<NaiveDate>, Option<NaiveDate>) {
    let mut parts = period.trim().split('~');
    let start = parts
        .next()
        .and_then(|value| datetime::parse_taiwan_date_short(value.trim()));
    let end = parts
        .next()
        .and_then(|value| datetime::parse_taiwan_date_short(value.trim()));

    (start, end)
}

/// 依 MOPS 的「股利所屬年(季)度」文字判定季別代碼。
///
/// 實測 `t187ap45_L` 只會出現 `年度`、`上半年`、`下半年`、`第1季`～`第4季` 六種值，
/// 直接對應成專案既有的季別慣例（空字串／`H1`／`H2`／`Q1`～`Q4`）。
/// 若日後出現未知文字，改用所屬期間的起訖月份推算。
///
/// 兩種方式都判不出來時回傳 `None`——這裡**不能**退回空字串，
/// 空字串在資料表裡代表「年度」，是個有意義的值，猜錯會直接寫錯資料列。
fn resolve_quarter(
    period_kind: &str,
    period_start: Option<NaiveDate>,
    period_end: Option<NaiveDate>,
) -> Option<String> {
    match period_kind.trim() {
        "年度" => return Some(String::new()),
        "上半年" => return Some("H1".to_string()),
        "下半年" => return Some("H2".to_string()),
        "第1季" => return Some("Q1".to_string()),
        "第2季" => return Some("Q2".to_string()),
        "第3季" => return Some("Q3".to_string()),
        "第4季" => return Some("Q4".to_string()),
        _ => {}
    }

    quarter_from_period(period_start, period_end)
}

/// 由所屬期間的起訖月份推算季別（`resolve_quarter` 的備援路徑）。
fn quarter_from_period(start: Option<NaiveDate>, end: Option<NaiveDate>) -> Option<String> {
    use chrono::Datelike;

    let (Some(start), Some(end)) = (start, end) else {
        return None;
    };

    match (start.month(), end.month()) {
        (1, 12) => Some(String::new()),
        (1, 6) => Some("H1".to_string()),
        (7, 12) => Some("H2".to_string()),
        (1, 3) => Some("Q1".to_string()),
        (4, 6) => Some("Q2".to_string()),
        (7, 9) => Some("Q3".to_string()),
        (10, 12) => Some("Q4".to_string()),
        _ => None,
    }
}

/// 解析金額欄位；空值或無法解析時視為 0。
///
/// 與除權息預告表不同，這裡的欄位是公司實際決議的分派金額，
/// 沒有「待公告」的情況（未決議的公司根本不會出現在這份資料裡），
/// 因此缺值以 0 處理是安全的。
fn parse_amount(value: &str) -> Decimal {
    util::text::parse_decimal(value.trim(), None).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 以真實回應 fixture 驗證解析結果。
    ///
    /// 涵蓋：年度／上半年／單季三種期別、盈餘配股、資本公積配息、民國年轉西元。
    #[test]
    fn test_parse_allotments_with_fixture() {
        // include_str! 的路徑相對於本檔案（mops/dividend_allotment.rs）→ mops/testdata/。
        const FIXTURE: &str = include_str!("testdata/dividend_allotment_t187ap45_L.json");
        let rows: Vec<DividendAllotmentRaw> =
            serde_json::from_str(FIXTURE).expect("fixture should parse");
        let result = parse_allotments(rows);

        assert_eq!(result.len(), 5);

        // 年度配息 + 配股：民國 114 → 西元 2025，季別為空字串。
        let annual = &result[0];
        assert_eq!(annual.stock_symbol, "1231");
        assert_eq!(annual.year_of_dividend, 2025);
        assert_eq!(annual.quarter, "");
        assert_eq!(annual.earnings_cash, dec!(1.5));
        assert_eq!(annual.earnings_stock, dec!(1.0));
        assert_eq!(annual.cash_dividend(), dec!(1.5));
        assert_eq!(annual.stock_dividend(), dec!(1.0));
        assert_eq!(annual.period_start, NaiveDate::from_ymd_opt(2025, 1, 1));
        assert_eq!(annual.period_end, NaiveDate::from_ymd_opt(2025, 12, 31));

        // 資本公積發放的現金要算進公積現金股利，不能記成盈餘配息。
        let reserve = result
            .iter()
            .find(|item| item.stock_symbol == "1101")
            .expect("1101 should exist");
        assert_eq!(reserve.earnings_cash, dec!(0));
        assert_eq!(reserve.capital_reserve_cash, dec!(0.8));
        assert_eq!(reserve.cash_dividend(), dec!(0.8));

        // 上半年配息 → H1。
        let half = result
            .iter()
            .find(|item| item.quarter == "H1")
            .expect("H1 should exist");
        assert_eq!(half.stock_symbol, "1315");
        assert_eq!(half.year_of_dividend, 2026);

        // 單季配息 → Q1／Q4 各一筆。
        assert!(result.iter().any(|item| item.quarter == "Q1"));
        assert!(result.iter().any(|item| item.quarter == "Q4"));
    }

    #[test]
    fn test_resolve_quarter_falls_back_to_period() {
        let start = NaiveDate::from_ymd_opt(2025, 4, 1);
        let end = NaiveDate::from_ymd_opt(2025, 6, 30);

        // 已知文字優先。
        assert_eq!(resolve_quarter("第3季", start, end).as_deref(), Some("Q3"));
        // 未知文字時改用期間推算。
        assert_eq!(resolve_quarter("其他", start, end).as_deref(), Some("Q2"));
        // 期間也不完整時必須是 None，不可退回代表「年度」的空字串。
        assert_eq!(resolve_quarter("其他", None, None), None);
    }

    #[test]
    fn test_parse_period() {
        let (start, end) = parse_period("1140101~1141231");
        assert_eq!(start, NaiveDate::from_ymd_opt(2025, 1, 1));
        assert_eq!(end, NaiveDate::from_ymd_opt(2025, 12, 31));

        let (start, end) = parse_period("");
        assert!(start.is_none());
        assert!(end.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();

        match visit().await {
            Err(why) => println!("取得上市公司股利分派情形失敗: {:?}", why),
            Ok(result) => {
                println!("股利分派情形 {} 筆", result.len());
                for item in result.iter().take(5) {
                    println!("{:?}", item);
                }
            }
        }
    }
}
