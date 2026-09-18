//! # 各資料來源的擷取與正規化
//!
//! 把各採集器回傳的原始資料整理成後續流程好查的索引，並載入資料庫既有的股利資料。
//! 各來源分別補哪些欄位，見 [`super`] 的「資料來源分工」。
//!
//! 這一層只負責「取得與整理」，不做任何期別或異動的判斷；
//! 哪些來源失敗可以降級、哪些必須中止，由 [`super::scan_ex_dividend_announcements`] 決定。

use std::collections::HashMap;

use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use tokio_retry::{
    Retry,
    strategy::{ExponentialBackoff, jitter},
};

use crate::{
    domain::dividend::{entity::Dividend, repository::DividendRepository},
    infra::crawler::{
        moneydj::dividend_schedule::DividendSchedule,
        mops::dividend_allotment::DividendAllotment,
        share::ExDividendAnnouncement,
        yahoo::{self, dividend::YahooDividend},
    },
};

/// 把採集結果併入清單；失敗時記錄錯誤並回傳 `true`。
pub(super) fn collect_or_log(
    result: Result<Vec<ExDividendAnnouncement>>,
    source: &str,
    sink: &mut Vec<ExDividendAnnouncement>,
) -> bool {
    match result {
        Ok(data) => {
            tracing::info!("{} 取得 {} 筆", source, data.len());
            sink.extend(data);
            false
        }
        Err(why) => {
            tracing::error!("取得{}失敗: {:?}", source, why);
            true
        }
    }
}

/// 以股票代號索引 MOPS 的股利分派情形。
pub(super) fn index_allotments(
    data: Vec<DividendAllotment>,
) -> HashMap<String, Vec<DividendAllotment>> {
    let mut map: HashMap<String, Vec<DividendAllotment>> = HashMap::with_capacity(data.len());
    for item in data {
        map.entry(item.stock_symbol.clone()).or_default().push(item);
    }
    map
}

/// 以「股票代號 + 除權息日」索引 MoneyDJ 的日程資料。
pub(super) fn index_schedules(
    data: Vec<DividendSchedule>,
) -> HashMap<(String, NaiveDate), DividendSchedule> {
    data.into_iter()
        .map(|item| ((item.stock_symbol.clone(), item.ex_date), item))
        .collect()
}

/// 載入公告涉及年度的既有股利資料，並以股票代號分組。
///
/// 除權息日的年份之外還要一併載入次年：季配、半年配跨年發放時，
/// 資料列是掛在**發放年度**底下的（12 月除息、隔年 1 月發放的事件記在隔年）。
pub(super) async fn fetch_existing(
    repository: &dyn DividendRepository,
    announcements: &[ExDividendAnnouncement],
) -> Result<HashMap<String, Vec<Dividend>>> {
    let mut years: Vec<i32> = announcements
        .iter()
        .flat_map(|item| [item.ex_date.year(), item.ex_date.year() + 1])
        .collect();
    years.sort_unstable();
    years.dedup();

    let rows = repository.fetch_by_years(&years).await?;
    let mut map: HashMap<String, Vec<Dividend>> = HashMap::with_capacity(rows.len());
    for row in rows {
        map.entry(row.security_code.clone()).or_default().push(row);
    }

    Ok(map)
}

/// 抓取單一股票的 Yahoo 股利政策，失敗時記錄並回傳 `None`。
pub(super) async fn fetch_yahoo_dividend(stock_symbol: &str) -> Option<YahooDividend> {
    let strategy = ExponentialBackoff::from_millis(100).map(jitter).take(3);
    match Retry::start(strategy, || yahoo::dividend::visit(stock_symbol)).await {
        Ok(data) => Some(data),
        Err(why) => {
            tracing::warn!("取得 {} 的 Yahoo 股利政策失敗: {:?}", stock_symbol, why);
            None
        }
    }
}
