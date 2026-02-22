use futures::future::join_all;
use tonic::{Request, Response, Status};

use crate::{
    crawler,
    crawler::twse,
    logging,
    rpc::stock::{
        stock_server::Stock, HolidaySchedule, HolidayScheduleReply, HolidayScheduleRequest,
        StockInfoReply, StockInfoRequest, StockQuotes, StockQuotesReply, StockQuotesRequest,
    },
};

#[derive(Default)]
pub struct StockService {}

#[tonic::async_trait]
impl Stock for StockService {
    async fn update_stock_info(
        &self,
        _req: Request<StockInfoRequest>,
    ) -> Result<Response<StockInfoReply>, Status> {
        Err(Status::unimplemented(
            "update_stock_info is not implemented",
        ))
    }

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

    //
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

async fn fetch_current_quotes_for_symbol(stock_symbol: &str) -> Option<StockQuotes> {
    if let Ok(sq) = crawler::fetch_stock_quotes_from_remote_site(stock_symbol).await {
        return Some(StockQuotes {
            stock_symbol: stock_symbol.to_string(),
            price: sq.price,
            change: sq.change,
            change_range: sq.change_range,
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

    /// 啟動 gRPC 伺服器並回傳其位址
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
