use anyhow::{anyhow, Result};
use scraper::{Html, Selector};

use crate::{
    internal::{
        crawler::twse,
        database::table::stock::extension::qualified_foreign_institutional_investor::QualifiedForeignInstitutionalInvestor,
    },
    util
};

/// 取得上櫃股票外資及陸資投資持股統計
pub async fn visit() -> Result<Vec<QualifiedForeignInstitutionalInvestor>> {
    let url = format!(
        "https://mops.{}/server-java/t13sa150_otc?&step=wh",
        twse::HOST,
    );
    let text = util::http::get_use_big5(&url).await?;
    let selector = Selector::parse("body > center > table:nth-child(1) > tbody > tr")
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;
    let document = Html::parse_document(text.as_str());
    let mut result = Vec::with_capacity(1024);

    for node in document.select(&selector) {
        let tds: Vec<String> = node.text().map(|v| v.to_string()).collect();
        if tds.len() != 23 {
            continue;
        }
        let qfii = QualifiedForeignInstitutionalInvestor::from(tds);
        result.push(qfii);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::result::Result::Ok;

    use crate::{
        internal::cache::SHARE,
        logging
    };

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 visit".to_string());

        match visit().await {
            Ok(qfiis) => {
                logging::debug_file_async(format!("qfiis:{:#?}", qfiis));
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because: {:?}", why));
            }
        }
        logging::debug_file_async("結束 visit".to_string());
    }
}
