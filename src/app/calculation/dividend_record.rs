use anyhow::Result;
use rust_decimal::Decimal;

use crate::{
    domain::dividend::repository::DividendRepository,
    domain::portfolio::entity::{ReceivedDividend, ReceivedDividendItem, StockOwnershipDetail},
    domain::portfolio::repository::PortfolioRepository,
    infra::database::repository::dividend::PgDividendRepository,
    infra::database::repository::portfolio::PgPortfolioRepository,
};

/// 計算指定年份領取的股利。
///
/// 這個入口會讀取目前未賣出的持股，逐筆重算指定年度已領取的股利總表與明細表。
/// 若股利資料是後續才回補進資料庫，重新執行此流程即可把既有持股的股利領取紀錄補寫回去。
pub async fn execute(year: i32, security_codes: Option<Vec<String>>) {
    tracing::info!("計算指定年份領取的股利開始");

    let portfolio_repo = PgPortfolioRepository::new();
    let dividend_repo = PgDividendRepository::new();

    match portfolio_repo.fetch_active_holdings(security_codes).await {
        Ok(inventories) => {
            if !inventories.is_empty() {
                let tasks = inventories
                    .into_iter()
                    .map(|sod| calculate_dividend(&portfolio_repo, &dividend_repo, sod, year))
                    .collect::<Vec<_>>();
                let results = futures::future::join_all(tasks).await;
                results
                    .into_iter()
                    .filter_map(|r| r.err())
                    .for_each(|e| tracing::error!("{:?}", e));
            }
        }
        Err(why) => {
            tracing::error!("Failed to execute fetch_active_holdings because {:?}", why);
        }
    }

    tracing::info!("計算指定年份領取的股利結束");
}

/// 單一股票已領股利回補的執行結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReceivedDividendRecordBackfillSummary {
    /// 本次找到並處理的目前持股筆數。
    pub holding_count: usize,
    /// 本次找到並處理的股利發放年度數。
    pub year_count: usize,
    /// 本次實際執行的持股年度重算次數。
    pub recalculated_count: usize,
}

/// 依股票代號回補目前持股已領取的股利紀錄。
pub async fn backfill_received_dividend_records_for_stock(
    security_code: &str,
) -> Result<ReceivedDividendRecordBackfillSummary> {
    let portfolio_repo = PgPortfolioRepository::new();
    let dividend_repo = PgDividendRepository::new();

    // 只撈目前仍持有的指定股票，避免已賣出部位被重新改寫。
    let holdings = portfolio_repo
        .fetch_active_holdings(Some(vec![security_code.to_string()]))
        .await?;
    // 從已回補完成的 dividend 表自動找年份，讓呼叫端不需要猜測要補哪一年。
    let years = dividend_repo
        .fetch_years_by_security_code(security_code)
        .await?;
    let mut summary = ReceivedDividendRecordBackfillSummary {
        holding_count: holdings.len(),
        year_count: years.len(),
        recalculated_count: 0,
    };

    for holding in holdings {
        for year in &years {
            // 每個持股批次在每個已知股利年度都重算一次，確保總表與逐項明細表同步更新。
            calculate_dividend(&portfolio_repo, &dividend_repo, holding.clone(), *year).await?;
            summary.recalculated_count += 1;
        }
    }

    Ok(summary)
}

/// 計算股票於該年度可以領取的股利。
async fn calculate_dividend(
    portfolio_repo: &PgPortfolioRepository,
    dividend_repo: &PgDividendRepository,
    mut sod: StockOwnershipDetail,
    year: i32,
) -> Result<()> {
    // 先撈出該持股在指定年度、且可能與持有期間重疊的股利資料。
    let dividends = dividend_repo
        .fetch_dividends_summary_by_date(&sod.security_code, year, sod.created_time)
        .await?;

    let number_of_shares_held = Decimal::new(sod.share_quantity, 0);
    let holding_date = sod.created_time.date_naive();

    let mut total_cash = Decimal::ZERO;
    let mut total_stock = Decimal::ZERO;
    let mut total_stock_money = Decimal::ZERO;

    let mut item_commands = Vec::new();

    for dividend in &dividends {
        let (cash, stock, stock_money, total) =
            dividend.calculate_payout(holding_date, number_of_shares_held);

        if cash.is_zero() && stock.is_zero() && stock_money.is_zero() {
            // 單筆股利完全不符合資格時，不寫入逐項明細。
            continue;
        }

        total_cash += cash;
        total_stock += stock;
        total_stock_money += stock_money;

        item_commands.push(ReceivedDividendItem {
            serial: 0,
            stock_ownership_details_serial: sod.serial,
            dividend_record_detail_serial: 0, // 稍後在 Repository 內寫入時回填
            dividend_serial: dividend.serial,
            cash,
            stock,
            stock_money,
            total,
            created_time: chrono::Local::now(),
            updated_time: chrono::Local::now(),
        });
    }

    let total_sum = total_cash + total_stock_money;

    if total_sum.is_zero() {
        // 完全沒有可領股利時不寫入 0 元總表，並清掉過去可能留下的舊紀錄。
        portfolio_repo
            .delete_received_dividend(sod.serial, year)
            .await?;
        return Ok(());
    }

    let summary = ReceivedDividend {
        serial: 0,
        stock_ownership_details_serial: sod.serial,
        year,
        cash: total_cash,
        stock: total_stock,
        stock_money: total_stock_money,
        total: total_sum,
        created_time: chrono::Local::now(),
        updated_time: chrono::Local::now(),
    };

    // 1. 寫入年度總表與配發細項
    portfolio_repo
        .save_received_dividend(&summary, &item_commands)
        .await?;

    // 2. 重新計算該股票其累積的領取股利並更新持股明細
    let (accum_cash, accum_stock, accum_stock_money, _accum_total) = portfolio_repo
        .calculate_accumulated_dividends(sod.serial)
        .await?;

    sod.update_cumulate_dividends(accum_cash, accum_stock, accum_stock_money);

    portfolio_repo.update_holding_dividends(&sod).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{domain::dividend::entity::Dividend, infra::cache::SHARE};
    use chrono::{Local, NaiveDate, TimeZone};
    use rust_decimal_macros::dec;

    use super::*;

    /// 驗證持有日必須嚴格早於除權息日才符合股利領取資格。
    #[test]
    fn test_is_holding_eligible_for_ex_date_requires_strictly_earlier_date() {
        let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
        let mut d = Dividend {
            serial: 0,
            year: 2025,
            year_of_dividend: 2024,
            quarter: "".to_string(),
            security_code: "".to_string(),
            earnings_cash_dividend: Decimal::ZERO,
            capital_reserve_cash_dividend: Decimal::ZERO,
            cash_dividend: Decimal::ZERO,
            earnings_stock_dividend: Decimal::ZERO,
            capital_reserve_stock_dividend: Decimal::ZERO,
            stock_dividend: Decimal::ZERO,
            sum: Decimal::ZERO,
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            ex_dividend_date_cash: "2025-06-21".to_string(),
            ex_dividend_date_stock: "2025-06-21".to_string(),
            payable_date_cash: "".to_string(),
            payable_date_stock: "".to_string(),
            created_time: Local::now(),
            updated_time: Local::now(),
        };

        assert!(d.is_eligible_for_cash(holding_date));

        d.ex_dividend_date_cash = "2025-06-20".to_string();
        assert!(!d.is_eligible_for_cash(holding_date));

        d.ex_dividend_date_cash = "2025-06-19".to_string();
        assert!(!d.is_eligible_for_cash(holding_date));

        d.ex_dividend_date_cash = "-".to_string();
        assert!(!d.is_eligible_for_cash(holding_date));
    }

    /// 驗證現金股利與股票股利會依各自的除息日、除權日分開判斷。
    #[test]
    fn test_calculate_eligible_dividend_amounts_checks_cash_and_stock_dates_separately() {
        let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
        let number_of_shares_held = Decimal::new(1000, 0);
        let d = Dividend {
            serial: 0,
            year: 2025,
            year_of_dividend: 2024,
            quarter: "".to_string(),
            security_code: "".to_string(),
            earnings_cash_dividend: Decimal::ZERO,
            capital_reserve_cash_dividend: Decimal::ZERO,
            cash_dividend: dec!(2.5),
            earnings_stock_dividend: Decimal::ZERO,
            capital_reserve_stock_dividend: Decimal::ZERO,
            stock_dividend: dec!(1.2),
            sum: dec!(3.7),
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            ex_dividend_date_cash: "2025-06-20".to_string(),
            ex_dividend_date_stock: "2025-06-21".to_string(),
            payable_date_cash: "".to_string(),
            payable_date_stock: "".to_string(),
            created_time: Local::now(),
            updated_time: Local::now(),
        };

        let (cash, stock, stock_money, total) =
            d.calculate_payout(holding_date, number_of_shares_held);

        // 持有日等於除息日，不可領現金；但仍早於除權日，所以股票股利仍可領。
        assert_eq!(cash, Decimal::ZERO);
        assert_eq!(stock_money, dec!(1200));
        assert_eq!(stock, dec!(120));
        assert_eq!(total, dec!(1200));
    }

    /// 驗證持有日不早於除息日與除權日時，整筆股利不會產生可領金額。
    #[test]
    fn test_calculate_eligible_dividend_amounts_returns_zero_when_holding_is_not_eligible() {
        let holding_date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let number_of_shares_held = Decimal::new(1000, 0);
        let d = Dividend {
            serial: 0,
            year: 2025,
            year_of_dividend: 2024,
            quarter: "".to_string(),
            security_code: "".to_string(),
            earnings_cash_dividend: Decimal::ZERO,
            capital_reserve_cash_dividend: Decimal::ZERO,
            cash_dividend: dec!(2.5),
            earnings_stock_dividend: Decimal::ZERO,
            capital_reserve_stock_dividend: Decimal::ZERO,
            stock_dividend: dec!(1.2),
            sum: dec!(3.7),
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            ex_dividend_date_cash: "2024-06-20".to_string(),
            ex_dividend_date_stock: "2024-06-21".to_string(),
            payable_date_cash: "".to_string(),
            payable_date_stock: "".to_string(),
            created_time: Local::now(),
            updated_time: Local::now(),
        };

        let (cash, stock, stock_money, total) =
            d.calculate_payout(holding_date, number_of_shares_held);
        assert!(cash.is_zero());
        assert!(stock.is_zero());
        assert!(stock_money.is_zero());
        assert!(total.is_zero());
    }

    #[test]
    fn eligible_dividend_amounts_supports_cash_only_and_stock_only_dates() {
        let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
        let number_of_shares_held = Decimal::new(1000, 0);
        let mut d = Dividend {
            serial: 0,
            year: 2025,
            year_of_dividend: 2024,
            quarter: "".to_string(),
            security_code: "".to_string(),
            earnings_cash_dividend: Decimal::ZERO,
            capital_reserve_cash_dividend: Decimal::ZERO,
            cash_dividend: dec!(1.5),
            earnings_stock_dividend: Decimal::ZERO,
            capital_reserve_stock_dividend: Decimal::ZERO,
            stock_dividend: dec!(0.5),
            sum: dec!(2.0),
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            ex_dividend_date_cash: "2025-06-21".to_string(),
            ex_dividend_date_stock: "-".to_string(),
            payable_date_cash: "".to_string(),
            payable_date_stock: "".to_string(),
            created_time: Local::now(),
            updated_time: Local::now(),
        };

        let (cash, _stock, stock_money, total) =
            d.calculate_payout(holding_date, number_of_shares_held);
        assert_eq!(cash, dec!(1500));
        assert_eq!(stock_money, Decimal::ZERO);
        assert_eq!(total, dec!(1500));

        d.ex_dividend_date_cash = "-".to_string();
        d.ex_dividend_date_stock = "2025-06-21".to_string();
        let (cash, stock, stock_money, total) =
            d.calculate_payout(holding_date, number_of_shares_held);
        assert_eq!(cash, Decimal::ZERO);
        assert_eq!(stock_money, dec!(500));
        assert_eq!(stock, dec!(50));
        assert_eq!(total, dec!(500));
    }

    #[test]
    fn eligible_dividend_amounts_treats_invalid_or_unpublished_dates_as_ineligible() {
        let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
        let number_of_shares_held = Decimal::new(1000, 0);
        let d = Dividend {
            serial: 0,
            year: 2025,
            year_of_dividend: 2024,
            quarter: "".to_string(),
            security_code: "".to_string(),
            earnings_cash_dividend: Decimal::ZERO,
            capital_reserve_cash_dividend: Decimal::ZERO,
            cash_dividend: dec!(1.5),
            earnings_stock_dividend: Decimal::ZERO,
            capital_reserve_stock_dividend: Decimal::ZERO,
            stock_dividend: dec!(0.5),
            sum: dec!(2.0),
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            ex_dividend_date_cash: "".to_string(),
            ex_dividend_date_stock: "2025/06/21".to_string(),
            payable_date_cash: "".to_string(),
            payable_date_stock: "".to_string(),
            created_time: Local::now(),
            updated_time: Local::now(),
        };

        let (cash, stock, stock_money, total) =
            d.calculate_payout(holding_date, number_of_shares_held);
        assert!(cash.is_zero());
        assert!(stock.is_zero());
        assert!(stock_money.is_zero());
        assert!(total.is_zero());
    }

    #[test]
    fn eligible_dividend_amounts_returns_zero_for_zero_shares() {
        let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
        let d = Dividend {
            serial: 0,
            year: 2025,
            year_of_dividend: 2024,
            quarter: "".to_string(),
            security_code: "".to_string(),
            earnings_cash_dividend: Decimal::ZERO,
            capital_reserve_cash_dividend: Decimal::ZERO,
            cash_dividend: dec!(1.5),
            earnings_stock_dividend: Decimal::ZERO,
            capital_reserve_stock_dividend: Decimal::ZERO,
            stock_dividend: dec!(0.5),
            sum: dec!(2.0),
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            ex_dividend_date_cash: "2025-06-21".to_string(),
            ex_dividend_date_stock: "2025-06-21".to_string(),
            payable_date_cash: "".to_string(),
            payable_date_stock: "".to_string(),
            created_time: Local::now(),
            updated_time: Local::now(),
        };

        let (cash, stock, stock_money, total) = d.calculate_payout(holding_date, Decimal::ZERO);
        assert!(cash.is_zero());
        assert!(stock.is_zero());
        assert!(stock_money.is_zero());
        assert!(total.is_zero());
    }

    #[tokio::test]
    async fn test_backfill_received_dividend_records_for_stock_backfills_after_dividend_insert() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let security_code = "0056";
        let summary = backfill_received_dividend_records_for_stock(security_code)
            .await
            .expect("backfill received dividend records by stock");

        dbg!(summary);
    }

    #[tokio::test]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        tracing::debug!("開始 calculate");
        for i in 2025..2026 {
            execute(i, None).await;
            tracing::debug!("calculate({}) 完成", i);
        }
        tracing::debug!("結束 calculate");
    }

    #[tokio::test]
    async fn test_calculate_dividend() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 calculate_dividend");

        let portfolio_repo = PgPortfolioRepository::new();
        let dividend_repo = PgDividendRepository::new();

        let sod = StockOwnershipDetail::reconstitute(
            102,
            "2882".to_string(),
            2,
            300,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            false,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Local.with_ymd_and_hms(2023, 4, 9, 0, 0, 0).unwrap(),
        );

        match calculate_dividend(&portfolio_repo, &dividend_repo, sod, 2023).await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to calculate_dividend because {:?}", why);
            }
        }
        tracing::debug!("結束 calculate_dividend");
    }
}
