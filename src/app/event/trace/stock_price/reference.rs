//! 開盤前載入當日除權息、減資恢復買賣股票的參考價，供即時報價的異常價格過濾使用。

use std::collections::HashMap;

use anyhow::Result;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::{
    domain::dividend::{entity::ex_rights_reference_price, repository::DividendRepository},
    domain::performance::repository::CorporateActionRepository,
    infra::cache::SHARE,
    infra::database::repository::{
        corporate_action::PgCorporateActionRepository, dividend::PgDividendRepository,
    },
};

/// 算出 `date` 當天除權息、減資／分割恢復買賣股票的參考價，整批寫入 [`SHARE`]。
///
/// 前日收盤取自 `last_daily_quotes`（開盤前仍是前一交易日的收盤；停止買賣期間
/// 由收盤補值沿用停止前的收盤）。兩類事件互相獨立，其中一類查詢失敗只記 warn，
/// 另一類照常載入。沒有任何事件的交易日會寫入空集合，清掉前一天的值。
pub(super) async fn load_reference_prices(date: NaiveDate) {
    let mut prices = match corporate_action_reference_prices(date).await {
        Ok(prices) => prices,
        Err(why) => {
            tracing::warn!(
                "Failed to load corporate action reference prices because {:?}",
                why
            );
            HashMap::new()
        }
    };

    match ex_rights_reference_prices(date, &prices).await {
        Ok(ex_rights) => prices.extend(ex_rights),
        Err(why) => tracing::warn!(
            "Failed to load ex-rights reference prices because {:?}",
            why
        ),
    }

    tracing::info!(
        "載入當日參考價（除權息／減資）: date={date}, symbols={}, detail={:?}",
        prices.len(),
        prices
    );
    SHARE.set_ex_rights_reference_prices(prices);
}

/// 由 `corporate_action` 算出 `date` 當天恢復買賣股票的參考價（前收 ÷ 股數比例）。
///
/// 減資恢復買賣的漲跌幅以恢復買賣參考價計算，前收卻是停止買賣前的價格：
/// 6550 2026-09-29 減資彌補虧損，前收 9.92、參考價約 19.09，HiStock 回報的昨收
/// 仍是 9.92，成交 18.35 整天被當成異常價格過濾（單日 2,313 次）。
async fn corporate_action_reference_prices(date: NaiveDate) -> Result<HashMap<String, Decimal>> {
    let actions = PgCorporateActionRepository::new()
        .fetch_by_effective_date(date)
        .await?;

    let mut prices = HashMap::with_capacity(actions.len());
    for action in actions {
        let Some(previous_close) = SHARE
            .get_stock_last_price(&action.stock_symbol)
            .await
            .map(|quote| quote.closing_price)
        else {
            continue;
        };
        if let Some(price) = action.reference_price(previous_close) {
            prices.insert(action.stock_symbol, price);
        }
    }

    Ok(prices)
}

/// 由資料庫的股利事件算出 `date` 當天除權息股票的參考價。
///
/// 同一檔同一天有多筆股利事件時合併計算。同一天也恢復買賣者（`resumption_prices`
/// 已有該代號），除權息以恢復買賣參考價為基準，比照交易所先減資、再除權的順序。
async fn ex_rights_reference_prices(
    date: NaiveDate,
    resumption_prices: &HashMap<String, Decimal>,
) -> Result<HashMap<String, Decimal>> {
    let infos = PgDividendRepository::new()
        .fetch_stocks_with_dividends_on_date(date)
        .await?;

    // (前日收盤, 現金股利合計, 股票股利合計)
    let mut totals: HashMap<String, (Decimal, Decimal, Decimal)> = HashMap::new();
    for info in &infos {
        let (cash, stock) = info.effective_on_date();
        let previous_close = resumption_prices
            .get(&info.stock_symbol)
            .copied()
            .unwrap_or(info.closing_price);
        let entry = totals.entry(info.stock_symbol.clone()).or_insert((
            previous_close,
            Decimal::ZERO,
            Decimal::ZERO,
        ));
        entry.1 += cash;
        entry.2 += stock;
    }

    Ok(totals
        .into_iter()
        .filter_map(|(symbol, (close, cash, stock))| {
            ex_rights_reference_price(close, cash, stock).map(|price| (symbol, price))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 沒有除權息與減資事件的日子，兩類參考價都是空集合，整批寫入也不出錯。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn reference_prices_are_empty_on_a_day_without_events() {
        dotenvy::dotenv().ok();
        let date = NaiveDate::from_ymd_opt(1999, 1, 4).unwrap();
        let Ok(resumption) = corporate_action_reference_prices(date).await else {
            println!("跳過 reference_prices_are_empty_on_a_day_without_events：無資料庫連接");
            return;
        };
        assert!(resumption.is_empty());
        assert!(
            ex_rights_reference_prices(date, &resumption)
                .await
                .expect("除權息參考價")
                .is_empty()
        );
        load_reference_prices(date).await;
    }
}
