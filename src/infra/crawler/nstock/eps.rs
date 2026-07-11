//! NStock EPS crawler。
//!
//! 此模組會呼叫 NStock 的公開 API，整理為年度與季度兩種 EPS 資料模型，
//! 供後續補齊 ROE、ROA 與毛利率等財務欄位使用。

use anyhow::{Result, anyhow};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    core::declare::Quarter,
    core::util::{self, map::Keyable, text},
};

#[derive(Serialize, Deserialize, Debug)]
struct EpsDataYear {
    #[serde(rename = "年度")]
    pub year: String,
    #[serde(rename = "公告基本每股盈餘(元)")]
    pub eps: String,
    #[serde(rename = "稅後權益報酬率(%)")]
    pub roe: String,
    #[serde(rename = "稅後資產報酬率(%)")]
    pub roa: String,
    #[serde(rename = "年營業利益率(％)")]
    pub operating_profit_margin: String,
    #[serde(rename = "年毛利率(％)")]
    pub gross_profit: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct EpsDataQuarter {
    #[serde(rename = "年季")]
    pub year_and_quarter: String,
    #[serde(rename = "公告基本每股盈餘(元)")]
    pub eps: String,
    #[serde(rename = "稅後權益報酬率(%)")]
    pub roe: String,
    #[serde(rename = "稅後資產報酬率(%)")]
    pub roa: String,
    #[serde(rename = "累計EPS")]
    pub cumulative_eps: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct EpsData {
    /*#[serde(rename = "股票代號")]
    pub stock_symbol: String,*/
    #[serde(rename = "季度EPS")]
    pub quarters: Vec<EpsDataQuarter>,
    #[serde(rename = "年度EPS")]
    pub years: Vec<EpsDataYear>,
}

#[derive(Serialize, Deserialize, Debug)]
struct EpsResponse {
    pub data: Vec<EpsData>,
}

#[derive(Serialize, Deserialize, Debug)]
/// NStock 回傳的單季 EPS 資料。
pub struct EpsQuarter {
    /// 股票代號。
    pub stock_symbol: String,
    /// 財報年度。
    pub year: i32,
    /// 財報季度。
    pub quarter: Quarter,
    /// 單季 EPS。
    pub eps: Decimal,
    /// 稅後股東權益報酬率。
    pub roe: Decimal,
    /// 稅後資產報酬率。
    pub roa: Decimal,
    /// 累計 EPS。
    pub cumulative_eps: Decimal,
}

impl Keyable for EpsQuarter {
    fn key(&self) -> String {
        format!("{}-{}-{}", self.stock_symbol, self.year, self.quarter)
    }

    fn key_with_prefix(&self) -> String {
        format!("EpsQuarter:{}", self.key())
    }
}

#[derive(Serialize, Deserialize, Debug)]
/// NStock 回傳的年度 EPS 資料。
pub struct EpsYear {
    /// 股票代號。
    pub stock_symbol: String,
    /// 財報年度。
    pub year: i32,
    /// 年度 EPS。
    pub eps: Decimal,
    /// 稅後股東權益報酬率。
    pub roe: Decimal,
    /// 稅後資產報酬率。
    pub roa: Decimal,
    /// 年營業利益率。
    pub operating_profit_margin: Decimal,
    /// 年毛利率。
    pub gross_profit: Decimal,
}

impl Keyable for EpsYear {
    fn key(&self) -> String {
        format!("{}-{}-", self.stock_symbol, self.year)
    }

    fn key_with_prefix(&self) -> String {
        format!("EpsYear:{}", self.key())
    }
}

#[derive(Serialize, Deserialize, Debug)]
/// NStock EPS API 的整體回傳結果。
pub struct Eps {
    /*  pub stock_symbol: String,*/
    /// 季度 EPS 清單。
    pub quarters: Vec<EpsQuarter>,
    /// 年度 EPS 清單。
    pub years: Vec<EpsYear>,
}

/// 向 NStock 取得指定股票的 EPS 資料。
///
/// 會同時整理年度與季度資料，並轉成專案內使用的數值型別。
///
/// # 參數
///
/// * `stock_symbol` - 股票代號
///
/// # 錯誤
///
/// 當 HTTP 請求失敗或回應 JSON 無法解析時回傳錯誤。
pub async fn visit(stock_symbol: &str) -> Result<Eps> {
    let url = format!(
        "https://www.nstock.tw/api/v2/eps/data?stock_id={stock_symbol}",
        stock_symbol = stock_symbol
    );
    // fetch 與 parse 分離：這裡只負責打 API 拿回 DTO（EpsResponse），
    // DTO → 專案數值模型的轉換交給下面的純函式 parse_eps_response，
    // 讓轉換邏輯可以用 testdata/ 的 JSON fixture 做離線單元測試。
    let res = util::http::get_json::<EpsResponse>(&url).await?;
    Ok(parse_eps_response(res, stock_symbol))
}

/// 把 NStock API 的原始回應（DTO）轉換成專案內的 [`Eps`] 模型（純函式）。
///
/// ## 為什麼抽成純函式
///
/// API 回傳的欄位都是字串（例如 EPS 是 `"32.34"`、年季是 `"202301"`），
/// 而專案內需要的是 `Decimal`、`i32` 等強型別。這段「字串 → 型別」的轉換
/// 是最容易因 API 改版而壞掉的部分，抽成不做任何 I/O 的純函式後，
/// 就能用 `testdata/eps_response.json` 這種 fixture 做穩定的離線測試。
///
/// ## 轉換規則
///
/// - 年度與季度分開處理，各自 `filter_map`：**單筆**解析失敗（例如
///   欄位是 `"N/A"` 或年季格式錯誤）只丟棄該筆，不影響其他筆——
///   歷史久遠的資料常有缺值，一筆壞資料不應讓整檔股票的資料進不來。
fn parse_eps_response(res: EpsResponse, stock_symbol: &str) -> Eps {
    let years = res
        .data
        .iter()
        .flat_map(|item| item.years.iter())
        .filter_map(|edy| parse_eps_year(stock_symbol.to_string(), edy))
        .collect();
    let quarters = res
        .data
        .iter()
        .flat_map(|item| item.quarters.iter())
        .filter_map(|edq| parse_eps_quarter(stock_symbol.to_string(), edq))
        .collect();

    Eps { quarters, years }
}

fn parse_eps_year(stock_symbol: String, eps_year: &EpsDataYear) -> Option<EpsYear> {
    Some(EpsYear {
        stock_symbol,
        year: text::parse_i32(&eps_year.year, None).ok()?,
        eps: text::parse_decimal(&eps_year.eps, None).ok()?,
        roe: text::parse_decimal(&eps_year.roe, None).ok()?,
        roa: text::parse_decimal(&eps_year.roa, None).ok()?,
        operating_profit_margin: text::parse_decimal(&eps_year.operating_profit_margin, None)
            .ok()?,
        gross_profit: text::parse_decimal(&eps_year.gross_profit, None).ok()?,
    })
}

fn parse_eps_quarter(stock_symbol: String, eps_quarter: &EpsDataQuarter) -> Option<EpsQuarter> {
    let (year, quarter_serial) = parse_year_and_quarter(&eps_quarter.year_and_quarter).ok()?;
    let quarter = Quarter::from_serial(quarter_serial)?;

    Some(EpsQuarter {
        stock_symbol,
        year,
        quarter,
        eps: text::parse_decimal(&eps_quarter.eps, None).ok()?,
        roe: text::parse_decimal(&eps_quarter.roe, None).ok()?,
        roa: text::parse_decimal(&eps_quarter.roa, None).ok()?,
        cumulative_eps: text::parse_decimal(&eps_quarter.cumulative_eps, None).ok()?,
    })
}

fn parse_year_and_quarter(input: &str) -> Result<(i32, u32)> {
    // 先驗證「長度為 6 且全部是 ASCII 數字」再切片。
    // [..4] 這種 byte index 切片要求索引落在 UTF-8 字元邊界上，外部 API 若
    // 回傳中文等多位元組字元（len() 仍可能等於 6），切在字元中間會直接 panic。
    // 全 ASCII 數字保證每個字元恰好 1 byte，之後的固定位置切片就絕對安全。
    if input.len() != 6 || !input.bytes().all(|b| b.is_ascii_digit()) {
        return Err(anyhow!("input:{} is InvalidDigit", input));
    }

    let year = input[..4].parse::<i32>()?;
    let quarter = input[4..].parse::<u32>()?;

    Ok((year, quarter))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 驗證年季字串正常解析。
    #[test]
    fn test_parse_year_and_quarter_parses_valid_input() {
        assert_eq!(parse_year_and_quarter("202301").unwrap(), (2023, 1));
        assert_eq!(parse_year_and_quarter("202504").unwrap(), (2025, 4));
    }

    /// 驗證含多位元組字元的輸入回傳錯誤而不是 panic。
    ///
    /// 「台台」恰好是 6 bytes，舊版只檢查 len() == 6 就做 `[..4]` 切片，
    /// byte 4 落在第二個字的中間會直接 panic。
    #[test]
    fn test_parse_year_and_quarter_rejects_multibyte_input() {
        assert!(parse_year_and_quarter("台台").is_err()); // 6 bytes，切 ..4 會跨字元
        assert!(parse_year_and_quarter("２０２３01").is_err()); // 全形數字
    }

    /// 驗證長度不符或含非數字的輸入回傳錯誤。
    #[test]
    fn test_parse_year_and_quarter_rejects_invalid_input() {
        assert!(parse_year_and_quarter("").is_err());
        assert!(parse_year_and_quarter("2023Q1").is_err()); // 含英文字母
        assert!(parse_year_and_quarter("20231").is_err()); // 只有 5 碼
    }

    // === JSON fixture 測試（離線、不連網路）===
    //
    // fixture 內容位於 testdata/eps_response.json，是手工構造的代表性樣本
    // （非真實數據），涵蓋正常資料與 API 常見的髒資料（"N/A"、格式錯的年季）。
    // 這是本專案「crawler fixture 測試模式」的 JSON 版示範：
    // 1. serde 反序列化 fixture（同時驗證中文欄位名的 rename 對應仍正確）。
    // 2. 呼叫純函式 parse_eps_response 驗證 DTO → 專案模型的轉換規則。

    /// 驗證 fixture 中的正常資料被完整轉換、髒資料被逐筆丟棄。
    #[test]
    fn parse_eps_response_filters_dirty_rows_from_fixture() {
        // include_str! 相對於本檔案（nstock/eps.rs）→ nstock/testdata/。
        const FIXTURE: &str = include_str!("testdata/eps_response.json");

        // 第一步：反序列化。若站方欄位改名（例如「年季」改「年/季」），
        // 這裡就會失敗，等於順帶測到 serde rename 的對應。
        let res: EpsResponse = serde_json::from_str(FIXTURE).expect("fixture 應可反序列化");

        // 第二步：轉換。fixture 有 4 筆季度（1 筆年季格式錯、1 筆 EPS 為 N/A）
        // 與 3 筆年度（1 筆年度為 N/A）——髒資料應被逐筆丟棄，不影響其他筆。
        let eps = parse_eps_response(res, "2330");

        assert_eq!(eps.quarters.len(), 2, "quarters: {:#?}", eps.quarters);
        assert_eq!(eps.quarters[0].year, 2023);
        assert_eq!(eps.quarters[0].quarter, Quarter::Q1);
        assert_eq!(eps.quarters[0].eps, rust_decimal_macros::dec!(7.98));
        assert_eq!(
            eps.quarters[1].cumulative_eps,
            rust_decimal_macros::dec!(14.99)
        );

        assert_eq!(eps.years.len(), 2, "years: {:#?}", eps.years);
        assert_eq!(eps.years[0].year, 2023);
        assert_eq!(eps.years[0].eps, rust_decimal_macros::dec!(32.34));
        assert_eq!(eps.years[1].gross_profit, rust_decimal_macros::dec!(59.56));

        // Keyable 的鍵格式順帶驗證（快取層以此當 Redis/記憶體鍵）。
        assert_eq!(eps.quarters[0].key(), "2330-2023-Q1");
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 visit");

        match visit("2330").await {
            Ok(e) => {
                dbg!(&e);
                tracing::debug!("nstock : {:#?}", e);
            }
            Err(why) => {
                tracing::debug!("Failed to visit because {:?}", why);
            }
        }

        tracing::debug!("結束 visit");
    }
}
