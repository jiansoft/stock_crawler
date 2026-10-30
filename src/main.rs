//! `stock_crawler` 可執行程式入口。
//!
//! 主要職責：
//! - 載入環境與主快取。
//! - 啟動排程與 gRPC 服務。
//! - 註冊結束訊號，讓行程可平順停止。
//! - 啟動後做一次 gRPC 自我連線驗證。

use std::error::Error;

// ── 引用函式庫 crate 的模組樹 ──────────────────────────────────────────────────
// 五層 DDD 架構，依賴方向由內而外（內層不知道外層存在）：
//   domain ← core ← infra ← app ← interfaces
//
// app        — 應用服務層：排程器 (scheduler)、Use Case 協調、跨領域流程串接
// core       — 基礎設施核心：config、logging、util 等不含業務邏輯的通用工具
// domain     — 領域模型：實體、值物件、領域服務，零外部框架依賴（純 Rust struct/trait）
// infra      — 基礎設施實作：database（SQLx/PostgreSQL）、nosql（Redis）、cache
// interfaces — 外部介面：gRPC server/client、Axum Web server、Telegram bot
//
// 這些模組實際定義在 `src/lib.rs`（`stock_crawler` 函式庫 target）。
// main.rs 與 bootstrap 只透過 `use` 引入，不再用 `pub mod` 重新宣告整棵模組樹，
// 避免 `cargo test` 把同一組程式碼（含所有單元測試）分別編譯進 lib 與 bin
// 兩個測試執行檔、對同一組 CI 服務容器重跑兩遍。
mod bootstrap;

/// 啟動 `stock_crawler` 主流程：初始化日誌、注入 adapter、完成啟動，再等待關機訊號平順停止。
///
/// `#[tokio::main]` 宏在底層建立多執行緒 Tokio runtime（`features = ["full"]` 啟用全部功能），
/// 並將 `async fn main()` 包在 `runtime.block_on()` 裡執行。
/// 回傳 `Result<(), Box<dyn Error>>` 讓任何啟動錯誤都能顯示給使用者，
/// 並以非零 exit code 退出（讓 systemd / Docker restart policy 能偵測到異常並重啟）。
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    bootstrap::telemetry::init().await;
    bootstrap::wiring::register_adapters();
    let running = bootstrap::startup::start().await?;
    bootstrap::shutdown::wait_and_stop(running).await
}
