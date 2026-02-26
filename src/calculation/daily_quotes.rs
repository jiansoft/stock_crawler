use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use futures::{stream, StreamExt};
use rust_decimal::Decimal;

use crate::{
    cache::SHARE,
    database::table::{daily_quote::DailyQuote, quote_history_record::QuoteHistoryRecord},
    logging, util,
};

/// 計算所有上市櫃公司在指定日期的均線值與歷史高低點。
///
/// 此函數會平行處理所有股票的計算，最後進行批次資料庫更新以極大化效能。
pub async fn calculate_moving_average(date: NaiveDate) -> Result<()> {
    let quotes = crate::database::table::daily_quote::fetch_daily_quotes_by_date(date).await?;

    // 使用並行流處理計算，但不在此處執行資料庫寫入
    let results = stream::iter(quotes)
        .map(|dq| async move { process_single_quote(dq).await })
        .buffer_unordered(util::concurrent_limit_32().expect("REASON"))
        .collect::<Vec<Result<(DailyQuote, Option<QuoteHistoryRecord>)>>>()
        .await;

    let mut quotes_to_update = Vec::new();
    let mut history_to_upsert = Vec::new();

    for res in results {
        match res {
            Ok((dq, qhr_opt)) => {
                quotes_to_update.push(dq);
                if let Some(qhr) = qhr_opt {
                    history_to_upsert.push(qhr);
                }
            }
            Err(why) => logging::error_file_async(format!("Calculation error: {:?}", why)),
        }
    }

    // --- 批次寫入資料庫 (效能核心) ---
    if !quotes_to_update.is_empty() {
        // 使用高效的批次更新方法
        if let Err(why) = DailyQuote::batch_update_moving_average(&quotes_to_update).await {
            logging::error_file_async(format!("Failed to batch update DailyQuotes: {:?}", why));
        }
    }

    if !history_to_upsert.is_empty() {
        for qhr in history_to_upsert {
            // 更新資料庫
            if let Err(why) = qhr.upsert().await {
                logging::error_file_async(format!("Failed to upsert history record: {:?}", why));
                continue;
            }
            // 資料庫更新成功後，同步回全域快取 (確保最終一致性)
            if let Ok(mut guard) = SHARE.quote_history_records.write() {
                guard.insert(qhr.security_code.clone(), qhr);
            }
        }
    }

    Ok(())
}

/// 處理單一報價的計算邏輯（純計算，不涉及全域快取寫入）。
async fn process_single_quote(
    mut dq: DailyQuote,
) -> Result<(DailyQuote, Option<QuoteHistoryRecord>)> {
    // 1. 計算均線
    dq.fill_moving_average().await?;

    // 2. 計算股價淨值比 (PBR)
    let stock = SHARE.get_stock(&dq.stock_symbol).await;
    dq.price_to_book_ratio = if let Some(s) = stock {
        if s.net_asset_value_per_share > Decimal::ZERO && dq.closing_price > Decimal::ZERO {
            dq.closing_price / s.net_asset_value_per_share
        } else {
            Decimal::ZERO
        }
    } else {
        Decimal::ZERO
    };

    // 3. 判斷是否需要更新歷史紀錄
    let qhr_opt = {
        let guard = SHARE
            .quote_history_records
            .read()
            .map_err(|e| anyhow!("{:?}", e))?;
        let current_qhr = guard.get(&dq.stock_symbol);

        match current_qhr {
            None => {
                // 初次建立
                let mut new_qhr = QuoteHistoryRecord::new(dq.stock_symbol.clone());
                update_qhr_fields(&mut new_qhr, &dq);
                Some(new_qhr)
            }
            Some(old_qhr) => {
                if should_update_history(old_qhr, &dq) {
                    let mut new_qhr = old_qhr.clone();
                    update_qhr_fields(&mut new_qhr, &dq);
                    Some(new_qhr)
                } else {
                    None
                }
            }
        }
    };

    Ok((dq, qhr_opt))
}

/// 判斷當前報價是否突破歷史紀錄。
fn should_update_history(old: &QuoteHistoryRecord, dq: &DailyQuote) -> bool {
    let price_to_book = dq.price_to_book_ratio.round_dp(4);
    if price_to_book == Decimal::ZERO {
        return false;
    }

    dq.highest_price.round_dp(4) > old.maximum_price.round_dp(4)
        || dq.lowest_price.round_dp(4) < old.minimum_price.round_dp(4)
        || old.minimum_price.is_zero()
        || price_to_book > old.maximum_price_to_book_ratio.round_dp(4)
        || price_to_book < old.minimum_price_to_book_ratio.round_dp(4)
        || old.minimum_price_to_book_ratio.is_zero()
}

/// 更新歷史紀錄欄位。
fn update_qhr_fields(qhr: &mut QuoteHistoryRecord, dq: &DailyQuote) {
    let pbr = dq.price_to_book_ratio.round_dp(4);
    let hp = dq.highest_price.round_dp(4);
    let lp = dq.lowest_price.round_dp(4);

    if hp > qhr.maximum_price || qhr.maximum_price.is_zero() {
        qhr.maximum_price = hp;
        qhr.maximum_price_date_on = dq.date;
    }
    if lp < qhr.minimum_price || qhr.minimum_price.is_zero() {
        qhr.minimum_price = lp;
        qhr.minimum_price_date_on = dq.date;
    }
    if pbr > qhr.maximum_price_to_book_ratio || qhr.maximum_price_to_book_ratio.is_zero() {
        qhr.maximum_price_to_book_ratio = pbr;
        qhr.maximum_price_to_book_ratio_date_on = dq.date;
    }
    if pbr < qhr.minimum_price_to_book_ratio || qhr.minimum_price_to_book_ratio.is_zero() {
        qhr.minimum_price_to_book_ratio = pbr;
        qhr.minimum_price_to_book_ratio_date_on = dq.date;
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_calculate_moving_average() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 calculate_moving_average".to_string());
        let date = NaiveDate::from_ymd_opt(2026, 2, 26);
        match calculate_moving_average(date.unwrap()).await {
            Ok(_) => {
                logging::debug_file_async("calculate_moving_average() 完成".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to calculate_moving_average because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 calculate_moving_average".to_string());
    }
}
