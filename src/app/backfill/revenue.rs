use crate::{
    app::backfill::acl::{RevenueAclMapper, UpdateRevenueCommand},
    core::util,
    infra::cache::SHARE,
    infra::crawler::twse,
};
use anyhow::Result;
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveTime, TimeDelta, TimeZone};
use futures::{StreamExt, stream};
use scopeguard::defer;

/// 調用  twse API 取得台股月營收
pub async fn execute() -> Result<()> {
    tracing::info!("更新台股月營收開始");
    defer! {
        tracing::info!("更新台股月營收結束");
    }

    process_revenues(previous_month_end(Local::now().date_naive())).await
}

/// 上個月最後一分鐘（台北時間 23:59），交給 TWSE 月營收爬蟲決定要抓哪個月份。
fn previous_month_end(today: NaiveDate) -> DateTime<FixedOffset> {
    let first_day = today.with_day(1).unwrap_or(today);
    let last_minute = first_day.and_time(NaiveTime::MIN) - TimeDelta::minutes(1);
    let taipei = FixedOffset::east_opt(8 * 60 * 60).expect("UTC+8 是合法時區");
    taipei
        .from_local_datetime(&last_minute)
        .single()
        .expect("固定時差的時區沒有重複或不存在的時間")
}

async fn process_revenues(last_month_timezone: DateTime<FixedOffset>) -> Result<()> {
    let year = last_month_timezone.year();
    let month = last_month_timezone.month();

    let revenues = twse::revenue::visit(last_month_timezone).await?;
    let cmds: Vec<UpdateRevenueCommand> = revenues.iter().map(RevenueAclMapper::from_dto).collect();
    save_revenues(cmds, year, month as i32).await?;

    // 整個月份的營收都寫完才派發事件：逐檔發會產生上千個事件，也無從彙總成一則通知。
    // 事件只帶月份，要通知哪些股票由 handler 依「目前持股」自行查詢。
    let revenue_date = i64::from((year * 100) + month as i32);
    crate::app::event::get_global_dispatcher()
        .dispatch_async(vec![
            crate::domain::events::DomainEvent::MonthlyRevenueUpdated {
                date: revenue_date,
                occurred_at: Local::now(),
            },
        ])
        .await;

    Ok(())
}

/// 逐檔寫入當月營收（附上月均價與高低價），全部寫完再重建最新營收日期表。
///
/// 單檔失敗只記錄錯誤，不影響其他股票。
async fn save_revenues(cmds: Vec<UpdateRevenueCommand>, year: i32, month: i32) -> Result<()> {
    use crate::domain::financial::repository::FinancialRepository;
    use crate::infra::database::repository::financial::PgFinancialRepository;

    stream::iter(cmds)
        .for_each_concurrent(util::concurrent_limit_16(), |cmd| async move {
            if let Err(why) = process_revenue(cmd, year, month).await {
                tracing::error!("Failed to process_revenue because {:?}", why);
            }
        })
        .await;

    PgFinancialRepository::new()
        .rebuild_revenue_last_date()
        .await
}

pub(crate) async fn process_revenue(
    cmd: UpdateRevenueCommand,
    year: i32,
    month: i32,
) -> Result<()> {
    use crate::domain::financial::entity::MonthlyRevenue as DomainMonthlyRevenue;
    use crate::domain::financial::repository::FinancialRepository;
    use crate::infra::database::repository::financial::PgFinancialRepository;

    use crate::domain::quote::repository::QuoteRepository;
    use crate::infra::database::repository::quote::PgQuoteRepository;

    let financial_repo = PgFinancialRepository::new();
    let quote_repo = PgQuoteRepository::new();
    let mut table_entity = RevenueAclMapper::from_command(&cmd);

    if let Ok(Some((lowest_price, avg_price, highest_price))) = quote_repo
        .fetch_monthly_stock_price_summary(&cmd.symbol, year, month)
        .await
    {
        table_entity.lowest_price = lowest_price;
        table_entity.avg_price = avg_price;
        table_entity.highest_price = highest_price;
    }

    // 轉成領域實體進行儲存
    let domain_entity = DomainMonthlyRevenue::from(table_entity.clone());
    financial_repo.save_monthly_revenue(&domain_entity).await?;

    // 快取維持使用原 Table 實體以維持相容性
    SHARE.set_last_revenues(table_entity.clone());

    let name = match SHARE.get_stock(&cmd.symbol).await {
        None => String::from("-"),
        Some(s) => s.name().to_string(),
    };

    tracing::info!(
        "公司代號:{}  公司名稱:{} 當月營收:{} 上月營收:{} 去年當月營收:{} 月均價:{} 最低價:{} 最高價:{}",
        cmd.symbol,
        name,
        cmd.monthly,
        cmd.last_month,
        cmd.last_year_this_month,
        table_entity.avg_price,
        table_entity.lowest_price,
        table_entity.highest_price
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::infra::database;

    /// 月中、月初與跨年都回到上個月最後一分鐘（台北時間）。
    #[test]
    fn previous_month_end_is_last_minute_of_previous_month_in_taipei() {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("日期應合法");
        for (today, expected) in [
            (day(2026, 10, 11), "2026-09-30T23:59:00+08:00"),
            (day(2026, 10, 1), "2026-09-30T23:59:00+08:00"),
            (day(2026, 3, 31), "2026-02-28T23:59:00+08:00"),
            (day(2026, 1, 5), "2025-12-31T23:59:00+08:00"),
        ] {
            assert_eq!(previous_month_end(today).to_rfc3339(), expected, "{today}");
        }
    }

    /// 假代號：A 有當月日線，B 沒有。
    const SYMBOL_A: &str = "79979RV";
    const SYMBOL_B: &str = "79979RW";

    async fn cleanup() {
        let pool = database::get_connection();
        for symbol in [SYMBOL_A, SYMBOL_B] {
            for sql in [
                r#"DELETE FROM "Revenue" WHERE stock_symbol = $1"#,
                "DELETE FROM revenue_last_date WHERE stock_symbol = $1",
                r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
                "DELETE FROM stocks WHERE stock_symbol = $1",
            ] {
                let _ = sqlx::query(sql).bind(symbol).execute(pool).await;
            }
        }
    }

    fn command(symbol: &str, monthly: Decimal) -> UpdateRevenueCommand {
        UpdateRevenueCommand {
            symbol: symbol.to_owned(),
            monthly,
            last_month: dec!(900),
            last_year_this_month: dec!(800),
            monthly_accumulated: monthly,
            last_year_monthly_accumulated: dec!(800),
            compared_with_last_month: dec!(11.11),
            compared_with_last_year_same_month: dec!(25),
            accumulated_compared_with_last_year: dec!(25),
            date: 202001,
        }
    }

    /// 寫入當月營收並帶上該月日線的最低、平均、最高價；沒有日線的股票價格欄維持 0；
    /// 寫完後最新營收日期表指向這個月份。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn save_revenues_writes_prices_and_rebuilds_latest_dates() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 save_revenues_writes_prices_and_rebuilds_latest_dates：無資料庫連接");
            return;
        }
        cleanup().await;
        let pool = database::get_connection();
        for symbol in [SYMBOL_A, SYMBOL_B] {
            sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, "SuspendListing") VALUES ($1, '營收測試', $1, 2, false)"#,
            )
            .bind(symbol)
            .execute(pool)
            .await
            .expect("種入股票主檔");
        }
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice")
               VALUES ('2020-01-02', $1, 2020, 1, 2, 100, 104, 98, 102),
                      ('2020-01-03', $1, 2020, 1, 3, 102, 110, 101, 108)"#,
        )
        .bind(SYMBOL_A)
        .execute(pool)
        .await
        .expect("種入日線");

        save_revenues(
            vec![command(SYMBOL_A, dec!(1000)), command(SYMBOL_B, dec!(500))],
            2020,
            1,
        )
        .await
        .expect("save_revenues");

        let rows: Vec<(String, Decimal, Decimal, Decimal, Decimal)> = sqlx::query_as(
            r#"SELECT stock_symbol, "Monthly", lowest_price, avg_price, highest_price FROM "Revenue" WHERE stock_symbol IN ($1, $2) AND "Date" = 202001 ORDER BY stock_symbol"#,
        )
        .bind(SYMBOL_A)
        .bind(SYMBOL_B)
        .fetch_all(pool)
        .await
        .expect("讀回營收");
        assert_eq!(
            rows,
            vec![
                (
                    SYMBOL_A.to_owned(),
                    dec!(1000),
                    dec!(98),
                    dec!(105),
                    dec!(110)
                ),
                (
                    SYMBOL_B.to_owned(),
                    dec!(500),
                    Decimal::ZERO,
                    Decimal::ZERO,
                    Decimal::ZERO
                ),
            ]
        );
        let latest: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM revenue_last_date l JOIN "Revenue" r ON r."Serial" = l.serial WHERE l.stock_symbol IN ($1, $2) AND r."Date" = 202001"#,
        )
        .bind(SYMBOL_A)
        .bind(SYMBOL_B)
        .fetch_one(pool)
        .await
        .expect("讀回最新營收日期");
        assert_eq!(latest, 2);

        cleanup().await;
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
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    #[tokio::test]
    #[ignore]
    async fn test_process_revenues() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 test_process_revenues");

        let naive_datetime = NaiveDate::from_ymd_opt(2025, 4, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let month_timezone = timezone.from_local_datetime(&naive_datetime).unwrap();

        match process_revenues(month_timezone).await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to test_process_revenues because {:?}", why);
            }
        }

        tracing::debug!("結束 test_process_revenues");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
