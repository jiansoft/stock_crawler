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
    execute_on(Local::now().date_naive()).await
}

/// [`execute`] 的本體：抓 `today` 上個月的月營收；日期可注入，讓測試以歷史月份跑完整流程。
async fn execute_on(today: NaiveDate) -> Result<()> {
    tracing::info!("更新台股月營收開始");
    defer! {
        tracing::info!("更新台股月營收結束");
    }

    process_revenues(previous_month_end(today)).await
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
mod tests;
