//! 單檔股利的寫入計畫：由 Yahoo 股利明細決定要寫哪些列、重算哪些年度合計，不碰資料庫。

use std::collections::{BTreeSet, HashMap};

use crate::{
    app::backfill::acl::YahooDividendAclMapper, domain::dividend::entity::Dividend,
    infra::crawler::yahoo,
};

/// 單檔股利的寫入計畫：要 upsert 的明細與要重算年度合計的發放年度。
#[derive(Debug, Default, PartialEq)]
pub(super) struct StockDividendPlan {
    /// 需要寫入（新增或來源有更正）的股利，依 Yahoo 原始順序。
    pub(super) to_save: Vec<Dividend>,
    /// 需要重算年度合計列的發放年度。
    pub(super) refresh_years: BTreeSet<i32>,
}

/// 由 Yahoo 股利明細決定這檔股票要寫哪些列、重算哪些年度合計；不碰資料庫。
///
/// 1. 只處理指定年度與前一年度的股利所屬年度（[`should_process_dividend_year`]）。
/// 2. 有季配/半年配明細的發放年度一律重算合計——2753 的 2026 合計就是因為明細早已存在、
///    全部被略過，錯誤的合計列（現金 0）才一直留在資料表裡。
/// 3. 快取中已有、且內容與來源一致的明細不寫（[`dividend_content_differs`]）；
///    有差異才寫回，才補得上例如「配股併入 H2」這種來源端的更正。
pub(super) fn plan_stock_dividends(
    year: i32,
    stock_symbol: &str,
    dividends_from_yahoo: &yahoo::dividend::YahooDividend,
    multiple_dividend_cache: &HashMap<String, Dividend>,
) -> StockDividendPlan {
    let mut plan = StockDividendPlan::default();
    for (_, details) in &dividends_from_yahoo.dividend {
        for detail in details {
            if !should_process_dividend_year(year, detail.year_of_dividend) {
                continue;
            }
            // key 使用股利所屬年度而非發放年度，才能正確區分跨年度發放的 Q4 或 H2 配息。
            let key = make_dividend_key(stock_symbol, detail.year_of_dividend, &detail.quarter);
            if !detail.quarter.is_empty() {
                plan.refresh_years.insert(detail.year);
            }

            let cmd = YahooDividendAclMapper::from_dto(stock_symbol, detail);
            let entity = YahooDividendAclMapper::from_command(&cmd);
            let unchanged = multiple_dividend_cache
                .get(&key)
                .is_some_and(|existing| !dividend_content_differs(existing, &entity));
            if !unchanged {
                plan.to_save.push(entity);
            }
        }
    }
    plan
}

/// 判斷來源資料與資料庫既有股利是否有實質差異。
///
/// 只比對 Yahoo 能提供的欄位：金額與四個除權息／發放日期。盈餘分配率由 Goodinfo 另外回補，
/// Yahoo 一律給 0，拿它比對會讓每次採集都判定為有差異而反覆覆寫。
pub(super) fn dividend_content_differs(existing: &Dividend, incoming: &Dividend) -> bool {
    existing.cash_dividend != incoming.cash_dividend
        || existing.stock_dividend != incoming.stock_dividend
        || existing.sum != incoming.sum
        || existing.ex_dividend_date_cash != incoming.ex_dividend_date_cash
        || existing.ex_dividend_date_stock != incoming.ex_dividend_date_stock
        || existing.payable_date_cash != incoming.payable_date_cash
        || existing.payable_date_stock != incoming.payable_date_stock
}

/// 判斷 Yahoo 股利明細是否屬於本次回補範圍。
///
/// 回補範圍限定為目標年度與前一年度的「股利所屬年度」。剛跨年度時，去年的 Q4 或 H2
/// 常會在今年發放，因此前一年度仍需納入；未來年度與更早歷史年度則略過，避免誤寫入。
pub(super) fn should_process_dividend_year(target_year: i32, year_of_dividend: i32) -> bool {
    year_of_dividend == target_year || year_of_dividend == target_year - 1
}

/// 建立季配/半年配去重用的股利 key。
///
/// key 使用股票代號、股利所屬年度與季度，格式需與 `Dividend::key()` 一致。這可讓
/// Yahoo 回補資料與資料庫既有資料用同一套規則比對，即使實際發放年度跨到下一年也不會誤判。
pub(super) fn make_dividend_key(
    stock_symbol: &str,
    year_of_dividend: i32,
    quarter: &str,
) -> String {
    format!("{stock_symbol}-{year_of_dividend}-{quarter}")
}
