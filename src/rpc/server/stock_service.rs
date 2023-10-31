use futures::future::join_all;
use rust_decimal::prelude::ToPrimitive;
use tonic::{Request, Response, Status};

use crate::{
    internal::crawler,
    rpc::stock::{
        stock_server::Stock, StockInfoReply, StockInfoRequest, StockPrice, StockPriceReply,
        StockPriceRequest,
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

    async fn fetch_current_stock_price(
        &self,
        req: Request<StockPriceRequest>,
    ) -> Result<Response<StockPriceReply>, Status> {
        let request = req.into_inner();
        let futures: Vec<_> = request
            .stock_symbols
            .iter()
            .map(|stock_symbol| fetch_current_price_for_symbol(stock_symbol))
            .collect();
        let stock_prices = join_all(futures).await;

        Ok(Response::new(StockPriceReply { stock_prices }))
    }
}

async fn fetch_current_price_for_symbol(stock_symbol: &str) -> StockPrice {
    if let Ok(remote_price) = crawler::fetch_stock_price_from_remote_site(stock_symbol).await {
        return StockPrice {
            stock_symbol: stock_symbol.to_string(),
            price: remote_price.to_f64().unwrap_or_default(),
        };
    }

    StockPrice {
        stock_symbol: stock_symbol.to_string(),
        price: 0.0,
    }
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

        let request = Request::new(StockPriceRequest {
            stock_symbols: vec!["2330".to_string(), "2888".to_string(), "3008".to_string()],
        });

        let resp = client
            .fetch_current_stock_price(request)
            .await
            .expect("RPC Failed!");
        println!("message:{:#?}", resp.into_inner().stock_prices)
        //assert_eq!(response.into_inner().message, "Hello Tonic!");
    }
}