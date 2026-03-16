//! Stock gRPC 服務實作。
//!
//! 提供股票相關的查詢服務，包括即時報價查詢與休市日清單查詢。

use futures::future::join_all;
use rust_decimal::prelude::ToPrimitive;
use tonic::{Request, Response, Status};

use crate::{
    cache::SHARE,
    crawler::twse,
    logging,
    rpc::stock::{
        stock_server::Stock, HolidaySchedule, HolidayScheduleReply, HolidayScheduleRequest,
        StockInfoReply, StockInfoRequest, StockQuotes, StockQuotesReply, StockQuotesRequest,
    },
};

/// Stock gRPC 服務。
///
/// 實作了 `Stock` trait，提供股票資訊、報價及休市日相關的 RPC 介面。
#[derive(Default)]
pub struct StockService {}

#[tonic::async_trait]
impl Stock for StockService {
    /// 更新股票資訊。
    ///
    /// 此方法目前尚未實作。
    async fn update_stock_info(
        &self,
        _req: Request<StockInfoRequest>,
    ) -> Result<Response<StockInfoReply>, Status> {
        Err(Status::unimplemented(
            "update_stock_info is not implemented",
        ))
    }

    /// 批次取得股票即時報價。
    ///
    /// 根據請求中的股票代碼列表，並行調用爬蟲取得最新報價。
    ///
    /// # Arguments
    ///
    /// * `req` - 包含 `StockQuotesRequest` (股票代碼列表) 的 gRPC 請求。
    ///
    /// # Returns
    ///
    /// 回傳 `StockQuotesReply`，包含所有成功取得的股票報價。
    async fn fetch_current_stock_quotes(
        &self,
        req: Request<StockQuotesRequest>,
    ) -> Result<Response<StockQuotesReply>, Status> {
        let request = req.into_inner();
        let futures: Vec<_> = request
            .stock_symbols
            .iter()
            .map(|stock_symbol| fetch_current_quotes_for_symbol(stock_symbol))
            .collect();
        let stock_prices = join_all(futures).await;
        let filtered: Vec<StockQuotes> = stock_prices.into_iter().flatten().collect();

        Ok(Response::new(StockQuotesReply {
            stock_prices: filtered,
        }))
    }

    /// 取得指定年度的休市日清單。
    ///
    /// 呼叫 TWSE 爬蟲取得該年度的所有休市日期與原因。
    ///
    /// # Arguments
    ///
    /// * `req` - 包含 `HolidayScheduleRequest` (年份) 的 gRPC 請求。
    ///
    /// # Returns
    ///
    /// 回傳 `HolidayScheduleReply`，包含休市日列表。
    async fn fetch_holiday_schedule(
        &self,
        req: Request<HolidayScheduleRequest>,
    ) -> Result<Response<HolidayScheduleReply>, Status> {
        let request = req.into_inner();
        let holiday_schedules = match twse::holiday_schedule::visit(request.year).await {
            Ok(holidays) => holidays
                .iter()
                .map(|holiday| HolidaySchedule {
                    date: holiday.date.format("%Y-%m-%d").to_string(),
                    why: holiday.why.to_string(),
                })
                .collect(),
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to visit twse::holiday_schedule because {:?}",
                    why
                ));
                vec![]
            }
        };

        Ok(Response::new(HolidayScheduleReply {
            holiday: holiday_schedules,
        }))
    }
}

/// 取得單一股票的即時報價，並轉成 gRPC 回傳型別。
///
/// 內部輔助函數，從 `SHARE` 快取中取得資料。優先使用即時報價快照 (`stock_snapshots`)，
/// 若無快照則嘗試取得最後交易日報價 (`last_trading_day_quotes`) 作為備援。
async fn fetch_current_quotes_for_symbol(stock_symbol: &str) -> Option<StockQuotes> {
    // 1. 優先從即時報價快照取得
    if let Some(snapshot) = SHARE.get_stock_snapshot(stock_symbol) {
        return Some(StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: snapshot.price.to_f64().unwrap_or_default(),
            change: snapshot.change.to_f64().unwrap_or_default(),
            change_range: snapshot.change_range.to_f64().unwrap_or_default(),
        });
    }

    // 2. 若無即時報價，則從最後交易日報價取得 (作為備援)
    if let Some(last_quote) = SHARE.get_stock_last_price(stock_symbol).await {
        return Some(StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: last_quote.closing_price.to_f64().unwrap_or_default(),
            change: 0.0,
            change_range: 0.0,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;

    use crate::rpc::{stock, stock::stock_server::StockServer};

    use super::*;

    /// 啟動測試用 gRPC 伺服器並回傳其位址。
    async fn start_test_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let addr_str = format!("http://{}", addr);

        let mock_service = StockService::default();
        let server = tonic::transport::Server::builder()
            .add_service(StockServer::new(mock_service))
            .serve_with_incoming(TcpListenerStream::new(listener));

        tokio::spawn(server);

        // 等待伺服器啟動
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        addr_str
    }

    /// 驗證查詢即時報價 RPC。
    #[tokio::test]
    async fn test_fetch_current_stock_price() {
        let addr = start_test_server().await;

        let mut client = stock::stock_client::StockClient::connect(addr)
            .await
            .expect("Failed to connect");

        let request = Request::new(StockQuotesRequest {
            stock_symbols: vec!["2330".to_string(), "2888".to_string(), "3008".to_string()],
        });

        let resp = client
            .fetch_current_stock_quotes(request)
            .await
            .expect("RPC Failed!");
        println!("message:{:#?}", resp.into_inner().stock_prices)
    }

    /// 驗證查詢休市日清單 RPC。
    #[tokio::test]
    async fn test_fetch_holiday_schedule() {
        let addr = start_test_server().await;

        let mut client = stock::stock_client::StockClient::connect(addr)
            .await
            .expect("Failed to connect");

        let request = Request::new(HolidayScheduleRequest { year: 2024 });

        let resp = client
            .fetch_holiday_schedule(request)
            .await
            .expect("RPC Failed!");
        println!("message:{:#?}", resp.into_inner().holiday)
    }
}
