//! gRPC 模組入口。
//!
//! 本模組負責管理所有與 gRPC 相關的功能，包括服務端實作、客戶端封裝
//! 以及透過 `tonic` 與 `prost` 產生的 Protocol Buffers 定義程式碼。

/// gRPC client 封裝。
///
/// 包含所有對外部服務調用的客戶端實作，例如將資料推送至其他服務。
pub mod client;

/// gRPC server 封裝。
///
/// 包含此專案對外提供的 gRPC 服務實作，例如提供即時股價查詢。
pub mod server;

/// Stock 服務產生碼。
///
/// 包含股票資訊管理、報價查詢及休市日查詢等 RPC 定義。
pub mod stock {
    #![allow(missing_docs)]
    include!("stock.rs");
}

/// 基本回應型別產生碼。
///
/// 包含通用的 gRPC 回應結構與基本資料型別定義。
pub mod basic {
    #![allow(missing_docs)]
    include!("basic.rs");
}

/// Control 服務產生碼。
///
/// 包含系統控制相關的 RPC 定義，例如健康檢查或系統測試。
pub mod control {
    #![allow(missing_docs)]
    include!("control.rs");
}
