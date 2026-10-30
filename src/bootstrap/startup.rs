//! 啟動第 4～14 步：分配器調校、資料庫與快取檢查、排程、gRPC／Web 伺服器、自我測試與 Redis。

// Instant 是單調遞增時鐘，精確到奈秒，用於效能量測。
// 與 SystemTime 的差別：不受使用者調整系統時間影響，適合計算程式執行時間。
use std::{error::Error, time::Instant};

use tokio::task::JoinHandle;
// tokio-cron-scheduler 是基於 Tokio 的非同步 cron 排程器，支援標準 cron 語法。
// 每個 job 在到期時以非同步 task 形式在 Tokio 執行緒池中執行，不阻塞主執行緒。
use tokio_cron_scheduler::JobScheduler;

use stock_crawler::{app, core, infra, interfaces};

/// 啟動完成後交給 [`super::shutdown::wait_and_stop`] 的資源。
pub(crate) struct Running {
    /// 收到關機訊號後通知 HTTP／gRPC 停止接受新連線。
    pub(crate) shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// 已啟動的排程器。
    pub(crate) sched: JobScheduler,
    /// gRPC 伺服器 task；`grpc_use_port` 為 0 時沒有。
    pub(crate) rpc_server: Option<JoinHandle<anyhow::Result<()>>>,
    /// Web 伺服器 task。
    pub(crate) web_server: JoinHandle<anyhow::Result<()>>,
    /// 啟動後的 gRPC 自我連線測試。
    pub(crate) grpc_self_test: JoinHandle<()>,
}

/// 依序完成啟動並回傳執行中的資源；資料庫、核心快取、排程或伺服器啟動失敗時回傳錯誤。
///
/// # Errors
///
/// 資料庫連不上、必要快取載入失敗、排程器建立或註冊失敗、gRPC／Web 伺服器啟動失敗時回傳錯誤
/// （皆已先發 Telegram 告警），讓 systemd／Docker 依非零 exit code 重啟。
pub(crate) async fn start() -> Result<Running, Box<dyn Error>> {
    // ── 4. 啟動計時器與 server 關機廣播 ─────────────────────────────────────
    // `startup_timer` 讓後面的各啟動階段都能記錄 elapsed 時間，方便找效能瓶頸。
    let startup_timer = Instant::now();
    // watch channel 會在收到 OS 訊號後通知 HTTP/gRPC 停止接受新連線。
    // receiver 可廉價 clone，兩個 server 都能觀察同一個單向關機狀態。
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // ── 5. 分配器調校（長執行程序模式）─────────────────────────────────────
    // 對 glibc malloc（透過 `mallopt` syscall）設定較積極的 arena 數量與 trim 閾值，
    // 避免長時間運行後因 arena fragmentation 導致 RSS 持續膨脹（記憶體佔用居高不下）。
    // 在非 glibc 環境（musl / macOS / Windows）此函式為 no-op，applied 欄位為 false。
    let allocator_tuning = core::util::diagnostics::tune_allocator_for_long_running_process();

    // 記錄分配器型別與調校結果到日誌，方便事後對照 Grafana RSS 趨勢圖。
    #[cfg(all(target_os = "linux", target_env = "musl"))]
    tracing::info!("{}", "allocator profile: mimalloc (musl target), purge_delay=0, purge_decommits=1, thp=0, large_os_pages=0, arena_eager_commit=0"
            .to_string(),);

    #[cfg(not(all(target_os = "linux", target_env = "musl")))]
    tracing::info!("allocator profile: system allocator");

    if allocator_tuning.applied {
        tracing::info!(
            "allocator tuning applied arena_max={} trim_threshold={}",
            allocator_tuning.arena_max_applied,
            allocator_tuning.trim_threshold_applied,
        );
    }

    // ── 6. 資料庫連線檢查（Fail Fast）───────────────────────────────────────
    // 在載入快取與啟動背景服務之前，先確認 PostgreSQL 可用。
    // 若資料庫不通，後續所有依賴 DB 的初始化都會失敗；
    // 提早報錯讓 Docker / systemd 立刻知道需要重啟或告警，比掛在後面更好追查。
    // 同時透過 Telegram 告警確保即使 ops 不在電腦前也能即時收到通知。
    tracing::info!("startup database check: ping database");
    if let Err(e) = infra::database::ping().await {
        let err_msg = format!("Failed to connect to database: {:?}", e);
        tracing::error!("{}", &err_msg);
        interfaces::bot::telegram::send_alert("資料庫連線失敗（主機啟動異常）", &err_msg).await;
        return Err(err_msg.into());
    }
    tracing::info!("startup database check: database is online");

    // ── 8. 全域快取預熱（SHARE.load_required，Fail Fast）───────────────────
    // `infra::cache::SHARE` 是全域單例，儲存所有股票的即時快照（RealtimeSnapshot）
    // 及基本資訊（名稱、市場別、產業類別等）。
    // `.load_required()` 從 PostgreSQL 載入全部股票清單並建立 HashMap，
    // 耗時視股票數量而定（台股約 1,800～2,000 支，通常 1～3 秒內完成）。
    // 後續爬蟲與排程任務都透過此快取直接查詢，避免頻繁打 DB；
    // 即時股價更新（set_stock_snapshot_price）也在記憶體內完成，不需回寫 DB。
    //
    // 必要快取（股票主檔、最後交易日報價）載入失敗或股票主檔為空時，
    // 這裡會直接中止啟動並發 Telegram 告警——空的核心快取代表 DB 內容或
    // 查詢已壞，讓服務「看似正常啟動」只會悄悄漏抓、漏算。
    // 非必要快取（指數、營收、歷史高低、公網 IP）失敗只降級，會列在 report 中。
    tracing::info!(
        "{}",
        "startup phase begin: crate::infra::cache::SHARE.load_required".to_string(),
    );
    let cache_load_timer = Instant::now();
    match infra::cache::SHARE.load_required().await {
        Ok(report) => {
            if !report.degraded.is_empty() {
                tracing::warn!(
                    "cache load degraded (non-critical caches failed): {:?}",
                    report.degraded
                );
            }
            tracing::info!(
                "startup phase done: SHARE.load_required elapsed={:?}, stocks={}, last_trading_day_quotes={}",
                cache_load_timer.elapsed(),
                report.stock_count,
                report.last_trading_day_quote_count
            );
        }
        Err(why) => {
            let err_msg = format!("Failed to load required caches: {:?}", why);
            tracing::error!("{}", &err_msg);
            interfaces::bot::telegram::send_alert("核心快取載入失敗（主機啟動異常）", &err_msg)
                .await;
            return Err(err_msg.into());
        }
    }

    // ── 9. 排程器建立 ───────────────────────────────────────────────────────
    // `JobScheduler::new()` 建立 tokio-cron-scheduler 實例，但尚未啟動任何 job。
    // 內部會建立 Tokio channel 與背景輪詢 task，因此需要 .await。
    // `?` 操作子：若建立失敗（極罕見，通常是 Tokio runtime 問題）直接向上回傳錯誤。
    tracing::info!("startup phase begin: JobScheduler::new");
    let scheduler_new_timer = Instant::now();
    let sched = JobScheduler::new().await?;
    tracing::info!(
        "startup phase done: JobScheduler::new elapsed={:?}",
        scheduler_new_timer.elapsed()
    );

    // ── 10. 排程任務註冊與啟動 ──────────────────────────────────────────────
    // `app::scheduler::start(&sched)` 把所有 cron job 加進 `sched`，並呼叫
    // `sched.start()` 讓排程器的背景執行緒開始輪詢到期任務。
    // 所有 cron 表達式皆為 UTC 時區（台灣 UTC+8 自行換算，例如 09:00 TWN = 01:00 UTC）。
    // 若其中一個 job 註冊失敗，整體回傳 Err 並中止啟動，避免部分任務靜默遺失。
    tracing::info!("startup phase begin: scheduler::start");
    let scheduler_start_timer = Instant::now();
    app::scheduler::start(&sched).await?;
    tracing::info!(
        "startup phase done: scheduler::start elapsed={:?}",
        scheduler_start_timer.elapsed()
    );

    // ── 11. gRPC 伺服器啟動 ──────────────────────────────────────────────────
    // `interfaces::rpc::server::start()` 在 app.json 設定的埠號（預設 9001）開始監聽。
    // 若 app.json 同時設定了 ssl_cert_file 與 ssl_key_file，
    // 會啟用 TLS 模式（Let's Encrypt 憑證，注意 90 天到期週期）；
    // 否則以 insecure 模式運行（僅限內網或開發環境）。
    // 啟動失敗（如埠號被佔用、憑證路徑錯誤）時發送 Telegram 告警並中止主程式。
    tracing::info!("startup phase begin: rpc::server::start");
    let rpc_start_timer = Instant::now();
    let rpc_server = match interfaces::rpc::server::start(shutdown_rx.clone()).await {
        Ok(handle) => handle,
        Err(why) => {
            let err_msg = format!("gRPC server failed to start: {:?}", why);
            tracing::error!("{}", &err_msg);
            interfaces::bot::telegram::send_alert("gRPC 伺服器啟動失敗", &err_msg).await;
            return Err(why.into());
        }
    };
    tracing::info!(
        "startup phase done: rpc::server::start elapsed={:?}",
        rpc_start_timer.elapsed()
    );

    // ── 12. Web 伺服器啟動（Axum）────────────────────────────────────────────
    // `interfaces::web::start()` 啟動 Axum HTTP 伺服器，提供 REST API 與靜態頁面。
    // 主要對外端點：月營收查詢（/stock/revenues）等，對應 Live Demo 網址。
    // 與 gRPC 一樣，啟動失敗時發送 Telegram 告警並中止主程式，確保不靜默失敗。
    tracing::info!("startup phase begin: web::start");
    let web_start_timer = Instant::now();
    let web_server = match interfaces::web::start(shutdown_rx.clone()).await {
        Ok(handle) => handle,
        Err(why) => {
            let err_msg = format!("Web server failed to start: {:?}", why);
            tracing::error!("{}", &err_msg);
            interfaces::bot::telegram::send_alert("Web 伺服器啟動失敗", &err_msg).await;
            return Err(why.into());
        }
    };
    tracing::info!(
        "startup phase done: web::start elapsed={:?}",
        web_start_timer.elapsed()
    );
    tracing::info!(
        "startup phase done: main init total elapsed={:?}",
        startup_timer.elapsed()
    );

    // ── 13. gRPC 自我連線測試（延遲 1 秒執行）──────────────────────────────
    // gRPC 伺服器 start() 只是「開始監聽」，實際的 accept loop 可能還需幾毫秒就緒。
    // 延遲 1 秒確保伺服器已完全準備好再做自我連線，避免誤報連線失敗。
    // 使用 `tokio::spawn` 非阻塞地執行，不延誤主程式繼續往下走。
    // 測試失敗時僅發 Telegram 告警，不中止主程式（gRPC 非核心爬蟲功能）。
    // 注意：TLS 憑證過期（Let's Encrypt 90 天）會讓此測試失敗，請留意到期日。
    let grpc_self_test = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        if let Err(why) = interfaces::rpc::client::test_client::run_test().await {
            let err_msg = format!("gRPC 自我測試失敗: {:?}", why);
            tracing::error!("{}", &err_msg);
            interfaces::bot::telegram::send_alert("gRPC 自我測試失敗", &err_msg).await;
        }
    });

    // ── 14. Redis 連線驗證 ───────────────────────────────────────────────────
    // 對 Redis 發送 PING 指令，預期回傳 "PONG"。
    // Redis 用途：即時股價快照緩存、排程鎖（避免分散式重複觸發）等。
    // 驗證失敗時不中止程式（爬蟲的 PostgreSQL 路徑仍可運作），
    // 但會寫 error log 並發 Telegram 告警，讓 ops 儘快修復 Redis 連線。
    let pong = infra::nosql::redis::CLIENT.ping().await;
    match pong {
        Ok(pong_val) => {
            // 印到 stdout 方便在容器 log 一眼確認 Redis 就緒。
            println!("pong: {}", pong_val);
        }
        Err(why) => {
            let err_msg = format!("Redis ping failed at startup: {:?}", why);
            tracing::error!("{}", &err_msg);
            interfaces::bot::telegram::send_alert("Redis 快取連線失敗", &err_msg).await;
        }
    }

    Ok(Running {
        shutdown_tx,
        sched,
        rpc_server,
        web_server,
        grpc_self_test,
    })
}
