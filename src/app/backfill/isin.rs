//! # 國際證券識別碼 (ISIN) 回補模組
//!
//! 此模組負責從台灣證券交易所 (TWSE) 爬取最新的國際證券識別碼 (ISIN) 資訊，
//! 以便自動新增或更新資料庫中的股票基本資料（例如交易所市場編號、產業分類、股票名稱等）。
//! 每天執行時會自動跳過週末，並在偵測到股票資料異動或新增時，同步更新記憶體快取、
//! 發送 Telegram 通知，以及將更新同步給 Go 語言的微服務。

use std::fmt::Write;

use crate::{
    app::backfill, core::declare::StockExchangeMarket, core::logging,
    core::util::datetime::Weekend, infra::cache::SHARE, infra::crawler::twse, interfaces::bot,
    interfaces::bot::telegram::Telegram, interfaces::rpc,
};

use anyhow::{anyhow, Result};
use chrono::Local;
use scopeguard::defer;

/// 執行台股國際證券識別碼 (ISIN) 的回補與更新流程。
///
/// # 運作流程
/// 1. 檢查當前時間是否為週末，如果是則直接返回（不執行更新）。
/// 2. 分別針對不同的交易所市場（上市、上櫃等）非同步呼叫 [`process_market`] 進行處理。
/// 3. 等待所有市場處理完畢，並記錄任何發生的錯誤。
///
/// # 回傳值
/// - `Ok(())`：執行成功。
/// - `Err(anyhow::Error)`：在執行或記錄錯誤時發生異常。
pub async fn execute() -> Result<()> {
    // 週末台股不開市且資料不會更新，因此直接跳過不執行以節省資源
    if Local::now().is_weekend() {
        return Ok(());
    }
    logging::info_file_async("更新台股國際證券識別碼開始");
    // 利用 scopeguard 的 defer 機制，不論流程正常結束或提早出錯返回，都會在離開函式時寫入結束日誌
    defer! {
       logging::info_file_async("更新台股國際證券識別碼結束");
    }
    // 遍歷所有定義的交易所市場（如上市、上櫃），併發執行 process_market
    let tasks: Vec<_> = StockExchangeMarket::iterator()
        .map(process_market)
        .collect();

    // 等待所有市場的非同步任務全部執行完畢
    let results = futures::future::join_all(tasks).await;
    for result in results {
        // 若其中某個交易所市場的任務失敗，記錄錯誤訊息，但不影響其他市場的處理結果
        if let Err(why) = result {
            logging::error_file_async(format!("Failed to process_market because {:?}", why));
        }
    }

    Ok(())
}

/// 針對特定的交易所市場爬取 ISIN 代碼資訊，並檢查是否有新增或修改的股票。
///
/// # 參數
/// - `mode`: [`StockExchangeMarket`] 交易所市場類型，決定要爬取哪個市場的證券識別碼。
///
/// # 運作流程
/// 1. 呼叫 `twse::international_securities_identification_number::visit` 取得該市場的最新證券識別碼資料。
/// 2. 遍歷爬取到的每檔證券，利用 [`backfill::is_stock_identity_new_or_changed`] 比對資料庫，
///    判斷是否為新股票或關鍵基本資料（產業、市場、名稱）有變動。
/// 3. 若資料為全新或有變動，呼叫 [`update_stock_info`] 進行資料庫 Upsert 及快取同步。
/// 4. 彙整異動訊息，若有新增或更新的股票，最後透過 Telegram 機器人發送通知。
///
/// # 回傳值
/// - `Ok(())`：該市場處理完成。
/// - `Err(anyhow::Error)`：爬取或處理過程中發生錯誤。
async fn process_market(mode: StockExchangeMarket) -> Result<()> {
    // 透過 twse crawler 模組爬取指定市場的 ISIN 證券識別碼網頁並解析
    let result = twse::international_securities_identification_number::visit(mode).await?;
    // 初始化 Telegram 訊息緩衝區，預分配 1024 位元組以減少動態記憶體配置的開銷
    let mut to_bot_msg = String::with_capacity(1024);
    for item in result {
        // 透過防腐層 (ACL) Mapper 將外部爬蟲 DTO 轉換為內部指令
        let cmd = match backfill::acl::IsinAclMapper::to_registration_command(&item) {
            Some(c) => c,
            None => continue, // 過濾無效或未分類的資料 (如 industry_id == 0)
        };

        // 比對資料庫快取，檢查這檔股票是否是新上市，或是其產業、市場、名稱欄位有發生變更
        let new_stock = backfill::is_stock_identity_new_or_changed(
            &cmd.symbol,
            cmd.industry_id,
            cmd.market_id,
            &cmd.name,
        )
        .await;

        // 若確認是新股票或資料有變動，則進行寫入與同步處理
        if new_stock {
            if let Err(why) = update_stock_info(&cmd, &mut to_bot_msg).await {
                // 若更新單一股票基本資料失敗，記錄錯誤後繼續處理下一檔，避免單一錯誤導致整個市場回補中斷
                logging::error_file_async(format!(
                    "Failed to update stock info for {} because {:?}",
                    cmd.symbol, why
                ));
            }
        }
    }

    // 若本次處理有產生任何新增或變更的股票，則一次性發送 Telegram 通知給管理員
    if !to_bot_msg.is_empty() {
        bot::telegram::send(&to_bot_msg).await;
    }

    Ok(())
}

/// 更新單一證券的詳細基本資訊至資料庫、記憶體快取，並同步通知 Go 服務與 Telegram 訊息緩衝區。
///
/// # 參數
/// - `cmd`: 註冊或變更證券基本識別資料的指令。
/// - `msg`: 用於累積 Telegram 發送通知的訊息字串緩衝區。
///
/// # 運作流程
/// 1. 獲取已存在的證券主檔或註冊一個新的，並使用業務方法更新識別資訊
/// 2. 寫入資料庫：利用 Repository 保存，會自動觸發搜尋索引重建與快取同步
/// 3. 解析易讀的市場名稱與產業名稱，供日誌與 Telegram 通知使用
/// 4. 格式化股票異動資訊，對股票名稱進行 Markdown 轉義以防 Telegram 訊息格式錯誤
/// 5. 寫入 Telegram 訊息快取緩衝區，並寫入非同步檔案日誌
/// 6. 同步通知 Go 撰寫的另一個微服務，將最新股票資訊同步推送過去
///
/// # 回傳值
/// - `Ok(())`：更新成功。
/// - `Err(anyhow::Error)`：資料庫寫入失敗或轉換過程發生錯誤（gRPC 同步失敗僅會記錄日誌，不會阻斷流程）。
async fn update_stock_info(
    cmd: &backfill::acl::RegisterStockCommand,
    msg: &mut String,
) -> Result<()> {
    use crate::domain::registry::entity::Stock;
    use crate::domain::registry::repository::StockRepository;
    use crate::infra::database::repository::stock::PgStockRepository;
    use crate::interfaces::rpc::stock::StockInfoRequest;

    let repo = PgStockRepository::new();

    // 1. 獲取已存在的證券主檔或註冊一個新的，並使用業務方法更新識別資訊
    let stock = match repo.find_by_symbol(&cmd.symbol).await? {
        Some(mut existing) => {
            existing.change_identity(
                cmd.name.clone(),
                cmd.market_id,
                cmd.industry_id,
            );
            existing
        }
        None => Stock::register(
            cmd.symbol.clone(),
            cmd.name.clone(),
            cmd.market_id,
            cmd.industry_id,
        ),
    };

    // 2. 寫入資料庫：利用 Repository 保存，會自動觸發搜尋索引重建與快取同步
    repo.save(&stock)
        .await
        .map_err(|why| anyhow!("Failed to stock.upsert() because {:?}", why))?;

    // 3. 解析易讀的市場名稱與產業名稱，供日誌與 Telegram 通知使用
    let market = StockExchangeMarket::from(stock.market_id());
    let market_name = match market {
        None => " - ",
        Some(sem) => &sem.name(),
    };
    let industry_name = SHARE
        .get_industry_name(stock.industry_id())
        .unwrap_or(" - ".to_string());

    // 4. 格式化股票異動資訊，對股票名稱進行 Markdown 轉義以防 Telegram 訊息格式錯誤
    let log_msg = format!(
        "新增股票︰ {stock_symbol} {stock_name} {market_name} {industry_name}",
        stock_symbol = stock.symbol().0,
        stock_name = Telegram::escape_markdown_v2(stock.name()),
        market_name = market_name,
        industry_name = industry_name
    );

    // 5. 寫入 Telegram 訊息快取緩衝區，並寫入非同步檔案日誌
    writeln!(msg, "{}\r\n", log_msg).ok(); // 即使寫入 msg 緩衝區失敗也不影響主流程
    logging::info_file_async(log_msg);

    // 6. 同步通知 Go 撰寫的另一個微服務，將最新股票資訊同步推送過去
    let request = StockInfoRequest::from(&stock);
    if let Err(why) = rpc::client::stock_service::push_stock_info_to_go_service(request).await {
        // gRPC 推送失敗時僅記錄錯誤日誌，不拋出錯誤，避免因為外部服務異常導致本機的 backfill 流程中斷
        logging::error_file_async(format!(
            "Failed to push_stock_info_to_go_service for {} because {:?}",
            stock.symbol().0,
            why
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {
                logging::debug_file_async("完成 execute".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
