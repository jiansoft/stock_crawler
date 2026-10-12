//! # 上市個股日本益比與股價淨值比
//!
//! 來源：TWSE `exchangeReport/BWIBBU_d?response=json&date=YYYYMMDD&selectType=ALL`
//! （個股日本益比、殖利率及股價淨值比），可查任意歷史日期，2012 年起實測可用。
//!
//! **欄位順序隨年份不同**：2012 年是「證券代號、證券名稱、本益比、殖利率、股價淨值比」，
//! 2020 年起變成「證券代號、證券名稱、收盤價、殖利率、股利年度、本益比、股價淨值比、
//! 財報年/季」，所以一律依 `fields` 的欄名找位置。虧損股的本益比是 `-`。
//! 非交易日 `stat` 不是 `OK`，視為沒有資料。

use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::{
    core::util,
    domain::quote::official_ratio::{OfficialRatio, parse_ratio},
    infra::crawler::twse,
};

/// BWIBBU_d 的回應主體；資料列混有字串與數字（股利年度），以 JSON 值接收。
#[derive(Debug, Default, Deserialize)]
struct BwibbuResponse {
    #[serde(default)]
    stat: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    data: Vec<Vec<serde_json::Value>>,
}

/// 取得指定日期的上市個股本益比與股價淨值比；非交易日回傳空陣列。
///
/// # Errors
///
/// 請求失敗、回應不是預期的 JSON，或缺少「證券代號」欄位時回傳錯誤。
pub async fn visit(date: NaiveDate) -> Result<Vec<OfficialRatio>> {
    let url = format!(
        "https://www.{host}/exchangeReport/BWIBBU_d?response=json&date={date}&selectType=ALL",
        host = twse::HOST,
        date = date.format("%Y%m%d")
    );
    let response = util::http::get_json::<BwibbuResponse>(&url).await?;
    parse(response)
}

/// 解析 BWIBBU_d 回應。
fn parse(response: BwibbuResponse) -> Result<Vec<OfficialRatio>> {
    if response.stat != "OK" {
        return Ok(Vec::new());
    }
    let column = |name: &str| response.fields.iter().position(|field| field == name);
    let symbol = column("證券代號").ok_or_else(|| anyhow!("BWIBBU_d 缺少「證券代號」欄位"))?;
    let (pe, pb) = (column("本益比"), column("股價淨值比"));

    Ok(response
        .data
        .iter()
        .filter_map(|row| {
            let text = |index: Option<usize>| index.and_then(|i| row.get(i)).map(cell_text);
            let ratio = OfficialRatio {
                stock_symbol: text(Some(symbol))?.trim().to_owned(),
                price_earning_ratio: text(pe).as_deref().and_then(parse_ratio),
                price_to_book_ratio: text(pb).as_deref().and_then(parse_ratio),
            };
            (!ratio.stock_symbol.is_empty() && ratio.has_any()).then_some(ratio)
        })
        .collect())
}

/// JSON 儲存格轉字串（數字欄位如股利年度也可能出現）。
fn cell_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn fixture(name: &str) -> BwibbuResponse {
        let path = format!(
            "{}/src/infra/crawler/twse/testdata/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("讀取 fixture"))
            .expect("解析 fixture")
    }

    /// 2020 年起的欄位順序：本益比在第 6 欄、股價淨值比在第 7 欄。
    #[test]
    fn parse_2020_layout() {
        let ratios = parse(fixture("bwibbu_d_20200630.json")).expect("可解析");
        let tsmc = ratios
            .iter()
            .find(|ratio| ratio.stock_symbol == "2330")
            .expect("有 2330");
        assert_eq!(tsmc.price_earning_ratio, Some(dec!(20.25)));
        assert_eq!(tsmc.price_to_book_ratio, Some(dec!(4.84)));
        let cement = ratios
            .iter()
            .find(|ratio| ratio.stock_symbol == "1101")
            .expect("有 1101");
        assert_eq!(cement.price_earning_ratio, Some(dec!(10.40)));
    }

    /// 2012 年的欄位順序：本益比在第 3 欄；虧損股本益比為 `-`，淨值比照樣有值。
    #[test]
    fn parse_2012_layout_and_loss_maker() {
        let ratios = parse(fixture("bwibbu_d_20120102.json")).expect("可解析");
        assert_eq!(ratios.len(), 3);
        let auo = ratios
            .iter()
            .find(|ratio| ratio.stock_symbol == "2409")
            .expect("有 2409");
        assert_eq!(auo.price_earning_ratio, None);
        assert_eq!(auo.price_to_book_ratio, Some(dec!(0.50)));
        let tsmc = ratios
            .iter()
            .find(|ratio| ratio.stock_symbol == "2330")
            .expect("有 2330");
        assert_eq!(tsmc.price_earning_ratio, Some(dec!(13.56)));
    }

    /// 非交易日回空陣列；缺少代號欄位是格式變更，回錯誤；兩個倍數都沒有的列略過。
    #[test]
    fn parse_holiday_and_bad_layout() {
        let holiday = BwibbuResponse {
            stat: "很抱歉，沒有符合條件的資料!".to_owned(),
            ..Default::default()
        };
        assert!(parse(holiday).expect("非交易日不是錯誤").is_empty());

        let renamed = BwibbuResponse {
            stat: "OK".to_owned(),
            fields: vec!["代號".to_owned()],
            data: vec![vec!["1101".into()]],
        };
        assert!(parse(renamed).is_err());

        let empty_row = BwibbuResponse {
            stat: "OK".to_owned(),
            fields: vec![
                "證券代號".to_owned(),
                "本益比".to_owned(),
                "股價淨值比".to_owned(),
            ],
            data: vec![
                vec!["9999".into(), "-".into(), "-".into()],
                vec![" ".into(), "3".into(), "1".into()],
            ],
        };
        assert!(parse(empty_row).expect("可解析").is_empty());
    }

    /// 實網：取 2020-06-30 的資料，確認端點與欄位名稱仍可用。
    #[tokio::test]
    #[ignore = "連線外部網站"]
    async fn test_visit_live() {
        dotenvy::dotenv().ok();
        let date = NaiveDate::from_ymd_opt(2020, 6, 30).expect("有效日期");
        let ratios = visit(date).await.expect("可取得");
        assert!(ratios.len() > 500, "{}", ratios.len());
        assert!(ratios.iter().any(|ratio| ratio.stock_symbol == "2330"));
    }
}
