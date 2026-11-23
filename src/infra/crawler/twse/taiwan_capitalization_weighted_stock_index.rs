use crate::core::util::datetime::TaipeiDateTime;
use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{core::util, infra::crawler::twse};

/// 調用台股指數 twse API 後其回應的數據
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
//#[serde(rename_all = "camelCase")]
pub struct TaiwanStockIndexDto {
    /// API 回應狀態。
    pub stat: String,
    /// 查詢日期。
    pub date: Option<String>,
    /// 回應標題。
    pub title: Option<String>,
    /// 欄位名稱清單。
    pub fields: Option<Vec<String>>,
    /// 原始指數資料列。
    pub data: Option<Vec<Vec<String>>>,
}

/// 取得台股指數
pub async fn visit(date: TaipeiDateTime) -> Result<TaiwanStockIndexDto> {
    let url = format!(
        "https://www.{}/exchangeReport/FMTQIK?response=json&date={}&_={}",
        twse::HOST,
        date.strftime("%Y%m%d"),
        date.timestamp().as_millisecond()
    );

    util::http::get_json::<TaiwanStockIndexDto>(&url).await
}

#[cfg(test)]
mod tests;
