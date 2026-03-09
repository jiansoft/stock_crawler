//! gRPC 模組入口。

/// gRPC client 封裝。
pub mod client;
/// gRPC server 封裝。
pub mod server;

/// Stock 服務產生碼。
pub mod stock {
    #![allow(missing_docs)]
    include!("stock.rs");
}

/// 基本回應型別產生碼。
pub mod basic {
    #![allow(missing_docs)]
    include!("basic.rs");
}

/// Control 服務產生碼。
pub mod control {
    #![allow(missing_docs)]
    include!("control.rs");
}
