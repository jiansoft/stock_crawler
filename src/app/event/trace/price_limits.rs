//! # 開盤前載入交易所公告的漲跌停價（H46）
//!
//! 盤中驗價（`Share::is_valid_price`）原本只有 Yahoo 類股報價會帶當日漲跌停價，其餘股票用固定的
//! 10.5% 門檻。國外成分 ETF、債券 ETF 沒有漲跌幅限制，又多半不在 Yahoo 類股裡：2026-11-06 加權
//! 下跌 4.5%，00954 盤中真實低點 12.6（昨收 14.21，−11.3%）被當成異常價格濾掉，同日還有 00735、
//! 00911、00951、00662 等，盤中價格追蹤因此看不到這些價。
//!
//! 開盤前讀上市 TWT84U 與上櫃收盤行情的次日漲跌停價寫入快取：
//! - **無漲跌幅限制**的證券一律採用：這個屬性不會每天變，前一個交易日的資料也適用。
//! - **漲跌停區間**只採用確定是今天的上市資料（TWT84U 資料日期＝今天）；上櫃的次日漲跌停價
//!   要知道「資料日期的下一個交易日是不是今天」才能用，遇到連假或資料晚更新容易套錯日期，
//!   一般股票本來就由固定門檻處理，這裡不冒這個險。
//!
//! Yahoo 類股報價之後每輪仍會覆蓋同一檔的限制，兩者並存時以盤中較新的 Yahoo 為準。

use anyhow::{Result, anyhow};
use chrono::{Local, NaiveDate};

use crate::infra::{
    cache::{PriceLimit, SHARE},
    crawler::{share::ExchangePriceLimits, tpex, twse},
};

/// 一個市場這次寫入快取的筆數。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Applied {
    /// 無漲跌幅限制的筆數。
    unlimited: usize,
    /// 漲跌停區間的筆數。
    ranges: usize,
}

/// 依資料日期挑出可以當成「今天」限制的項目。
///
/// `ranges_apply_today` 為真時連區間一起採用，否則只留無漲跌幅限制。
fn usable_limits(
    data: ExchangePriceLimits,
    ranges_apply_today: bool,
) -> (Vec<(String, PriceLimit)>, Applied) {
    let mut applied = Applied::default();
    let limits = data
        .limits
        .into_iter()
        .filter(|(_, limit)| match limit {
            PriceLimit::Unlimited => {
                applied.unlimited += 1;
                true
            }
            PriceLimit::Range { .. } if ranges_apply_today => {
                applied.ranges += 1;
                true
            }
            PriceLimit::Range { .. } | PriceLimit::Unknown => false,
        })
        .collect();
    (limits, applied)
}

/// 把一個市場的資料寫入今天的漲跌停快取，回傳資料日期與寫入筆數。
fn apply(
    data: ExchangePriceLimits,
    today: NaiveDate,
    ranges_apply_today: bool,
) -> (NaiveDate, Applied) {
    let date = data.date;
    let (limits, applied) = usable_limits(data, ranges_apply_today);
    SHARE.set_price_limits(today, limits);
    (date, applied)
}

/// 讀上市、上櫃的漲跌停價寫入快取；兩個市場互不影響，一邊失敗只記 warn。
///
/// # Errors
///
/// 兩個市場都失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    execute_on(Local::now().date_naive()).await
}

/// [`execute`] 的實作，`today` 由呼叫端指定以便測試。
async fn execute_on(today: NaiveDate) -> Result<()> {
    let (listed, otc) = tokio::join!(twse::price_limit::visit(), tpex::price_limit::visit());

    let listed = listed.map(|data| {
        let ranges_apply_today = data.date == today;
        apply(data, today, ranges_apply_today)
    });
    let otc = otc.map(|data| apply(data, today, false));

    match (&listed, &otc) {
        (Ok((listed_date, listed)), Ok((otc_date, otc))) => tracing::info!(
            "交易所漲跌停價已載入: 上市 資料日={listed_date} 無限制={} 區間={}；上櫃 資料日={otc_date} 無限制={}",
            listed.unlimited,
            listed.ranges,
            otc.unlimited
        ),
        (Ok(_), Err(why)) => tracing::warn!("上櫃次日漲跌停價載入失敗，只用上市資料: {why:#}"),
        (Err(why), Ok(_)) => tracing::warn!("上市漲跌停價載入失敗，只用上櫃資料: {why:#}"),
        (Err(listed), Err(otc)) => {
            return Err(anyhow!(
                "交易所漲跌停價載入失敗：上市 {listed:#}；上櫃 {otc:#}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
