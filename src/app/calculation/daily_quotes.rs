use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use futures::{StreamExt, stream};
use rust_decimal::Decimal;

use crate::{
    core::util,
    domain::quote::{
        entity::{DailyQuote as DomainDailyQuote, QuoteHistoryRecord},
        repository::QuoteRepository,
    },
    infra::cache::SHARE,
    infra::database::repository::quote::PgQuoteRepository,
};

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

/// 重算指定股票自 `from` 起（含）的均線與年內統計，回傳實際更新的列數。
///
/// 回補或替換過去日期的行情後使用（每日收盤只算當天，補進的日子不會回頭
/// 修正之後的均線）。同時依日K重建這些股票的歷史最高、最低價。每批 [`RECALCULATE_BATCH`] 檔交給資料庫以視窗函數一次算完，
/// 任一批失敗即回傳錯誤；已完成的批次不受影響，重跑只會更新仍有差異的列。
pub async fn recalculate_moving_averages(
    repository: &dyn QuoteRepository,
    stock_symbols: &[String],
    from: NaiveDate,
) -> Result<u64> {
    let mut updated = 0;
    let mut extremes = 0;
    for (index, batch) in stock_symbols.chunks(RECALCULATE_BATCH).enumerate() {
        let rows = repository.recalculate_moving_averages(batch, from).await?;
        updated += rows;
        extremes += repository
            .rebuild_quote_history_price_extremes(batch)
            .await?;
        tracing::info!(
            batch = index + 1,
            symbols = batch.len(),
            rows,
            from = %from,
            "重算均線"
        );
    }
    if extremes > 0 {
        // 每日收盤拿記憶體快取裡的舊紀錄比較並整筆寫回，不重新載入會把剛重建的極值蓋掉。
        SHARE.reload_quote_history_records().await;
    }
    Ok(updated)
}

/// 重算 `from`（含）之後的均線；`stock_symbols` 為 `None` 時重算這段期間有報價的所有代號。
///
/// 手動回補用：替換過去某天的全市場行情（`backfill::quote::execute`）或整批補歷史後，
/// 從那天起重算，之後最多 240 個交易日受影響的均線一併修正。
pub async fn recalculate_moving_averages_since(
    from: NaiveDate,
    stock_symbols: Option<Vec<String>>,
) -> Result<u64> {
    let repository = PgQuoteRepository::new();
    let stock_symbols = match stock_symbols {
        Some(symbols) => symbols,
        None => repository.fetch_symbols_quoted_since(from).await?,
    };
    recalculate_moving_averages(&repository, &stock_symbols, from).await
}

/// 重算均線時每批的代號數。一檔十幾年約三千多列，五十檔一批的視窗計算
/// 在正式庫只要數秒，單一 UPDATE 也不會一次鎖住太多列。
const RECALCULATE_BATCH: usize = 50;

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
                // 若有舊紀錄，套用本次報價後有任何極值改變才寫回
                let mut new_qhr = old_qhr.clone();
                update_qhr_fields(&mut new_qhr, &dq);
                (new_qhr != *old_qhr).then_some(new_qhr)
            }
        }
    };

    Ok((dq, qhr_opt))
}

/// 以一筆日報價更新歷史極值。
///
/// 價格與股價淨值比分開判斷：ETF、沒有每股淨值的股票淨值比是 0，舊版遇到 0 就整筆略過，
/// 連創新高、新低的價格都沒有記到（2026-10-06 盤點有 484 檔最高價、415 檔最低價落後）。
/// 價格或淨值比為 0（無成交、無淨值）不算新低。
fn update_qhr_fields(qhr: &mut QuoteHistoryRecord, dq: &DomainDailyQuote) {
    // 統一四捨五入，確保精確度
    let pbr = dq.price_to_book_ratio.round_dp(4);
    let hp = dq.highest_price.round_dp(4);
    let lp = dq.lowest_price.round_dp(4);

    // 突破歷史最高價更新
    if hp > qhr.maximum_price {
        qhr.maximum_price = hp;
        qhr.maximum_price_date_on = dq.date;
    }
    // 跌破歷史最低價更新
    if lp > Decimal::ZERO && (lp < qhr.minimum_price || qhr.minimum_price.is_zero()) {
        qhr.minimum_price = lp;
        qhr.minimum_price_date_on = dq.date;
    }
    if pbr <= Decimal::ZERO {
        return;
    }
    // 突破歷史最高 PB 更新
    if pbr > qhr.maximum_price_to_book_ratio {
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
    use crate::domain::quote::test_double::CountingQuoteRepository;
    use rust_decimal_macros::dec;

    fn quote(date: u32, high: Decimal, low: Decimal, pbr: Decimal) -> DomainDailyQuote {
        DomainDailyQuote {
            date: NaiveDate::from_ymd_opt(2026, 5, date).expect("日期應合法"),
            highest_price: high,
            lowest_price: low,
            price_to_book_ratio: pbr,
            ..Default::default()
        }
    }

    /// 淨值比是 0（ETF）時，價格新高、新低仍要記錄，淨值比欄位不動。
    #[test]
    fn update_qhr_fields_records_prices_without_price_to_book() {
        let mut qhr = QuoteHistoryRecord::new("0050".to_string());
        update_qhr_fields(&mut qhr, &quote(4, dec!(14.74), dec!(10.75), Decimal::ZERO));
        update_qhr_fields(&mut qhr, &quote(26, dec!(16.63), dec!(11), Decimal::ZERO));

        assert_eq!(qhr.maximum_price, dec!(16.63));
        assert_eq!(qhr.maximum_price_date_on.to_string(), "2026-05-26");
        assert_eq!(qhr.minimum_price, dec!(10.75));
        assert_eq!(qhr.minimum_price_date_on.to_string(), "2026-05-04");
        assert_eq!(qhr.maximum_price_to_book_ratio, Decimal::ZERO);
        assert_eq!(qhr.minimum_price_to_book_ratio, Decimal::ZERO);
    }

    /// 無成交（最低價 0）不算新低；淨值比照常更新極值。
    #[test]
    fn update_qhr_fields_ignores_zero_lows() {
        let mut qhr = QuoteHistoryRecord::new("2330".to_string());
        update_qhr_fields(&mut qhr, &quote(4, dec!(100), dec!(90), dec!(2.5)));
        update_qhr_fields(
            &mut qhr,
            &quote(5, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO),
        );
        update_qhr_fields(&mut qhr, &quote(6, dec!(95), dec!(91), dec!(2.1)));

        assert_eq!(qhr.minimum_price, dec!(90));
        assert_eq!(qhr.maximum_price_to_book_ratio, dec!(2.5));
        assert_eq!(qhr.minimum_price_to_book_ratio, dec!(2.1));
    }

    /// 代號依批次大小切開，每批都從同一個起始日重算，更新列數累加。
    #[tokio::test]
    async fn recalculate_moving_averages_splits_symbols_into_batches() {
        let repository = CountingQuoteRepository::default();
        let symbols: Vec<String> = (0..RECALCULATE_BATCH + 1)
            .map(|i| format!("{:04}", 1000 + i))
            .collect();
        let from = NaiveDate::from_ymd_opt(2019, 7, 1).expect("日期應合法");

        let updated = recalculate_moving_averages(&repository, &symbols, from)
            .await
            .expect("重算應成功");

        let calls = repository.recalculated();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0.len(), RECALCULATE_BATCH);
        assert_eq!(calls[1].0, vec![symbols[RECALCULATE_BATCH].clone()]);
        assert!(calls.iter().all(|(_, date)| *date == from));
        assert_eq!(repository.rebuilt_extremes(), symbols);
        // 假倉儲每檔回報一列。
        assert_eq!(updated, symbols.len() as u64);
    }

    /// 沒有代號就不碰資料庫。
    #[tokio::test]
    async fn recalculate_moving_averages_skips_an_empty_list() {
        let repository = CountingQuoteRepository::default();
        let from = NaiveDate::from_ymd_opt(2019, 7, 1).expect("日期應合法");

        let updated = recalculate_moving_averages(&repository, &[], from)
            .await
            .expect("空清單應成功");

        assert_eq!(updated, 0);
        assert!(repository.recalculated().is_empty());
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_calculate_moving_average() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 calculate_moving_average");
        let date = NaiveDate::from_ymd_opt(2026, 2, 26);
        match calculate_moving_average(date.unwrap()).await {
            Ok(_) => {
                tracing::debug!("calculate_moving_average() 完成");
            }
            Err(why) => {
                tracing::debug!("Failed to calculate_moving_average because {:?}", why);
            }
        }

        tracing::debug!("結束 calculate_moving_average");
    }
}
