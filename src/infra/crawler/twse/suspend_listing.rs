use anyhow::Result;
use serde::Deserialize;

use crate::{core::util, infra::crawler::twse};

/// 調用 twse suspendListingCsvAndHtml API 後其回應的數據
#[derive(Default, Debug, Clone, PartialEq, Deserialize)]
pub struct SuspendListing {
    #[serde(rename(deserialize = "DelistingDate"))]
    /// 下市日期。
    pub delisting_date: String,
    #[serde(rename(deserialize = "Company"))]
    /// 公司名稱。
    pub name: String,
    #[serde(rename(deserialize = "Code"))]
    /// 股票代號。
    pub stock_symbol: String,
}

/// 取得終止上市公司名單
pub async fn visit() -> Result<Vec<SuspendListing>> {
    let url = format!(
        "https://openapi.{}/v1/company/suspendListingCsvAndHtml",
        twse::HOST,
    );

    util::http::get_json::<Vec<SuspendListing>>(&url).await
}

#[cfg(test)]
mod tests;
