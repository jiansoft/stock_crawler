use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use futures::{StreamExt, stream};
use rust_decimal::Decimal;

use crate::{core::util, domain::quote::{
        entity::{DailyQuote as DomainDailyQuote, QuoteHistoryRecord},
        repository::QuoteRepository,
    }, infra::cache::SHARE, infra::database::repository::quote::PgQuoteRepository};

/// 計算所有上市櫃公司在指定日期的均線值與歷史高低點。
///
/// 此函數會平行處理所有股票的計算，最後進行批次資料庫更新以極大化效能。
pub async fn calculate_moving_average(date: NaiveDate) -> Result<()> {
    // 建立報價領域倉儲實例
    let repo = PgQuoteRepository::new();
    // 透過倉儲獲取指定日期的每日報價領域實體列表
    let quotes = repo.fetch_quotes_by_date(date).await?;

    // 使用並行流處理計算，但不在此處執行資料庫寫入
    let results = stream::iter(quotes)
        .map(|dq| async move { process_single_quote(dq).await })
        .buffer_unordered(util::concurrent_limit_32().expect("REASON"))
        .collect::<Vec<Result<(DomainDailyQuote, Option<QuoteHistoryRecord>)>>>()
        .await;

    // 用來儲存需要更新的日報價
    let mut quotes_to_update = Vec::new();
    // 用來儲存需要寫入的歷史高低統計紀錄
    let mut history_to_upsert = Vec::new();

    // 彙整並行處理的結果
    for res in results {
        match res {
            Ok((dq, qhr_opt)) => {
                // 將計算完成的日報價加入待更新列表
                quotes_to_update.push(dq);
                if let Some(qhr) = qhr_opt {
                    // 將有變動的歷史紀錄加入待寫入列表
                    history_to_upsert.push(qhr);
                }
            }
            // 記錄計算過程中的錯誤 log
            Err(why) => tracing::error!("Calculation error: {:?}", why),
        }
    }

    // --- 批次寫入資料庫 (效能核心) ---
    if !quotes_to_update.is_empty() {
        // 呼叫倉儲的批次更新方法寫入資料庫
        if let Err(why) = repo.batch_update_moving_average(&quotes_to_update).await {
            tracing::error!("Failed to batch update DailyQuotes: {:?}", why);
        }
    }

    // 處理歷史統計高低紀錄的寫入與快取同步
    if !history_to_upsert.is_empty() {
        for qhr in history_to_upsert {
            // 寫入/更新歷史紀錄資料庫表
            if let Err(why) = repo.save_quote_history_record(&qhr).await {
                tracing::error!("Failed to upsert history record: {:?}", why);
                continue;
            }
            // 同步更新全域記憶體快取以維持最終一致性
            if let Ok(mut guard) = SHARE.quote_history_records.write() {
                guard.insert(qhr.security_code.clone(), qhr);
            }
        }
    }

    Ok(())
}

/// 處理單一報價的計算邏輯（純計算，不涉及全域快取寫入）。
async fn process_single_quote(
    dq: DomainDailyQuote,
) -> Result<(DomainDailyQuote, Option<QuoteHistoryRecord>)> {
    let repo = PgQuoteRepository::new();
    let mut dq = dq;
    // 呼叫倉儲計算均線與年內極值
    repo.fill_moving_average(&mut dq).await?;

    // 2. 計算股價淨值比 (PBR)
    let stock = SHARE.get_stock(&dq.stock_symbol).await;
    dq.price_to_book_ratio = if let Some(s) = stock {
        // 確保淨值與收盤價皆大於零，避免除以零的錯誤
        if s.net_asset_value_per_share() > Decimal::ZERO && dq.closing_price > Decimal::ZERO {
            dq.closing_price / s.net_asset_value_per_share()
        } else {
            Decimal::ZERO
        }
    } else {
        Decimal::ZERO
    };

    // 3. 判斷是否需要更新歷史紀錄
    let qhr_opt = {
        // 讀取全域歷史紀錄快取
        let guard = SHARE
            .quote_history_records
            .read()
            .map_err(|e| anyhow!("{:?}", e))?;
        // 尋找此股票是否有舊的歷史紀錄
        let current_qhr = guard.get(&dq.stock_symbol);

        match current_qhr {
            None => {
                // 若無舊紀錄則初次建立全新歷史紀錄
                let mut new_qhr = QuoteHistoryRecord::new(dq.stock_symbol.clone());
                // 更新欄位值
                update_qhr_fields(&mut new_qhr, &dq);
                Some(new_qhr)
            }
            Some(old_qhr) => {
                // 若有舊紀錄，判定本次計算結果是否突破歷史極限
                if should_update_history(old_qhr, &dq) {
                    let mut new_qhr = old_qhr.clone();
                    // 更新歷史統計極限值與對應日期
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
fn should_update_history(old: &QuoteHistoryRecord, dq: &DomainDailyQuote) -> bool {
    // 取得當前股價淨值比，並四捨五入至小數後四位
    let price_to_book = dq.price_to_book_ratio.round_dp(4);
    if price_to_book == Decimal::ZERO {
        return false;
    }

    // 比較最高價、最低價及股價淨值比的歷史區間，判斷是否破高或破低
    dq.highest_price.round_dp(4) > old.maximum_price.round_dp(4)
        || dq.lowest_price.round_dp(4) < old.minimum_price.round_dp(4)
        || old.minimum_price.is_zero()
        || price_to_book > old.maximum_price_to_book_ratio.round_dp(4)
        || price_to_book < old.minimum_price_to_book_ratio.round_dp(4)
        || old.minimum_price_to_book_ratio.is_zero()
}

/// 更新歷史紀錄欄位。
fn update_qhr_fields(qhr: &mut QuoteHistoryRecord, dq: &DomainDailyQuote) {
    // 統一四捨五入，確保精確度
    let pbr = dq.price_to_book_ratio.round_dp(4);
    let hp = dq.highest_price.round_dp(4);
    let lp = dq.lowest_price.round_dp(4);

    // 突破歷史最高價更新
    if hp > qhr.maximum_price || qhr.maximum_price.is_zero() {
        qhr.maximum_price = hp;
        qhr.maximum_price_date_on = dq.date;
    }
    // 跌破歷史最低價更新
    if lp < qhr.minimum_price || qhr.minimum_price.is_zero() {
        qhr.minimum_price = lp;
        qhr.minimum_price_date_on = dq.date;
    }
    // 突破歷史最高 PB 更新
    if pbr > qhr.maximum_price_to_book_ratio || qhr.maximum_price_to_book_ratio.is_zero() {
        qhr.maximum_price_to_book_ratio = pbr;
        qhr.maximum_price_to_book_ratio_date_on = dq.date;
    }
    // 跌破歷史最低 PB 更新
    if pbr < qhr.minimum_price_to_book_ratio || qhr.minimum_price_to_book_ratio.is_zero() {
        qhr.minimum_price_to_book_ratio = pbr;
        qhr.minimum_price_to_book_ratio_date_on = dq.date;
    }
}

#[cfg(test)]
mod tests {
use super::*;

    #[tokio::test]
    async fn test_calculate_moving_average() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 calculate_moving_average");
        let date = NaiveDate::from_ymd_opt(2026, 2, 26);
        match calculate_moving_average(date.unwrap()).await {
            Ok(_) => {
                tracing::debug!("calculate_moving_average() 完成");
            }
            Err(why) => {
                tracing::debug!("Failed to calculate_moving_average because {:?}",
                    why);
            }
        }

        tracing::debug!("結束 calculate_moving_average");
    }
}
