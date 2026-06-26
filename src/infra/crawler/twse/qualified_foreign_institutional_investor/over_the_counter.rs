use anyhow::{Result, anyhow};
use scraper::{Html, Selector};

use crate::{
    core::util::{self, convert::FromValue},
    infra::crawler::{share::QfiiDto, twse},
};

/// 取得上櫃股票外資及陸資投資持股統計
pub async fn visit() -> Result<Vec<QfiiDto>> {
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
        let stock_symbol = tds[1].get_string(None);
        let issued_share = tds[5].get_i64(None);
        let shares_held = tds[9].get_i64(None);
        let share_holding_percentage = tds[13].get_decimal(Some(vec!['\u{a0}']));

        result.push(QfiiDto {
            stock_symbol,
            issued_share,
            shares_held,
            share_holding_percentage,
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::result::Result::Ok;

    use crate::{core::logging, infra::cache::SHARE};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 visit");

        match visit().await {
            Ok(qfiis) => {
                dbg!(&qfiis);
                tracing::debug!("qfiis:{:#?}", qfiis);
            }
            Err(why) => {
                tracing::debug!("Failed to visit because: {:?}", why);
            }
        }
        tracing::debug!("結束 visit");
    }
}
