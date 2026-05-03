use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::{
    database::{
        self,
        table::{
            dividend, dividend_record_detail::DividendRecordDetail, dividend_record_detail_more,
            stock_ownership_details,
        },
    },
    logging,
};

/// 計算指定年份領取的股利。
///
/// 這個入口會讀取目前未賣出的持股，逐筆重算指定年度已領取的股利總表與明細表。
/// 若股利資料是後續才回補進資料庫，重新執行此流程即可把既有持股的股利領取紀錄補寫回去。
pub async fn execute(year: i32, security_codes: Option<Vec<String>>) {
    logging::info_file_async("計算指定年份領取的股利開始".to_string());

    match stock_ownership_details::StockOwnershipDetail::fetch(security_codes).await {
        Ok(inventories) => {
            if !inventories.is_empty() {
                let tasks = inventories
                    .into_iter()
                    .map(|sod| calculate_dividend(sod, year))
                    .collect::<Vec<_>>();
                let results = futures::future::join_all(tasks).await;
                results
                    .into_iter()
                    .filter_map(|r| r.err())
                    .for_each(|e| logging::error_file_async(format!("{:?}", e)));
            }
        }
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to execute StockOwnershipDetail::fetch because {:?}",
                why
            ));
        }
    }

    logging::info_file_async("計算指定年份領取的股利結束".to_string());
}

/// 單一股票已領股利回補的執行結果。
///
/// 此結構用於回報只輸入股票代號時，實際找到多少筆目前持股、多少個股利發放年度，
/// 以及總共執行多少次「持股 x 年度」的重算。
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
///
/// 呼叫端只需要提供股票代號；流程會自動：
/// 1. 讀取該股票目前尚未賣出的持股。
/// 2. 從 `dividend` 表找出該股票已有資料的所有發放年度。
/// 3. 逐筆持股、逐年度呼叫 `calculate_dividend`，重算總表與逐項明細表。
///
/// 這個函式適合在歷史股利資料事後回補完成後使用，避免既有持股缺少
/// `dividend_record_detail` 與 `dividend_record_detail_more` 紀錄。
///
/// # 參數
///
/// - `security_code`：要回補已領股利紀錄的股票代號，例如 `2886`。
///
/// # 錯誤
///
/// 查詢目前持股、查詢股利年度或任一筆持股年度重算失敗時會回傳 `Err`。
pub async fn backfill_received_dividend_records_for_stock(
    security_code: &str,
) -> Result<ReceivedDividendRecordBackfillSummary> {
    // 只撈目前仍持有的指定股票，避免已賣出部位被重新改寫。
    let holdings =
        stock_ownership_details::StockOwnershipDetail::fetch(Some(vec![security_code.to_string()]))
            .await?;
    // 從已回補完成的 dividend 表自動找年份，讓呼叫端不需要猜測要補哪一年。
    let years = dividend::Dividend::fetch_years_by_security_code(security_code).await?;
    let mut summary = ReceivedDividendRecordBackfillSummary {
        holding_count: holdings.len(),
        year_count: years.len(),
        recalculated_count: 0,
    };

    for holding in holdings {
        for year in &years {
            // 每個持股批次在每個已知股利年度都重算一次，確保總表與逐項明細表同步更新。
            calculate_dividend(holding.clone(), *year).await?;
            summary.recalculated_count += 1;
        }
    }

    Ok(summary)
}

/// 單筆持股在單一股利明細上，依除息/除權日換算實際可領取金額。
///
/// 現金股利與股票股利必須分開判斷：
/// - 持有日必須嚴格早於除息日，才能領取現金股利。
/// - 持有日必須嚴格早於除權日，才能領取股票股利。
///
/// 這樣可以避免持有日剛好落在除息與除權之間時，把不該領的那一側股利一併算入。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct EligibleDividendAmounts {
    /// 本筆持股可領取的現金股利總額（元）。
    cash: Decimal,
    /// 本筆持股可領取的股票股利股數（股）。
    stock: Decimal,
    /// 本筆持股可領取的股票股利金額（元）。
    stock_money: Decimal,
    /// 本筆持股可領取的股利總額（元）。
    total: Decimal,
}

impl EligibleDividendAmounts {
    /// 判斷本筆計算結果是否沒有任何可領取股利。
    ///
    /// 當現金、股票股利金額與股票股利股數全為 0 時，代表持有日不符合
    /// 本次除息/除權資格，後續不應寫入總表或逐項明細表。
    fn is_zero(&self) -> bool {
        self.cash.is_zero()
            && self.stock.is_zero()
            && self.stock_money.is_zero()
            && self.total.is_zero()
    }
}

/// 判斷持有日是否符合單一除權息事件的領取資格。
///
/// 只有當持有日嚴格早於除權息日，才視為這筆持股有資格領取股利。
/// 若日期字串不是 `YYYY-MM-DD`、是 `-`、空字串或尚未公布，則視為不可領取。
fn is_holding_eligible_for_ex_date(holding_date: NaiveDate, ex_date: &str) -> bool {
    // 只有能正確解析的實際日期才可參與判斷，未知日期不能誤判為可領取。
    let Ok(ex_date) = NaiveDate::parse_from_str(ex_date, "%Y-%m-%d") else {
        return false;
    };

    holding_date < ex_date
}

/// 依持有日與股利明細，計算本筆持股實際可領取的金額。
///
/// 現金與股票股利會各自檢查對應的除息/除權日，不會因為同一列股利資料被查出，
/// 就把兩種股利都直接算入。
fn calculate_eligible_dividend_amounts(
    holding_date: NaiveDate,
    dividend: &dividend::Dividend,
    number_of_shares_held: Decimal,
) -> EligibleDividendAmounts {
    let cash = if is_holding_eligible_for_ex_date(holding_date, &dividend.ex_dividend_date1) {
        dividend.cash_dividend * number_of_shares_held
    } else {
        Decimal::ZERO
    };

    let stock_money = if is_holding_eligible_for_ex_date(holding_date, &dividend.ex_dividend_date2)
    {
        dividend.stock_dividend * number_of_shares_held
    } else {
        Decimal::ZERO
    };
    let stock = stock_money / dec!(10);

    EligibleDividendAmounts {
        cash,
        stock,
        stock_money,
        total: cash + stock_money,
    }
}

/// 刪除指定持股在指定年度的股利領取紀錄。
///
/// 當回補重算後確認該持股在該年度沒有任何可領取股利時，會先刪除逐項明細，
/// 再刪除年度總表，避免留下舊的 0 元紀錄或過去誤算的不該領股利。
///
/// # 參數
///
/// - `stock_ownership_details_serial`：持股明細序號。
/// - `year`：要清除的股利發放年度。
///
/// # 錯誤
///
/// 取得交易或刪除 SQL 執行失敗時會回傳 `Err`。
async fn delete_dividend_record_for_year(
    stock_ownership_details_serial: i64,
    year: i32,
) -> Result<()> {
    let mut tx = database::get_tx().await?;

    // 先刪逐項明細，避免總表刪除後留下孤兒明細。
    sqlx::query(
        r#"
DELETE FROM dividend_record_detail_more
WHERE stock_ownership_details_serial = $1
  AND dividend_record_detail_serial IN (
      SELECT serial
      FROM dividend_record_detail
      WHERE stock_ownership_details_serial = $1 AND year = $2
  );
"#,
    )
    .bind(stock_ownership_details_serial)
    .bind(year)
    .execute(&mut *tx)
    .await?;

    // 再刪年度總表；若原本沒有紀錄，DELETE 會是 no-op。
    sqlx::query(
        r#"
DELETE FROM dividend_record_detail
WHERE stock_ownership_details_serial = $1 AND year = $2;
"#,
    )
    .bind(stock_ownership_details_serial)
    .bind(year)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(())
}

/// 計算股票於該年度可以領取的股利。
///
/// 此函式會同時重算 `dividend_record_detail` 與 `dividend_record_detail_more`，
/// 並回寫持股的累積股利欄位。若股利資料是後補的，只要持有日早於除息/除權日，
/// 重新執行後就會補齊領取紀錄。
async fn calculate_dividend(
    mut sod: stock_ownership_details::StockOwnershipDetail,
    year: i32,
) -> Result<()> {
    // 先撈出該持股在指定年度、且可能與持有期間重疊的股利資料。
    let mut d = dividend::Dividend::new();
    d.security_code = sod.security_code.to_string();
    d.year = year;
    let dividends = dividend::Dividend::fetch_dividends_summary_by_date(
        &d.security_code,
        d.year,
        sod.created_time,
    )
    .await?;

    let number_of_shares_held = Decimal::new(sod.share_quantity, 0);
    let holding_date = sod.created_time.date_naive();
    // 總表金額必須與後續逐筆明細使用完全相同的資格判斷，避免總表/明細不一致。
    let dividend_sum =
        dividends
            .iter()
            .fold(EligibleDividendAmounts::default(), |mut acc, dividend| {
                let eligible = calculate_eligible_dividend_amounts(
                    holding_date,
                    dividend,
                    number_of_shares_held,
                );
                acc.cash += eligible.cash;
                acc.stock += eligible.stock;
                acc.stock_money += eligible.stock_money;
                acc.total += eligible.total;
                acc
            });

    if dividend_sum.is_zero() {
        // 完全沒有可領股利時不寫入 0 元總表，並清掉過去可能留下的舊紀錄。
        delete_dividend_record_for_year(sod.serial, year).await?;
        return Ok(());
    }

    let mut drd = DividendRecordDetail::new(
        sod.serial,
        year,
        dividend_sum.cash,
        dividend_sum.stock,
        dividend_sum.stock_money,
        dividend_sum.total,
    );

    let mut tx_option = database::get_tx().await.ok();
    //更新股利領取記錄
    let dividend_record_detail_serial = match drd.upsert(&mut tx_option).await {
        Ok(serial) => serial,
        Err(why) => {
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to execute upsert query for dividend_record_detail because {:?}",
                why
            ));
        }
    };

    for dividend in dividends {
        // 回寫明細時也必須逐筆檢查持有日，避免把不符合資格的一側股利寫入。
        let eligible =
            calculate_eligible_dividend_amounts(holding_date, &dividend, number_of_shares_held);
        if eligible.is_zero() {
            // 單筆股利完全不符合資格時，不寫入逐項明細。
            continue;
        }

        let mut rdrm = dividend_record_detail_more::DividendRecordDetailMore::new(
            sod.serial,
            dividend_record_detail_serial,
            dividend.serial,
            eligible.cash,
            eligible.stock,
            eligible.stock_money,
            eligible.total,
        );

        if let Err(why) = rdrm.upsert(&mut tx_option).await {
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to execute upsert query for dividend_record_detail_more because {:?}",
                why
            ));
        }

        // 計算指定股票其累積的領取股利
        let cumulate_dividend = match drd.fetch_cumulate_dividend(&mut tx_option).await {
            Ok(cd) => cd,
            Err(why) => {
                if let Some(tx) = tx_option {
                    tx.rollback().await?;
                }
                return Err(anyhow!(
                    "Failed to execute calculate_cumulate_dividend because {:?}",
                    why
                ));
            }
        };

        sod.cumulate_dividends_cash = cumulate_dividend.cash;
        sod.cumulate_dividends_stock_money = cumulate_dividend.stock_money;
        sod.cumulate_dividends_stock = cumulate_dividend.stock;
        sod.cumulate_dividends_total = cumulate_dividend.total;

        if let Err(why) = sod.update_cumulate_dividends(&mut tx_option).await {
            if let Some(tx) = tx_option {
                tx.rollback().await?;
            }
            return Err(anyhow!(
                "Failed to execute update_cumulate_dividends because {:?}",
                why
            ));
        }
    }

    if let Some(tx) = tx_option {
        tx.commit().await?;
    }

    Ok(())
}

/*/// 計算指定年份領取的股利
pub async fn calculate(year: i32) {
    logging::info_file_async("計算指定年份領取的股利開始".to_string());
    // 先取得庫存股票
    match table::inventory::fetch().await {
        Ok(mut inventories) => {
            for item in inventories.iter_mut() {
                match item.calculate_dividend(year).await {
                    Ok(drd) => match drd.calculate_cumulate_dividend().await {
                        Ok(cumulate_dividend) => {
                            let (cash, stock_money, stock, total) = cumulate_dividend;
                            item.cumulate_cash = cash;
                            item.cumulate_stock_money = stock_money;
                            item.cumulate_stock = stock;
                            item.cumulate_total = total;
                            if let Err(why) = item.update_cumulate_dividends().await {
                                logging::error_file_async(format!(
                                    "Failed to update_cumulate_dividends because {:?}",
                                    why
                                ));
                            }
                        }
                        Err(why) => {
                            logging::error_file_async(format!(
                                "Failed to calculate_cumulate_dividend because {:?}",
                                why
                            ));
                        }
                    },
                    Err(why) => {
                        logging::error_file_async(format!(
                            "Failed to calculate_dividend because {:?}",
                            why
                        ));
                    }
                };
            }
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to inventory::fetch because {:?}", why));
        }
    }

    logging::info_file_async("計算指定年份領取的股利結束".to_string());
}*/

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};

    use crate::{cache::SHARE, logging};

    use super::*;

    /// 驗證持有日必須嚴格早於除權息日才符合股利領取資格。
    ///
    /// 此測試涵蓋早於、等於、晚於與無效日期，避免未知或尚未公布的日期被誤判為可領取。
    #[test]
    fn test_is_holding_eligible_for_ex_date_requires_strictly_earlier_date() {
        let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();

        assert!(is_holding_eligible_for_ex_date(holding_date, "2025-06-21"));
        assert!(!is_holding_eligible_for_ex_date(holding_date, "2025-06-20"));
        assert!(!is_holding_eligible_for_ex_date(holding_date, "2025-06-19"));
        assert!(!is_holding_eligible_for_ex_date(holding_date, "-"));
    }

    /// 驗證現金股利與股票股利會依各自的除息日、除權日分開判斷。
    ///
    /// 當持有日等於除息日但早於除權日時，測試會確認現金股利不會被算入，
    /// 但股票股利仍會依除權資格正常寫入可領金額。
    #[test]
    fn test_calculate_eligible_dividend_amounts_checks_cash_and_stock_dates_separately() {
        let holding_date = NaiveDate::from_ymd_opt(2025, 6, 20).unwrap();
        let number_of_shares_held = Decimal::new(1000, 0);
        let mut dividend = dividend::Dividend::new();
        dividend.cash_dividend = dec!(2.5);
        dividend.stock_dividend = dec!(1.2);
        dividend.ex_dividend_date1 = "2025-06-20".to_string();
        dividend.ex_dividend_date2 = "2025-06-21".to_string();

        let eligible =
            calculate_eligible_dividend_amounts(holding_date, &dividend, number_of_shares_held);

        // 持有日等於除息日，不可領現金；但仍早於除權日，所以股票股利仍可領。
        assert_eq!(eligible.cash, Decimal::ZERO);
        assert_eq!(eligible.stock_money, dec!(1200));
        assert_eq!(eligible.stock, dec!(120));
        assert_eq!(eligible.total, dec!(1200));
    }

    /// 驗證持有日不早於除息日與除權日時，整筆股利不會產生可領金額。
    ///
    /// 此情境用來保護回補流程：若持股是 `2025-01-01` 才建立，
    /// 則 2024 年以前已除息或除權的股利不應寫入領取總表與逐項明細。
    #[test]
    fn test_calculate_eligible_dividend_amounts_returns_zero_when_holding_is_not_eligible() {
        let holding_date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let number_of_shares_held = Decimal::new(1000, 0);
        let mut dividend = dividend::Dividend::new();
        dividend.cash_dividend = dec!(2.5);
        dividend.stock_dividend = dec!(1.2);
        dividend.ex_dividend_date1 = "2024-06-20".to_string();
        dividend.ex_dividend_date2 = "2024-06-21".to_string();

        let eligible =
            calculate_eligible_dividend_amounts(holding_date, &dividend, number_of_shares_held);

        assert!(eligible.is_zero());
    }

    /// 驗證只輸入股票代號即可回補該股目前持股的已領股利紀錄。
    ///
    /// 此測試使用本機既有資料庫資料，直接指定股票代號並呼叫回補入口。
    /// 回補入口會自動找出目前持股與該股票已有股利資料的年度，逐年重算
    /// `dividend_record_detail` 與 `dividend_record_detail_more`。
    #[tokio::test]
    #[ignore]
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
    #[ignore]
    async fn test_calculate() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 calculate".to_string());
        for i in 2025..2026 {
            execute(i, None).await;
            logging::debug_file_async(format!("calculate({}) 完成", i));
        }
        logging::debug_file_async("結束 calculate".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_calculate_dividend() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 calculate_dividend".to_string());
        let mut sod = stock_ownership_details::StockOwnershipDetail::new();
        sod.serial = 102;
        sod.security_code = "2882".to_string();
        sod.member_id = 2;
        sod.share_quantity = 300;
        sod.created_time = Local.with_ymd_and_hms(2023, 4, 9, 0, 0, 0).unwrap();
        match calculate_dividend(sod, 2023).await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to calculate_dividend because {:?}",
                    why
                ));
            }
        }
        logging::debug_file_async("結束 calculate_dividend".to_string());
    }
}
