use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    future::Future,
};

use anyhow::Result;
use chrono::Local;
use rust_decimal::Decimal;

use crate::{
    core::declare::Quarter,
    core::util::map::Keyable,
    domain::financial::{
        entity::FinancialStatement as DomainFinancialStatement, repository::FinancialRepository,
    },
    infra::crawler::nstock::{self, eps::Eps},
    infra::database::repository::financial::PgFinancialRepository,
    infra::nosql::redis::CLIENT,
};

/// 更新台股年度財報
pub mod annual;
/// 更新台股季度財報
pub mod quarter;

/// nstock 查過的股票，這段時間內不再查（3 天）。
///
/// 旗標的值是查詢當時待補列的最新年度：之後出現更新年度的待補列（新一季）照樣會查，
/// 不會被舊旗標擋住；最新年度的列另外每天由 04:00 的 Yahoo profile 補抓。
const SKIP_TTL_SECONDS: usize = 60 * 60 * 24 * 3;

/// 一輪 nstock 補 ROE／ROA 的統計。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RoeRoaRun {
    /// ROE 或 ROA 為 0 的待補列數。
    rows: usize,
    /// 待補列涉及的股票檔數。
    stocks: usize,
    /// 近期查過、這次略過的檔數。
    skipped: usize,
    /// 實際查詢 nstock 的檔數。
    fetched: usize,
    /// nstock 查詢失敗的檔數。
    failed: usize,
    /// 實際寫回的列數。
    updated: usize,
}

impl fmt::Display for RoeRoaRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rows={} stocks={} skipped={} fetched={} failed={} updated={}",
            self.rows, self.stocks, self.skipped, self.fetched, self.failed, self.updated
        )
    }
}

/// 以 nstock 補齊 ROE 或 ROA 為 0 的財報；`quarter` 為 `None` 時處理年報。
async fn update_roe_and_roa_for_zero_values(quarter: Option<Quarter>) -> Result<()> {
    let financial_repo = PgFinancialRepository::new();
    let fss = financial_repo
        .fetch_roe_or_roa_equal_to_zero(None, quarter)
        .await?;
    let run = fill_roe_roa(&financial_repo, fss, quarter, |stock_symbol| async move {
        nstock::eps::visit(&stock_symbol).await
    })
    .await;
    tracing::info!("nstock 補 ROE/ROA 結束: scope={} {run}", scope(quarter));

    Ok(())
}

/// 逐檔向 nstock 取 ROE／ROA，寫回有補到的列。
///
/// `fetch` 是 nstock 的查詢函式，測試時換成假資料。
async fn fill_roe_roa<F, Fut>(
    repo: &PgFinancialRepository,
    fss: Vec<DomainFinancialStatement>,
    quarter: Option<Quarter>,
    fetch: F,
) -> RoeRoaRun
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Eps>>,
{
    let pending = pending_by_stock(fss);
    let mut run = RoeRoaRun {
        rows: pending.values().map(Vec::len).sum(),
        stocks: pending.len(),
        ..RoeRoaRun::default()
    };

    for (stock_symbol, mut rows) in pending {
        let latest_year = rows.iter().map(|fs| fs.year).max().unwrap_or_default();
        let skip_key = skip_cache_key(&stock_symbol, quarter);
        // 沒有旗標或 Redis 讀不到都當作沒查過，照常查詢。
        let checked_year = CLIENT
            .get_string(&skip_key)
            .await
            .ok()
            .and_then(|year| year.parse().ok());
        if already_checked(checked_year, latest_year) {
            run.skipped += 1;
            continue;
        }

        run.fetched += 1;
        let eps = match fetch(stock_symbol.clone()).await {
            Ok(eps) => eps,
            Err(why) => {
                run.failed += 1;
                tracing::error!(
                    "nstock 補 ROE/ROA 查詢失敗: stock_symbol={stock_symbol}, error={why:?}"
                );
                continue;
            }
        };

        let values = nstock_roe_roa(eps, quarter);
        for fs in &mut rows {
            let key = format!("{}-{}-{}", fs.security_code, fs.year, fs.quarter);
            let Some(&(roe, roa)) = values.get(&key) else {
                continue;
            };
            if !should_write(fs, roe, roa) {
                continue;
            }
            fs.return_on_equity = roe;
            fs.return_on_assets = roa;
            fs.updated_time = Local::now();
            match repo.update_statement_roe_roa(fs).await {
                Ok(()) => run.updated += 1,
                Err(why) => tracing::error!("{why:?}"),
            }
        }

        // 已用 nstock 最新的資料補過；還是 0 的列多半是 nstock 也沒有，近期內不必再查。
        if let Err(why) = CLIENT.set(&skip_key, latest_year, SKIP_TTL_SECONDS).await {
            tracing::error!("寫入 nstock 補 ROE/ROA 跳過旗標失敗: key={skip_key}, error={why:?}");
        }
    }

    run
}

/// 挑出 ROE 或 ROA 為 0 的列，依股票分組。
///
/// 撈出的列也包含只有每股淨值為 0 的（約 2.8 萬列）。nstock 不提供每股淨值，
/// 原本每天為了這些列對 nstock 打近 1,900 次請求，再把相同的值寫回去，什麼也沒補到；
/// 每股淨值改由 Yahoo profile 與 05:10 的排程補。
fn pending_by_stock(
    fss: Vec<DomainFinancialStatement>,
) -> BTreeMap<String, Vec<DomainFinancialStatement>> {
    let mut pending: BTreeMap<String, Vec<DomainFinancialStatement>> = BTreeMap::new();
    for fs in fss
        .into_iter()
        .filter(|fs| fs.return_on_equity.is_zero() || fs.return_on_assets.is_zero())
    {
        pending
            .entry(fs.security_code.clone())
            .or_default()
            .push(fs);
    }
    pending
}

/// 把 nstock 的資料整理成「股票-年度-季度」→（ROE, ROA）；年報的季度是空字串。
fn nstock_roe_roa(eps: Eps, quarter: Option<Quarter>) -> HashMap<String, (Decimal, Decimal)> {
    match quarter {
        None => eps
            .years
            .into_iter()
            .map(|year| (year.key(), (year.roe, year.roa)))
            .collect(),
        Some(_) => eps
            .quarters
            .into_iter()
            .map(|quarter| (quarter.key(), (quarter.roe, quarter.roa)))
            .collect(),
    }
}

/// nstock 的值是否值得寫回：兩者都是 0 等於沒補到，與現值相同也不必寫。
fn should_write(fs: &DomainFinancialStatement, roe: Decimal, roa: Decimal) -> bool {
    !(roe.is_zero() && roa.is_zero()) && (fs.return_on_equity != roe || fs.return_on_assets != roa)
}

/// 處理範圍的名稱：季報用季度，年報用 `annual`。
fn scope(quarter: Option<Quarter>) -> String {
    quarter.map_or_else(|| "annual".to_string(), |quarter| quarter.to_string())
}

/// 單檔股票的跳過旗標 key，見 [`SKIP_TTL_SECONDS`]。
fn skip_cache_key(stock_symbol: &str, quarter: Option<Quarter>) -> String {
    format!("nstock:roe_roa:{stock_symbol}:{}", scope(quarter))
}

/// 近期查過、且待補列沒有比當時更新的年度時略過。
fn already_checked(checked_year: Option<i64>, latest_year: i64) -> bool {
    checked_year.is_some_and(|checked| checked >= latest_year)
}

#[cfg(test)]
mod tests;
