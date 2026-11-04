use anyhow::Result;
use chrono::{Local, NaiveDate, TimeZone};

use crate::app::backfill::acl::IndexAclMapper;
use crate::app::event::handlers::get_global_dispatcher;
use crate::domain::events::DomainEvent;
use crate::domain::market_index::repository::MarketIndexRepository;
use crate::infra::database::repository::market_index::PgMarketIndexRepository;
use crate::{infra::cache::SHARE, infra::crawler::twse};

pub async fn execute() -> Result<()> {
    let tai_ex = twse::taiwan_capitalization_weighted_stock_index::visit(Local::now()).await?;
    if tai_ex.stat.to_uppercase() != "OK" {
        tracing::warn!("抓取加權股價指數 Finish taiex.Stat is not ok");
        return Ok(());
    }

    if let Some(data) = tai_ex.data {
        for item in data {
            let cmd = match IndexAclMapper::from_strings(&item) {
                Some(c) => c,
                None => continue,
            };

            let key = format!("{}-TAIEX", cmd.date);
            if SHARE.get_stock_index(&key).is_some() {
                continue;
            }

            let index_entity = IndexAclMapper::from_command(&cmd);
            let index_repo = PgMarketIndexRepository::new();

            match index_repo.save(&index_entity).await {
                Ok(_) => {
                    tracing::info!("index add {:?}", index_entity);

                    // 派發領域事件以發送 Telegram 通知
                    let event = DomainEvent::StockIndexUpdated {
                        date: cmd.date,
                        index: cmd.index,
                        change: cmd.change,
                        occurred_at: Local::now(),
                    };
                    get_global_dispatcher().dispatch_async(vec![event]).await;

                    SHARE.set_stock_index(key, index_entity).await;
                }
                Err(why) => {
                    tracing::error!(
                        "Failed to index.upsert({:#?}) because {:?}",
                        index_entity,
                        why
                    );
                }
            }
        }
    }

    Ok(())
}

/// 依指定日期調用 TWSE API 回補該日的台股加權指數。
///
/// TWSE API 會回傳整個月份的資料，但此函式只會 upsert 與 `date` 完全相符的那一筆。
///
/// 與 [`execute`] 的差異：
/// 1. 使用呼叫端傳入的 `date` 查詢 TWSE。
/// 2. 只寫入指定日期的資料，忽略同月份其他交易日。
/// 3. 跳過 `SHARE` 快取檢查，確保資料一定會 upsert 到資料庫。
/// 4. 不發送 Telegram 通知，避免歷史回補產生誤導訊息。
///
/// 回傳成功 upsert 的資料筆數（0 或 1）。
pub async fn execute_for_date(date: NaiveDate) -> Result<usize> {
    // 將 NaiveDate 轉成 DateTime<Local>，TWSE API 只看日期中的年月部分。
    let datetime = Local
        .from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
        .single()
        .unwrap_or_else(Local::now);

    let tai_ex = twse::taiwan_capitalization_weighted_stock_index::visit(datetime).await?;
    if tai_ex.stat.to_uppercase() != "OK" {
        tracing::warn!("抓取加權股價指數 (date={date}) Finish taiex.Stat is not ok");
        return Ok(0);
    }

    Ok(upsert_for_date(tai_ex.data, date).await)
}

/// 從 TWSE 整月資料中挑出與 `date` 相符的那一筆寫入資料庫並更新快取；回傳寫入筆數（0 或 1）。
///
/// 格式不符的資料列略過；單筆寫入失敗只記錄錯誤。
async fn upsert_for_date(data: Option<Vec<Vec<String>>>, date: NaiveDate) -> usize {
    let mut upserted_count: usize = 0;

    if let Some(data) = data {
        for item in data {
            let cmd = match IndexAclMapper::from_strings(&item) {
                Some(c) => c,
                None => continue,
            };

            // TWSE 回傳整月資料，只處理與指定日期相符的那一筆。
            if cmd.date != date {
                continue;
            }

            let index_entity = IndexAclMapper::from_command(&cmd);
            let index_repo = PgMarketIndexRepository::new();

            match index_repo.save(&index_entity).await {
                Ok(_) => {
                    tracing::info!("index upsert (backfill) {:?}", index_entity);
                    let key = format!("{}-TAIEX", index_entity.date);
                    SHARE.set_stock_index(key, index_entity).await;
                    upserted_count += 1;
                }
                Err(why) => {
                    tracing::error!(
                        "Failed to index.upsert({:#?}) because {:?}",
                        index_entity,
                        why
                    );
                }
            }
        }
    }

    upserted_count
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只寫入指定日期那一筆：其他日期與格式不符的資料列略過，寫入後快取也有這一天。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn upsert_for_date_writes_only_the_requested_day() {
        dotenvy::dotenv().ok();
        let pool = crate::infra::database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 upsert_for_date_writes_only_the_requested_day：無資料庫連接");
            return;
        }
        let date = NaiveDate::from_ymd_opt(1989, 6, 2).expect("固定日期合法");
        let other = NaiveDate::from_ymd_opt(1989, 6, 1).expect("固定日期合法");
        let cleanup = || async {
            sqlx::query(r#"DELETE FROM "index" WHERE category = 'TAIEX' AND "date" = ANY($1)"#)
                .bind(vec![date, other])
                .execute(pool)
                .await
                .expect("清除測試指數");
        };
        cleanup().await;

        let row = |day: &str, index: &str| -> Vec<String> {
            [day, "1,000", "2,000", "300", index, "-12.5"]
                .map(String::from)
                .to_vec()
        };
        let data = vec![
            row("78/06/01", "9,000.00"),
            row("78/06/02", "9,123.45"),
            vec!["78/06/02".to_string()],
        ];
        assert_eq!(upsert_for_date(Some(data), date).await, 1);
        assert_eq!(upsert_for_date(None, date).await, 0);

        let saved: Vec<(NaiveDate, rust_decimal::Decimal, rust_decimal::Decimal)> = sqlx::query_as(
            r#"SELECT "date", index, change FROM "index" WHERE category = 'TAIEX' AND "date" = ANY($1)"#,
        )
        .bind(vec![date, other])
        .fetch_all(pool)
        .await
        .expect("讀回測試指數");
        assert_eq!(
            saved,
            vec![(
                date,
                rust_decimal::Decimal::new(912_345, 2),
                rust_decimal::Decimal::new(-125, 1)
            )]
        );
        assert!(SHARE.get_stock_index("1989-06-02-TAIEX").is_some());

        cleanup().await;
    }

    /// 以本機 mock server 取代 TWSE（G15）：排程入口與指定日期回補的連外分支。
    mod mock_sources {
        use rust_decimal::Decimal;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::core::util::http::mock_origin::with_origins;
        use crate::domain::market_index::entity::MarketIndex;

        const TWSE: &str = "https://www.twse.com.tw";
        const FMTQIK: &str = "/exchangeReport/FMTQIK";

        /// TWSE 整月指數的一列：日期、成交股數、成交金額、成交筆數、指數、漲跌點數。
        fn row(day: &str, index: &str) -> serde_json::Value {
            serde_json::json!([day, "1,000", "2,000", "300", index, "-12.5"])
        }

        /// 回應固定內容的 FMTQIK mock server。
        async fn index_server(status: u16, body: serde_json::Value) -> MockServer {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(FMTQIK))
                .respond_with(ResponseTemplate::new(status).set_body_json(body))
                .mount(&server)
                .await;
            server
        }

        fn day(month: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(1989, month, day).expect("固定日期合法")
        }

        /// 讀回 1989 年測試日期的 TAIEX 指數，依日期排序。
        async fn saved(dates: &[NaiveDate]) -> Vec<(NaiveDate, Decimal)> {
            sqlx::query_as(
                r#"SELECT "date", index FROM "index" WHERE category = 'TAIEX' AND "date" = ANY($1) ORDER BY "date""#,
            )
            .bind(dates)
            .fetch_all(crate::infra::database::get_connection())
            .await
            .expect("讀回測試指數")
        }

        async fn cleanup(dates: &[NaiveDate]) {
            sqlx::query(r#"DELETE FROM "index" WHERE category = 'TAIEX' AND "date" = ANY($1)"#)
                .bind(dates)
                .execute(crate::infra::database::get_connection())
                .await
                .expect("清除測試指數");
        }

        /// stat 不是 OK（休市、查無資料）時兩個入口都直接結束，不碰資料庫。
        #[tokio::test]
        async fn non_ok_stat_writes_nothing() {
            let server = index_server(
                200,
                serde_json::json!({ "stat": "很抱歉，沒有符合條件的資料!" }),
            )
            .await;

            let (scheduled, backfilled) = with_origins(&[(TWSE, &server.uri())], async {
                (execute().await, execute_for_date(day(7, 3)).await)
            })
            .await;

            scheduled.expect("stat 非 OK 不算錯誤");
            assert_eq!(backfilled.expect("stat 非 OK 不算錯誤"), 0);
            let requests = server.received_requests().await.unwrap_or_default();
            assert_eq!(requests.len(), 2);
        }

        /// TWSE 回 HTTP 500 時兩個入口都回錯誤。
        #[tokio::test]
        async fn http_error_is_returned() {
            let server = index_server(500, serde_json::json!({})).await;

            let (scheduled, backfilled) = with_origins(&[(TWSE, &server.uri())], async {
                (execute().await, execute_for_date(day(7, 3)).await)
            })
            .await;

            assert!(scheduled.is_err(), "排程入口要把連線錯誤往上傳");
            assert!(backfilled.is_err(), "回補入口要把連線錯誤往上傳");
        }

        /// 排程入口寫入整月中快取還沒有的交易日；快取已有的那天不重寫。
        #[tokio::test]
        #[cfg_attr(
            not(feature = "integration-tests"),
            ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
        )]
        async fn execute_saves_days_missing_from_cache() {
            dotenvy::dotenv().ok();
            if crate::infra::database::ping().await.is_err() {
                println!("跳過 execute_saves_days_missing_from_cache：無資料庫連接");
                return;
            }
            let dates = [day(7, 5), day(7, 6)];
            cleanup(&dates).await;
            let cached = MarketIndex::new(
                "TAIEX".to_string(),
                dates[0],
                Decimal::new(8_000, 0),
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO,
            );
            SHARE
                .set_stock_index(format!("{}-TAIEX", dates[0]), cached)
                .await;
            let server = index_server(
                200,
                serde_json::json!({
                    "stat": "OK",
                    "data": [row("78/07/05", "9,000.00"), row("78/07/06", "9,123.45"), ["78/07/06"]]
                }),
            )
            .await;

            with_origins(&[(TWSE, &server.uri())], execute())
                .await
                .expect("execute");

            assert_eq!(
                saved(&dates).await,
                vec![(dates[1], Decimal::new(912_345, 2))],
                "快取已有 07-05，只寫 07-06"
            );
            assert!(SHARE.get_stock_index("1989-07-06-TAIEX").is_some());
            cleanup(&dates).await;
        }

        /// 指定日期回補只寫那一天，同月份其他交易日略過。
        #[tokio::test]
        #[cfg_attr(
            not(feature = "integration-tests"),
            ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
        )]
        async fn execute_for_date_writes_only_requested_day() {
            dotenvy::dotenv().ok();
            if crate::infra::database::ping().await.is_err() {
                println!("跳過 execute_for_date_writes_only_requested_day：無資料庫連接");
                return;
            }
            let dates = [day(7, 10), day(7, 11)];
            cleanup(&dates).await;
            let server = index_server(
                200,
                serde_json::json!({
                    "stat": "ok",
                    "data": [row("78/07/10", "9,500.00"), row("78/07/11", "9,600.00")]
                }),
            )
            .await;

            let written = with_origins(&[(TWSE, &server.uri())], execute_for_date(dates[1]))
                .await
                .expect("execute_for_date");

            assert_eq!(written, 1);
            assert_eq!(
                saved(&dates).await,
                vec![(dates[1], Decimal::new(9_600, 0))]
            );
            cleanup(&dates).await;
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 execute");

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 execute");
    }
}
