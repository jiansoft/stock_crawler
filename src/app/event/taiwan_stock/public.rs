use std::fmt::Write;

use anyhow::Result;
use chrono::Local;
use rust_decimal::{Decimal, prelude::ToPrimitive};
use rust_decimal_macros::dec;

use crate::domain::quote::repository::QuoteRepository;
// 通知走 core::alert（port），跳脫工具走 core::util::text——
// app 層不 import interfaces::bot，維持「外層依賴內層」的合法方向。
use crate::{
    core::alert,
    core::declare,
    core::util::{map::Keyable, text},
    infra::database::repository::quote::PgQuoteRepository,
};

/// 提醒目前可公開申購的股票。
pub async fn execute() -> Result<()> {
    let ps = crate::infra::crawler::twse::public::visit().await?;
    let mut msg = String::with_capacity(2048);
    let now = Local::now().date_naive();

    for stock in ps {
        if stock.market == "中央登錄公債" {
            continue;
        }

        if let (Some(start), Some(end)) = (stock.offering_start_date, stock.offering_end_date)
            && now >= start
            && now <= end
        {
            let cache_key = stock.key_with_prefix();
            let is_jump = crate::infra::nosql::redis::CLIENT
                .get_bool(&cache_key)
                .await?;

            if is_jump {
                continue;
            }

            // 實例化報價倉儲，並獲取個股最新收盤價（封裝雙層快取策略）
            let quote_repo = PgQuoteRepository::new();
            let stock_last_price = quote_repo.fetch_last_quote(&stock.stock_symbol).await?;
            let last_price = match &stock_last_price {
                None => String::from(" - "),
                Some(last_quote) => match last_quote.closing_price.to_f64() {
                    None => String::from(" - "),
                    Some(price) => price.to_string(),
                },
            };
            // 直接取報價的數值；新股還沒有收盤價時是 0，不經過 `" - "` 字串再解析（那會記一筆 warn）。
            let last_price_dec = stock_last_price
                .as_ref()
                .map(|last_quote| last_quote.closing_price)
                .unwrap_or(Decimal::ZERO);
            let offering_price = stock.offering_price.unwrap_or(Decimal::ZERO);
            let price_change = calculate_price_change(offering_price, last_price_dec);
            let _ = writeln!(
                &mut msg,
                "{stock_symbol} {stock_name} 起迄日︰{start}~{end} 承銷價︰{offering_price} 參考價︰{last_price} {price_change}發行市場:{market}",
                market = stock.market.clone(),
                stock_symbol = stock.stock_symbol,
                stock_name = stock.stock_name,
                start = start,
                end = end,
                offering_price = offering_price,
                last_price = last_price,
                price_change = price_change,
            );
            let mut duration = (end - now).num_seconds() as usize;

            if duration == 0 {
                duration = declare::ONE_DAYS_IN_SECONDS;
            }

            crate::infra::nosql::redis::CLIENT
                .set(cache_key, true, duration)
                .await?;
        }
    }

    if !msg.is_empty() {
        // 整段訊息為動態內容（含日期、價格的小數點），送出前統一做 MarkdownV2 跳脫。
        let to_bot_msg = text::escape_markdown_v2(format!("{now} 可以申購的股票如下︰\n{msg}"));
        alert::send_message(&to_bot_msg).await;
        return Ok(());
    }

    Ok(())
}

/// 計算參考價與承銷價之間的價差與報酬率。
fn calculate_price_change(offering_price: Decimal, last_price: Decimal) -> String {
    if offering_price == Decimal::ZERO || last_price == Decimal::ZERO {
        return String::from(" - ");
    }
    //價差 = 參考價 - 承銷價
    //價差百分比 = 價差 / 參考價 * 100%
    let gap = last_price - offering_price;
    let change = (gap / last_price) * dec!(100);
    format!(
        "價差︰{gap} ({change}%) ",
        gap = gap,
        change = change.round_dp(2)
    )
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;

    use super::*;

    /// 驗證公開申購提醒流程可執行。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::info!("開始 execute");
        //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to execute because: {why:?}");
            }
        }

        tracing::info!("結束 execute");
    }

    /// 承銷價或參考價為 0 時無法計算價差；否則以參考價為分母算百分比。
    #[test]
    fn calculate_price_change_handles_missing_prices() {
        assert_eq!(calculate_price_change(Decimal::ZERO, dec!(60)), " - ");
        assert_eq!(calculate_price_change(dec!(50), Decimal::ZERO), " - ");
        assert_eq!(
            calculate_price_change(dec!(50), dec!(60)),
            "價差︰10 (16.67%) "
        );
        assert_eq!(
            calculate_price_change(dec!(60), dec!(50)),
            "價差︰-10 (-20.00%) "
        );
    }

    /// 以本機 mock server 取代證交所公開申購公告（G15），驗證提醒流程（G17）。
    mod mock_sources {
        use chrono::{Datelike, Days, NaiveDate};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::core::util::http::mock_origin::with_origins;
        use crate::infra::database;
        use crate::infra::nosql::redis::CLIENT;

        const PUBLIC_PATH: &str = "/rwd/zh/announcement/publicForm";
        const OPEN: &str = "79979P";
        const CLOSED: &str = "79979Q";
        const BOND: &str = "79979R";

        /// 民國年格式（`115/07/14`），與公告表格一致。
        fn roc(date: NaiveDate) -> String {
            format!(
                "{}/{:02}/{:02}",
                date.year() - 1911,
                date.month(),
                date.day()
            )
        }

        /// 公告表格的一列（12 欄）。
        fn row(symbol: &str, market: &str, start: NaiveDate, end: NaiveDate) -> Vec<String> {
            [
                "1",
                &roc(end),
                "測試申購",
                symbol,
                market,
                &roc(start),
                &roc(end),
                "1,000",
                "1,000",
                "50.00",
                "50.00",
                &roc(end),
            ]
            .map(str::to_string)
            .to_vec()
        }

        async fn cleanup() {
            for symbol in [OPEN, CLOSED, BOND] {
                let _ = CLIENT.delete(&format!("Public:{symbol}")).await;
            }
        }

        /// 只提醒申購期間包含今天的股票並設略過旗標；公債與已截止的不處理，第二次執行直接略過。
        #[tokio::test]
        #[cfg_attr(
            not(feature = "integration-tests"),
            ignore = "需要外部服務（PostgreSQL、Redis），請加 --features integration-tests 執行"
        )]
        async fn execute_flags_open_offerings_only() {
            dotenvy::dotenv().ok();
            if database::ping().await.is_err() || CLIENT.ping().await.is_err() {
                println!("跳過 execute_flags_open_offerings_only：無資料庫或 Redis 連接");
                return;
            }
            cleanup().await;
            // 申購期間由公告決定，必須以今天為準才會進入提醒分支；只寫 Redis 旗標，不寫資料庫。
            let today = Local::now().date_naive();
            let yesterday = today - Days::new(1);
            let tomorrow = today + Days::new(1);
            let last_week = today - Days::new(7);
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(PUBLIC_PATH))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "stat": "OK",
                    "date": "",
                    "title": "公開申購公告",
                    "fields": [],
                    "notes": [],
                    "total": 3,
                    "data": [
                        row(OPEN, "上市", yesterday, tomorrow),
                        row(CLOSED, "上櫃", last_week, yesterday),
                        row(BOND, "中央登錄公債", yesterday, tomorrow),
                    ]
                })))
                .expect(2)
                .mount(&server)
                .await;

            let uri = server.uri();
            let origins = [("https://www.twse.com.tw", uri.as_str())];
            let first = with_origins(&origins, execute()).await;
            let open_flagged = CLIENT.get_bool(&format!("Public:{OPEN}")).await;
            let closed_flagged = CLIENT.get_bool(&format!("Public:{CLOSED}")).await;
            let bond_flagged = CLIENT.get_bool(&format!("Public:{BOND}")).await;
            let second = with_origins(&origins, execute()).await;
            cleanup().await;

            assert!(first.is_ok(), "{first:?}");
            assert!(second.is_ok(), "已設旗標時應略過而非失敗：{second:?}");
            assert!(
                matches!(open_flagged, Ok(true)),
                "申購中應設旗標：{open_flagged:?}"
            );
            assert!(
                matches!(closed_flagged, Ok(false)),
                "已截止不應設旗標：{closed_flagged:?}"
            );
            assert!(
                matches!(bond_flagged, Ok(false)),
                "中央登錄公債不提醒：{bond_flagged:?}"
            );
        }

        /// 公告回應的 stat 不是 OK 時不提醒也不報錯。
        #[tokio::test]
        async fn execute_ignores_non_ok_stat() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(PUBLIC_PATH))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "stat": "很抱歉，沒有符合條件的資料!",
                    "date": "",
                    "title": "",
                    "fields": [],
                    "notes": [],
                    "total": 0,
                    "data": []
                })))
                .expect(1)
                .mount(&server)
                .await;

            let uri = server.uri();
            let result = with_origins(&[("https://www.twse.com.tw", &uri)], execute()).await;

            assert!(result.is_ok(), "{result:?}");
        }
    }
}
