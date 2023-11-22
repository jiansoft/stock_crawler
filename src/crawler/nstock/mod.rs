use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{self, Value};

use crate::{logging, util};

pub mod price;

const HOST: &str = "www.nstock.tw";

pub struct NStock {}

#[derive(Serialize, Deserialize, Debug)]
struct EPSRecord {
    年季: String,
    #[serde(rename = "公告基本每股盈餘(元)")]
    公告基本每股盈餘元: Value,
    #[serde(rename = "稅後權益報酬率(%)")]
    稅後權益報酬率百分比: Value,
    #[serde(rename = "稅後權益報酬率(%)")]
    稅後資產報酬率: Value,
}

#[derive(Serialize, Deserialize, Debug)]
struct Struct2 {
    #[serde(rename = "年度")]
    pub __field: String,
    #[serde(rename = "公告基本每股盈餘(元)")]
    pub ___________field: String,
    #[serde(rename = "稅後權益報酬率(%)")]
    pub __________field: String,
    #[serde(rename = "稅後資產報酬率(%)")]
    pub __________field_0: String,
    #[serde(rename = "公告基本每股盈餘年成長(%)")]
    pub ______________field: Option<String>,
    #[serde(rename = "公告基本每股盈餘年成長2(%)")]
    pub ___________2: Option<String>,
    #[serde(rename = "年收盤價")]
    pub ____field: Option<String>,
    #[serde(rename = "年營業利益率(％)")]
    pub _________field: String,
    #[serde(rename = "年營收(億)")]
    pub ______field: String,
    #[serde(rename = "年成長(％)")]
    pub ______field_0: String,
    #[serde(rename = "年毛利率(％)")]
    pub _______field: String,
    #[serde(rename = "年稅後淨利(億)")]
    pub ________field: String,
    #[serde(rename = "年稅後淨利率(％)")]
    pub _________field_0: String,
    #[serde(rename = "本業佔比")]
    pub ____field_0: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Struct1 {
    #[serde(rename = "年季")]
    pub __field: String,
    #[serde(rename = "公告基本每股盈餘(元)")]
    pub ___________field: String,
    #[serde(rename = "稅後權益報酬率(%)")]
    pub __________field: String,
    #[serde(rename = "稅後資產報酬率(%)")]
    pub __________field_0: String,
    #[serde(rename = "累計EPS")]
    pub __eps: String,
    #[serde(rename = "EPS年增率")]
    pub eps: String,
    #[serde(rename = "公告基本每股盈餘年成長(%)")]
    pub ______________field: Option<String>,
    #[serde(rename = "公告基本每股盈餘年成長2(%)")]
    pub ___________2: Option<String>,
    #[serde(rename = "季收盤價")]
    pub ____field: Option<String>,
    #[serde(rename = "單季營業利益率(％)")]
    pub __________field_1: String,
    #[serde(rename = "季營收(億)")]
    pub ______field: String,
    #[serde(rename = "單季年成長(％)")]
    pub ________field: String,
    #[serde(rename = "單季毛利率(％)")]
    pub ________field_0: String,
    #[serde(rename = "單季稅後淨利(億)")]
    pub _________field: String,
    #[serde(rename = "單季稅後淨利率(％)")]
    pub __________field_2: String,
    #[serde(rename = "本業佔比")]
    pub ____field_0: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct EpsData {
    #[serde(rename = "股票代號")]
    pub stock_symbol: String,
    #[serde(rename = "季度EPS")]
    pub quarters: Vec<Struct1>,
    #[serde(rename = "年度EPS")]
    pub years: Vec<Struct2>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Eps {
    pub data: Vec<EpsData>,
}

pub async fn visit(stock_symbol: &str) -> Result<()> {
    let url = format!(
        "https://www.nstock.tw/api/v2/eps/data?stock_id={stock_symbol}",
        stock_symbol = stock_symbol
    );
    let text = util::http::get_use_json::<Eps>(&url).await?;
    dbg!(text);

    // logging::debug_file_async(format!("{}", &text));

    //let re_eps_seasons = Regex::new(r#""EPS_SEASONS":\s*(\[[^\]]*\])"#)?;
    //let eps_years_re = Regex::new(r#"("EPS_YEARS"\s*:\s*\[.*?\])"#).unwrap();
    /* let re = Regex::new(r#"EPS_SEASONS:(\[{.*}\])"#)?;

    if let Some(caps) = re.captures(&text) {
        println!("EPS_SEASONS: {}", &caps[1]);
    } else {
        println!("No match found");
    }*/

    /*if let Some(caps) = re.captures(&text) {
            println!("EPS_SEASONS: {:#?}", &caps);
        } else {
            println!("No match found");
        }
    */

    Ok(())
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
