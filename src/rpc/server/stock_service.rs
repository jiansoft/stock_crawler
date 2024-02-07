use futures::future::join_all;
use tonic::{Request, Response, Status};

use crate::{
    crawler,
    logging,
    rpc::{
        stock::{
            StockQuotesRequest,
            stock_server::Stock,
            StockInfoReply,
            StockInfoRequest,
            StockQuotes,
            StockQuotesReply,
            HolidayScheduleReply,
            HolidayScheduleRequest
        }
    },
    crawler::twse,
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
    async fn fetch_holiday_schedule(&self, req: Request<HolidayScheduleRequest>) -> Result<Response<HolidayScheduleReply>, Status> {
        let request = req.into_inner();
        let formatted_dates = match twse::holiday_schedule::visit(request.year).await {
            Ok(holiday) => holiday.iter()
                .map(|date| date.format("%Y-%m-%d").to_string())
                .collect(),
            Err(why) => {
                logging::error_file_async(format!("Failed to visit twse::holiday_schedule because {:?}", why));
                vec![]
            }
        };

        Ok(Response::new(HolidayScheduleReply {
            holiday: formatted_dates,
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
    use crate::rpc::{stock, stock::stock_server::StockServer};

    use super::*;

    #[tokio::test]
    async fn test_fetch_current_stock_price() {
        // Create the mock server
        let mock_service = StockService::default();
        let mock_server = tonic::transport::Server::builder()
            .add_service(StockServer::new(mock_service))
            .serve("127.0.0.1:50051".parse().unwrap());
        //.await .expect("Server failed");

        tokio::spawn(mock_server);

        // Wait a bit for server to be up. In real-world cases, you'd use a more robust mechanism.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Use the service like you would against a real server
        let mut client = stock::stock_client::StockClient::connect("http://127.0.0.1:50051")
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
        //assert_eq!(response.into_inner().message, "Hello Tonic!");
    }

    #[tokio::test]
    async fn test_fetch_holiday_schedule() {
        // Create the mock server
        let mock_service = StockService::default();
        let mock_server = tonic::transport::Server::builder()
            .add_service(StockServer::new(mock_service))
            .serve("127.0.0.1:50051".parse().unwrap());
        //.await .expect("Server failed");

        tokio::spawn(mock_server);

        // Wait a bit for server to be up. In real-world cases, you'd use a more robust mechanism.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Use the service like you would against a real server
        let mut client = stock::stock_client::StockClient::connect("http://127.0.0.1:50051")
            .await
            .expect("Failed to connect");

        let request = Request::new(HolidayScheduleRequest {
           year:2024,
        });

        let resp = client
            .fetch_holiday_schedule(request)
            .await
            .expect("RPC Failed!");
        println!("message:{:#?}", resp.into_inner().holiday)
        //assert_eq!(response.into_inner().message, "Hello Tonic!");
    }
}
