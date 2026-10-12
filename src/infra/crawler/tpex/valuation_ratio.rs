//! # 上櫃個股日本益比與股價淨值比
//!
//! 來源：TPEx `www/zh-tw/afterTrading/peQryDate?date=YYYY/MM/DD&response=json`
//! （上櫃股票個股本益比、殖利率、股價淨值比依日期查詢），可查任意歷史日期。
//! 每日收盤流程用的 openapi `tpex_mainboard_peratio_analysis` 只給最新一天，
//! 所以上櫃的歷史本益比要從這裡回補。
//!
//! 回應是 `{"stat":"ok","tables":[{"fields":[…],"data":[[…]]}]}`；代號欄叫「股票代號」，
//! 公司名稱尾端有全形空白補齊，沒有值時是 `N/A`。非交易日 `data` 為空。

use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::{
    core::util,
    domain::quote::official_ratio::{OfficialRatio, parse_ratio},
    infra::crawler::tpex,
};

#[derive(Debug, Default, Deserialize)]
struct PeQryResponse {
    #[serde(default)]
    tables: Vec<PeQryTable>,
}

#[derive(Debug, Default, Deserialize)]
struct PeQryTable {
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    data: Vec<Vec<serde_json::Value>>,
}

/// 取得指定日期的上櫃個股本益比與股價淨值比；非交易日回傳空陣列。
///
/// # Errors
///
/// 請求失敗、回應不是預期的 JSON，或有資料卻缺少「股票代號」欄位時回傳錯誤。
pub async fn visit(date: NaiveDate) -> Result<Vec<OfficialRatio>> {
    let url = format!(
        "https://{host}/www/zh-tw/afterTrading/peQryDate?date={date}&response=json",
        host = tpex::HOST,
        date = date.format("%Y%%2F%m%%2F%d")
    );
    let response = util::http::get_json::<PeQryResponse>(&url).await?;
    parse(response)
}

/// 解析 peQryDate 回應。
fn parse(response: PeQryResponse) -> Result<Vec<OfficialRatio>> {
    let Some(table) = response.tables.into_iter().next() else {
        return Ok(Vec::new());
    };
    if table.data.is_empty() {
        return Ok(Vec::new());
    }
    let column = |name: &str| table.fields.iter().position(|field| field == name);
    let symbol = column("股票代號").ok_or_else(|| anyhow!("peQryDate 缺少「股票代號」欄位"))?;
    let (pe, pb) = (column("本益比"), column("股價淨值比"));

    Ok(table
        .data
        .iter()
        .filter_map(|row| {
            let text = |index: Option<usize>| {
                index
                    .and_then(|i| row.get(i))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
            };
            let ratio = OfficialRatio {
                stock_symbol: text(Some(symbol))?.to_owned(),
                price_earning_ratio: text(pe).and_then(parse_ratio),
                price_to_book_ratio: text(pb).and_then(parse_ratio),
            };
            (!ratio.stock_symbol.is_empty() && ratio.has_any()).then_some(ratio)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn fixture() -> PeQryResponse {
        let path = format!(
            "{}/src/infra/crawler/tpex/testdata/pe_qry_date_20200630.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("讀取 fixture"))
            .expect("解析 fixture")
    }

    /// 依欄名取本益比與淨值比；`N/A` 為沒有值。
    #[test]
    fn parse_fixture() {
        let ratios = parse(fixture()).expect("可解析");
        let farm = ratios
            .iter()
            .find(|ratio| ratio.stock_symbol == "1240")
            .expect("有 1240");
        assert_eq!(farm.price_earning_ratio, Some(dec!(14.25)));
        assert_eq!(farm.price_to_book_ratio, Some(dec!(1.87)));
        let loss = ratios
            .iter()
            .find(|ratio| ratio.stock_symbol == "1258")
            .expect("有 1258");
        assert_eq!(loss.price_earning_ratio, None);
        assert_eq!(loss.price_to_book_ratio, Some(dec!(0.85)));
    }

    /// 沒有 table 或沒有資料（非交易日）回空陣列；有資料卻缺代號欄位回錯誤。
    #[test]
    fn parse_empty_and_bad_layout() {
        assert!(parse(PeQryResponse::default()).expect("可解析").is_empty());
        let holiday = PeQryResponse {
            tables: vec![PeQryTable::default()],
        };
        assert!(parse(holiday).expect("可解析").is_empty());

        let renamed = PeQryResponse {
            tables: vec![PeQryTable {
                fields: vec!["代號".to_owned()],
                data: vec![vec!["6488".into()]],
            }],
        };
        assert!(parse(renamed).is_err());
    }

    /// 實網：取 2020-06-30 的資料，確認端點與欄位名稱仍可用。
    #[tokio::test]
    #[ignore = "連線外部網站"]
    async fn test_visit_live() {
        dotenvy::dotenv().ok();
        let date = NaiveDate::from_ymd_opt(2020, 6, 30).expect("有效日期");
        let ratios = visit(date).await.expect("可取得");
        assert!(ratios.len() > 500, "{}", ratios.len());
        assert!(ratios.iter().any(|ratio| ratio.stock_symbol == "6488"));
    }
}
