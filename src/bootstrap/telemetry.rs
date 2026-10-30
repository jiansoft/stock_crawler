//! 啟動第 1～3 步：`.env`、tracing subscriber、輪轉日誌檔參數與 Seq。

use stock_crawler::core;

/// 載入 `.env` 並初始化日誌；必須在任何 `tracing::*!` 呼叫之前執行。
pub(crate) async fn init() {
    // ── 1. 環境變數載入 ──────────────────────────────────────────────────────
    // `.env` 檔案讓本機開發不需設定系統環境變數（資料庫密碼、Token 等敏感值）。
    // `.ok()` 讓「找不到 .env」靜默忽略—生產環境透過 Docker env 或 K8s Secret 注入。
    // 此行必須在 SETTINGS 第一次被存取之前執行，否則 config 讀不到 .env 覆蓋值。
    dotenvy::dotenv().ok();

    // ── 2. 日誌系統初始化 ────────────────────────────────────────────────────
    //
    // 使用 tracing + tracing-subscriber 建立兩層 subscriber：
    //
    // Layer 1 — fmt（標準輸出）
    //   由 RUST_LOG 環境變數控制要不要輸出，以及輸出哪個等級。
    //   預設不設定 RUST_LOG 時完全靜音，不影響生產環境。
    //   開發除錯時執行：RUST_LOG=info cargo run
    //
    // Layer 2 — FileLogLayer（輪轉日誌檔）
    //   實作在 core::logging::FileLogLayer，不受 RUST_LOG 控制，改由 FILE_LOG_LEVEL 控制等級。
    //   預設只落 INFO 以上，寫入 log/YYYY-MM-DD_default_{level}.log；
    //   臨時除錯可設 FILE_LOG_LEVEL=debug 開回 DEBUG。
    //   底層仍使用原有的非同步輪轉機制（core::logging::LOGGER）。
    //
    // 事件流向：
    //   tracing::info!("...")
    //     ├─ FileLogLayer → LOGGER → 輪轉日誌檔（+ Seq，若有設定）
    //     └─ fmt layer    → stdout（僅 RUST_LOG 啟用時）
    //
    // 注意：subscriber 必須在任何 tracing::*! 呼叫之前完成初始化，
    //       否則初始化前的事件會被靜默丟棄。
    {
        use tracing_subscriber::Layer;
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_filter(tracing_subscriber::EnvFilter::from_default_env()),
            )
            // 檔案日誌預設只落 INFO 以上（由 FILE_LOG_LEVEL 覆寫），
            // 避免 prod 將大量 DEBUG/TRACE 寫入磁碟導致日誌檔暴增。
            .with(core::logging::FileLogLayer.with_filter(core::logging::file_log_env_filter()))
            .init();
        // 慢查詢當日統計的起點定在啟動時間（23:30 主機健康彙總會註明「自 HH:MM 起」）。
        core::logging::slow_query::init();
    }

    // ── 2.5 輪轉日誌檔參數 ──────────────────────────────────────────────────
    // 單檔大小上限與保留天數來自 app.json 的 `logging.file`，可由 .env 的
    // LOG_FILE_MAX_SIZE_MB / LOG_FILE_MAX_AGE_DAYS 覆蓋；皆未設定時為 10 MB / 7 天。
    // 必須在此套用（而非 logger 建立時讀 SETTINGS），避免 logger 反向依賴 config
    // 造成 Lazy 初始化互鎖。
    core::logging::init_file_rotation(
        core::config::SETTINGS.logging.file.max_size_mb,
        core::config::SETTINGS.logging.file.max_age_days,
    );

    // ── 3. Seq 結構化日誌收集器（選用）──────────────────────────────────────
    // Seq 是一套可視化的結構化日誌平台（類似 ELK，但部署更輕量）。
    // 若 app.json 中 `logging.seq.server_url` 為空字串，此函式直接返回，不做任何事。
    // 設定後，tracing 事件會同時發送至 Seq HTTP ingest API，
    // 讓 ops 人員在 Seq 介面上做結構化查詢、設定告警、追蹤 correlation ID。
    core::logging::init_seq(
        &core::config::SETTINGS.logging.seq.server_url,
        &core::config::SETTINGS.logging.seq.api_key,
    )
    .await;
}
