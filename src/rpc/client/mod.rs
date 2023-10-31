use std::sync::Arc;

use anyhow::Result;
use once_cell::sync::Lazy;
use tokio::{fs, sync::OnceCell as TokioOnceCell};
use tonic::transport::{Certificate, Channel, ClientTlsConfig};

use crate::{
    internal::{config::SETTINGS},
    rpc::stock::stock_client::StockClient
};

pub mod stock_service;

static GRPC: Lazy<Arc<TokioOnceCell<Grpc>>> = Lazy::new(|| Arc::new(TokioOnceCell::new()));

struct Grpc {
    stock: StockClient<Channel>,
}

impl Grpc {
    pub async fn new() -> Result<Self> {
        let pem = fs::read_to_string(&SETTINGS.rpc.go_service.tls_cert_file).await?;
        let ca = Certificate::from_pem(pem);
        let tls = ClientTlsConfig::new()
            .ca_certificate(ca)
            .domain_name(&SETTINGS.rpc.go_service.domain_name);
        let channel = Channel::from_static(&SETTINGS.rpc.go_service.target)
            .tls_config(tls)?
            .connect()
            .await?;
        let client = StockClient::new(channel);

        Ok(Grpc { stock: client })
    }
}

async fn get_client() -> Result<&'static Grpc> {
    GRPC.get_or_try_init(|| async { Grpc::new().await }).await
}
