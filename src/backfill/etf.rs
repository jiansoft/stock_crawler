use std::fmt::Write;

use crate::{
    backfill, bot,
    bot::telegram::Telegram,
    cache::SHARE,
    crawler::{share::EtfInfo, tpex, twse},
    database::table,
    declare::StockExchangeMarket,
    logging, rpc,
    util::datetime::Weekend,
};
use anyhow::{anyhow, Result};
use chrono::Local;
use scopeguard::defer;

/// 執行台股 ETF 資訊的同步與更新。
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    logging::info_file_async("更新台股 ETF 資訊開始");
    defer! {
       logging::info_file_async("更新台股 ETF 資訊結束");
    }

    // 1. 抓取上市 ETF 資料
    match twse::etf::visit().await {
        Ok(items) => update_stocks(items).await?,
        Err(why) => logging::error_file_async(format!("處理上市 ETF 市場失敗: {:?}", why)),
    }

    // 2. 抓取上櫃 ETF 資料
    match tpex::etf::visit().await {
        Ok(items) => update_stocks(items).await?,
        Err(why) => logging::error_file_async(format!("處理上櫃 ETF 市場失敗: {:?}", why)),
    }

    Ok(())
}

/// 批次更新股票資訊到資料庫。
///
/// 此函式接收一個 ETF 資訊列表，並逐一檢查是否需要寫入資料庫。
async fn update_stocks(items: Vec<EtfInfo>) -> Result<()> {
    // 預分配一個 1024 字元的字串，用來累積要發送給 Telegram 的變動訊息
    let mut to_bot_msg = String::with_capacity(1024);

    // 遍歷所有傳入的 ETF 項目
    for item in items {
        // 從系統快取 (SHARE) 中查詢該股票代號，比對現有資料。
        let is_new_or_changed = backfill::is_stock_identity_new_or_changed(
            &item.stock_symbol,
            item.industry_id,
            item.exchange_market.stock_exchange_market_id,
            &item.name,
        )
        .await;

        // 如果確認是新資料或有變動
        if is_new_or_changed {
            // 呼叫下方的 update_stock_info 進行實質的資料庫寫入與通知動作
            if let Err(why) = update_stock_info(&item, &mut to_bot_msg).await {
                // 若更新單一項目失敗，記錄錯誤日誌，但不中斷整個批次流程
                logging::error_file_async(format!(
                    "更新 ETF {} 資訊失敗: {:?}",
                    item.stock_symbol, why
                ));
            }
        }
    }

    // 如果累積的變動訊息不為空，代表有新增或更新，則發送一次 Telegram 通知
    if !to_bot_msg.is_empty() {
        bot::telegram::send(&to_bot_msg).await;
    }

    Ok(())
}

/// 更新單一 ETF 的實體資訊至各個儲存層。
async fn update_stock_info(etf: &EtfInfo, msg: &mut String) -> Result<()> {
    // 1. 準備資料庫對象：建立新的 Stock 資料列實例並填入採集到的欄位
    let mut stock = table::stock::Stock::new();
    stock.stock_symbol = etf.stock_symbol.clone();
    stock.name = etf.name.clone();
    stock.stock_exchange_market_id = etf.exchange_market.stock_exchange_market_id;
    stock.stock_industry_id = etf.industry_id;

    // 2. 寫入資料庫：執行 Upsert (Update or Insert)
    // 如果代號已存在則更新，不存在則新增一筆
    stock
        .upsert()
        .await
        .map_err(|why| anyhow!("資料庫 upsert 失敗: {:?}", why))?;

    // 3. 更新記憶體快取：確保系統的其他功能（如行情查詢）能立刻讀到最新欄位
    if let Ok(mut stocks) = SHARE.stocks.write() {
        stocks.insert(stock.stock_symbol.to_string(), stock.clone());
    }

    // 4. 取得易讀的名稱（用於日誌與通知）
    let market = StockExchangeMarket::from(stock.stock_exchange_market_id);
    let market_name = market
        .map(|m| m.name())
        .unwrap_or_else(|| "未知".to_string());
    let industry_name = SHARE
        .get_industry_name(stock.stock_industry_id)
        .unwrap_or_else(|| "未知".to_string());

    // 組合要顯示在通知與日誌上的訊息文字
    let log_msg = format!(
        "新增/更新 ETF︰ {} {} {} {}",
        stock.stock_symbol,
        Telegram::escape_markdown_v2(&stock.name), // 處理 Telegram 特殊符號轉義
        market_name,
        industry_name
    );

    // 將訊息附加到傳入的訊息緩衝區
    writeln!(msg, "{}\r\n", log_msg).ok();
    // 同步記錄到系統檔案日誌
    logging::info_file_async(&log_msg);

    // 5. 跨服務通知：透過 gRPC 將最新的股票基本資料推送到其他微服務 (Go Service)
    if let Err(why) =
        rpc::client::stock_service::push_stock_info_to_go_service(stock.to_stock_info_request())
            .await
    {
        logging::error_file_async(format!(
            "推送 ETF 資訊至 Go Service 失敗 ({}): {:?}",
            stock.stock_symbol, why
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging;

    #[tokio::test]
    #[ignore]
    async fn test_execute_etf() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute_etf".to_string());

        match execute().await {
            Ok(_) => {
                logging::debug_file_async("完成 execute_etf".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!("執行失敗: {:?}", why));
            }
        }
    }
}
