//! 啟動第 3.5 步：把 interfaces 的具體實作（adapter）注入 core／app 的抽象介面（port）。

use stock_crawler::{app, core, interfaces};

/// 註冊告警、股票資料推送與策略換股的 adapter；必須在任何可能使用這些介面的流程之前呼叫。
pub(crate) fn register_adapters() {
    // ── 3.5 註冊 port/adapter 接線 ──────────────────────────────────────────
    // 依 DDD 分層，core/infra/app 不直接依賴 interfaces，改呼叫抽象介面（port）；
    // 這裡把具體實作（adapter）注入，必須在任何可能使用這些介面的流程之前完成。
    //
    // 1. 告警管道：core/infra 呼叫 core::alert::send_alert / send_message，
    //    訊息由 Telegram adapter 實際送出。
    core::alert::register_alert_sink(std::sync::Arc::new(
        interfaces::bot::telegram::TelegramAlertSink,
    ));
    // 2. 股票資料推送：app handler 呼叫 app::ports::push_stock_info，
    //    由 gRPC adapter 轉成 StockInfoRequest 後推送到 Go 服務。
    app::ports::register_stock_info_gateway(std::sync::Arc::new(
        interfaces::rpc::client::stock_service::GrpcStockInfoGateway,
    ));
    // 3. 策略換股通知：app use case 呼叫 app::ports::strategy_gateway，
    //    由 data API 的條件選股回測引擎實際計算。
    app::ports::register_strategy_gateway(std::sync::Arc::new(
        interfaces::web::data_api::BacktestStrategyGateway,
    ));
}
