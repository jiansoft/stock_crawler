//! # TWSE 上市除權除息計算結果表採集器
//!
//! 資料來源為證交所 `rwd/zh/exRight/TWT49U`（網頁「除權除息計算結果表」）。與
//! [`super::ex_dividend_announcement`] 的預告表不同，這裡是**已經除權息**的實際結果：
//! 交易所依實際除權息日計算參考價，日期與息值是最可靠的來源。
//!
//! 支援 `startDate`／`endDate` 日期區間，一次請求就能取回一整年（2025 年 1,358 筆）。
//!
//! ## 欄位注意事項
//!
//! - 「權/息」為 `息` 時，「權值+息值」就是每股現金股利。
//! - `權`、`權息` 只給以價格計的合併值，拆不出現金與股票股利各多少，金額一律留 `None`，
//!   只拿日期與類別比對；`權` 也可能是現金增資而不是配股。

use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::{
    core::{declare::StockExchangeMarket, util::http, util::text},
    infra::crawler::{
        share::{ExDividendAnnouncement, parse_ex_dividend_kind},
        twse,
    },
};

/// `TWT49U` 的回應主體。
#[derive(Deserialize, Debug)]
pub struct Twt49uResponse {
    /// 查詢狀態；成功為 `OK`。
    pub stat: Option<String>,
    /// 資料列：資料日期、股票代號、股票名稱、除權息前收盤價、除權息參考價、權值+息值、權/息……
    pub data: Option<Vec<Vec<String>>>,
}

/// 資料列欄位索引。
const COL_DATE: usize = 0;
const COL_CODE: usize = 1;
const COL_NAME: usize = 2;
const COL_VALUE: usize = 5;
const COL_KIND: usize = 6;

/// 取得 `start`～`end`（含）之間已除權息的上市證券。
///
/// # 錯誤
///
/// HTTP 請求、JSON 反序列化失敗，或 `stat` 不是 `OK` 也不是「查無資料」時回傳錯誤；
/// 不把來源異常當成「沒有除權息」，否則核對流程會以為資料庫多出事件。
pub async fn visit(start: NaiveDate, end: NaiveDate) -> Result<Vec<ExDividendAnnouncement>> {
    let url = format!(
        "https://www.{}/rwd/zh/exRight/TWT49U?response=json&startDate={}&endDate={}",
        twse::HOST,
        start.format("%Y%m%d"),
        end.format("%Y%m%d")
    );
    let response = http::get_json::<Twt49uResponse>(&url)
        .await
        .with_context(|| format!("Failed to fetch TWT49U for {start}~{end}"))?;
    parse_results(&response)
}

/// 將回應轉成除權息事件；單一資料列格式不符只略過該列。
pub fn parse_results(response: &Twt49uResponse) -> Result<Vec<ExDividendAnnouncement>> {
    match response.stat.as_deref() {
        Some("OK") => {}
        Some(stat) if stat.contains("沒有符合條件") => return Ok(Vec::new()),
        other => anyhow::bail!("TWT49U 回應狀態異常：{other:?}"),
    }

    let rows = response.data.as_deref().unwrap_or_default();
    Ok(rows
        .iter()
        .filter(|row| row.len() > COL_KIND)
        .filter_map(|row| {
            let ex_date = parse_roc_full_date(&row[COL_DATE])?;
            let kind = row[COL_KIND].trim();
            let (is_cash, is_stock) = parse_ex_dividend_kind(kind);
            if !is_cash && !is_stock {
                return None;
            }
            // 只有純除息時，合併值才等於現金股利。
            let cash_dividend = (kind == "息")
                .then(|| text::parse_decimal(&row[COL_VALUE], Some(vec![','])).ok())
                .flatten()
                .map(|value| value.round_dp(4));
            Some(ExDividendAnnouncement {
                stock_symbol: row[COL_CODE].trim().to_string(),
                name: row[COL_NAME].trim().to_string(),
                ex_date,
                is_cash,
                is_stock,
                cash_dividend,
                stock_dividend_ratio: None,
                market: StockExchangeMarket::Listed,
            })
        })
        .collect())
}

/// 解析 `114年01月02日` 形式的民國日期。
fn parse_roc_full_date(raw: &str) -> Option<NaiveDate> {
    let digits: Vec<i32> = raw
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();
    let [year, month, day] = digits[..] else {
        return None;
    };
    if !(1..=200).contains(&year) {
        return None;
    }
    NaiveDate::from_ymd_opt(
        year + 1911,
        u32::try_from(month).ok()?,
        u32::try_from(day).ok()?,
    )
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn fixture() -> Twt49uResponse {
        serde_json::from_str(include_str!("testdata/twt49u_2025_sample.json"))
            .expect("fixture 應為合法 JSON")
    }

    #[test]
    fn parse_results_maps_cash_only_events_with_amounts() {
        let results = parse_results(&fixture()).expect("解析成功");
        assert_eq!(results.len(), 5);

        let first = &results[0];
        assert_eq!(first.stock_symbol, "00730");
        assert_eq!(first.ex_date, NaiveDate::from_ymd_opt(2025, 1, 17).unwrap());
        assert!(first.is_cash && !first.is_stock);
        assert_eq!(first.cash_dividend, Some(dec!(0.086)));
        assert_eq!(first.market, StockExchangeMarket::Listed);
    }

    /// 「權」與「權息」拆不出現金與股票股利，金額留 None，只保留類別。
    #[test]
    fn rights_events_keep_the_kind_but_no_amounts() {
        let results = parse_results(&fixture()).expect("解析成功");
        let rights = results
            .iter()
            .find(|r| r.stock_symbol == "6658")
            .expect("6658");
        assert!(!rights.is_cash && rights.is_stock);
        assert_eq!(rights.cash_dividend, None);

        let both = results
            .iter()
            .find(|r| r.stock_symbol == "1605")
            .expect("1605");
        assert!(both.is_cash && both.is_stock);
        assert_eq!(both.cash_dividend, None);
    }

    #[test]
    fn empty_result_and_abnormal_status() {
        let empty = Twt49uResponse {
            stat: Some("很抱歉，沒有符合條件的資料!".to_string()),
            data: None,
        };
        assert!(parse_results(&empty).expect("查無資料不是錯誤").is_empty());

        let broken = Twt49uResponse {
            stat: Some("查詢日期大於今日，請重新查詢!".to_string()),
            data: None,
        };
        assert!(parse_results(&broken).is_err());
        assert!(
            parse_results(&Twt49uResponse {
                stat: None,
                data: None
            })
            .is_err()
        );
    }

    #[test]
    fn parse_roc_full_date_handles_the_minguo_format() {
        assert_eq!(
            parse_roc_full_date("114年01月02日"),
            NaiveDate::from_ymd_opt(2025, 1, 2)
        );
        assert_eq!(parse_roc_full_date("2025年01月02日"), None);
        assert_eq!(parse_roc_full_date("114年13月02日"), None);
        assert_eq!(parse_roc_full_date("--"), None);
    }
}
