//! NStock EPS crawler。
//!
//! 此模組會呼叫 NStock 的公開 API，整理為年度與季度兩種 EPS 資料模型，
//! 供後續補齊 ROE、ROA 與毛利率等財務欄位使用。

use anyhow::{anyhow, Result};
use rust_decimal::Decimal;
use serde_derive::{Deserialize, Serialize};

use crate::{
    declare::Quarter,
    util::{self, map::Keyable, text},
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
    let res = util::http::get_json::<EpsResponse>(&url).await?;
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

    Ok(Eps { quarters, years })
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
    if input.len() != 6 {
        return Err(anyhow!("input:{} is InvalidDigit", input));
    }

    let year = input[..4].parse::<i32>()?;
    let quarter = input[4..].parse::<u32>()?;

    Ok((year, quarter))
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());

        match visit("2330").await {
            Ok(e) => {
                dbg!(&e);
                logging::debug_file_async(format!("nstock : {:#?}", e));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
