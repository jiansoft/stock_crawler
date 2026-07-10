//! `stock_crawler` 可執行程式入口。
//!
//! 主要職責：
//! - 載入環境與主快取。
//! - 啟動排程與 gRPC 服務。
//! - 註冊結束訊號，讓行程可平順停止。
//! - 啟動後做一次 gRPC 自我連線驗證。

// ── 條件編譯引用（musl + mimalloc）─────────────────────────────────────────
// musl 是 Linux 上的靜態 C 函式庫（相對於 glibc 的動態版本）。
// 使用 musl 目標編譯出的執行檔不依賴系統 glibc，非常適合放進 Alpine Linux 容器，
// 可大幅縮小映像體積（scratch / alpine 基底映像）。
// glibc 內建的 ptmalloc 在 musl 環境下效能不佳；mimalloc 是 Microsoft 開源的
// 高效能記憶體分配器，可大幅改善小物件頻繁分配的場景（例如爬蟲 HTTP 回應解析）。
#[cfg(all(target_os = "linux", target_env = "musl"))]
use mimalloc::MiMalloc;
// `c_char` 用於與 C 的 setenv() 互動，傳遞 null-terminated 字串指標。
#[cfg(all(target_os = "linux", target_env = "musl"))]
use std::ffi::c_char;

// ── 標準函式庫引用 ───────────────────────────────────────────────────────────
use std::{
    error::Error,
    // Instant 是單調遞增時鐘，精確到奈秒，用於效能量測。
    // 與 SystemTime 的差別：不受使用者調整系統時間影響，適合計算程式執行時間。
    time::Instant,
};

// ── Tokio 非同步訊號引用 ──────────────────────────────────────────────────────
// Unix 平台上的細粒度訊號（SIGINT = Ctrl+C、SIGTERM = 系統/容器關閉）。
// Windows 上沒有 POSIX 訊號機制，因此用 cfg(unix) 限制，避免編譯錯誤。
// `unix_signal` 重命名以避免與模組名稱 `signal` 衝突。
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal as unix_signal};
// tokio-cron-scheduler 是基於 Tokio 的非同步 cron 排程器，支援標準 cron 語法。
// 每個 job 在到期時以非同步 task 形式在 Tokio 執行緒池中執行，不阻塞主執行緒。
use tokio_cron_scheduler::JobScheduler;

// ── 全域記憶體分配器（musl 目標）────────────────────────────────────────────
// `#[global_allocator]` 告訴 Rust 編譯器：用 MiMalloc 取代系統預設的 malloc/free。
// 只在 musl 目標啟用；glibc 目標使用系統 malloc（glibc 2.17+ 的 ptmalloc 已夠好）。
#[cfg(all(target_os = "linux", target_env = "musl"))]
#[global_allocator]
static GLOBAL_ALLOCATOR: MiMalloc = MiMalloc;

/// 在 `main` 之前為 musl + mimalloc 設定較積極的回收策略。
///
/// 這些選項來自 mimalloc 官方環境參數：
/// - `MIMALLOC_PURGE_DELAY=0`：頁面一旦空出就立即 purge。
/// - `MIMALLOC_PURGE_DECOMMITS=1`：purge 時用 decommit / `MADV_DONTNEED`，讓 RSS 較快下降。
/// - `MIMALLOC_ALLOW_THP=0`：避免 THP 讓 purge 顆粒變粗，影響回收速度。
/// - `MIMALLOC_ALLOW_LARGE_OS_PAGES=0`：避免不可 purge 的 large OS pages。
/// - `MIMALLOC_ARENA_EAGER_COMMIT=0`：避免過早 commit 大 arena。
#[cfg(all(target_os = "linux", target_env = "musl"))]
#[used]
// `.init_array` 是 ELF 二進位格式的初始化函式指標陣列。
// 動態連結器在執行 main() 之前會逐一呼叫陣列中的函式指標，
// 確保環境變數在 mimalloc 第一次分配記憶體前就已設好。
// `overwrite = 0` 表示若環境中已有該變數則不覆蓋，方便 Docker 從外部覆寫行為。
#[unsafe(link_section = ".init_array")]
static INIT_MIMALLOC_ENV: extern "C" fn() = init_mimalloc_env;

/// 透過 libc `setenv` 設定 mimalloc 執行期旗標。
///
/// 之所以用 `extern "C" fn` 而非 `#[ctor]` crate，是為了盡量減少外部依賴，
/// 並維持對 `.init_array` 機制的完整控制。
#[cfg(all(target_os = "linux", target_env = "musl"))]
extern "C" fn init_mimalloc_env() {
    // Safety: 所有字串字面值均為 null-terminated（\0 結尾）且具靜態生命週期，
    // 指標永遠有效；overwrite=0 保證不覆蓋外部設定。
    unsafe {
        setenv(
            b"MIMALLOC_PURGE_DELAY\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_PURGE_DECOMMITS\0".as_ptr().cast::<c_char>(),
            b"1\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_ALLOW_THP\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_ALLOW_LARGE_OS_PAGES\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_ARENA_EAGER_COMMIT\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
    }
}

// libc setenv 的 Rust FFI 宣告。
// `unsafe extern "C"` 表示這是遵循 C ABI 的外部函式，
// 呼叫者必須自行確保參數合法（指標不為 null、字串以 \0 結尾）。
#[cfg(all(target_os = "linux", target_env = "musl"))]
unsafe extern "C" {
    fn setenv(name: *const c_char, value: *const c_char, overwrite: i32) -> i32;
}

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
// main.rs 只透過 `use` 引入，不再用 `pub mod` 重新宣告整棵模組樹，
// 避免 `cargo test` 把同一組程式碼（含所有單元測試）分別編譯進 lib 與 bin
// 兩個測試執行檔、對同一組 CI 服務容器重跑兩遍。
use stock_crawler::{app, core, infra, interfaces};

/// 在 Unix 平台等待 `SIGINT`（Ctrl+C）或 `SIGTERM`（Docker/systemd 關閉訊號）。
///
/// `tokio::select!` 同時 await 兩個訊號流，哪個先到就哪個分支觸發退出；
/// 另一個分支對應的 future 會被自動取消（drop），不造成資源洩漏。
///
/// 若 Unix 訊號建立失敗（極罕見，通常是 fd 耗盡），向上回傳錯誤並中止服務，
/// 避免程序在無法接收關機訊號的狀態下繼續運作。
#[cfg(unix)]
async fn shutdown_signal() -> Result<(), Box<dyn Error>> {
    let mut sigint = unix_signal(SignalKind::interrupt())?;
    let mut sigterm = unix_signal(SignalKind::terminate())?;

    tokio::select! {
        _ = sigint.recv() => {}
        _ = sigterm.recv() => {}
    }

    Ok(())
}

/// 在非 Unix 平台等待 `Ctrl+C` 關機訊號。
///
/// Windows 沒有 POSIX SIGTERM，因此使用 Tokio 提供的跨平台 Ctrl+C future。
#[cfg(not(unix))]
async fn shutdown_signal() -> Result<(), Box<dyn Error>> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// 等待單一 server task 完成平順關機，逾時時才強制取消該 accept loop。
///
/// Server 內部已先收到 shutdown 訊號並停止接受新連線；此 timeout 只用來避免
/// 異常或永不結束的連線讓整個程序永久卡在關機階段。
async fn wait_for_server_shutdown(
    server_name: &str,
    mut handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    timeout: tokio::time::Duration,
) {
    match tokio::time::timeout(timeout, &mut handle).await {
        Ok(Ok(Ok(()))) => tracing::info!(server = server_name, "server graceful shutdown done"),
        Ok(Ok(Err(why))) => {
            tracing::error!(server = server_name, error = %why, "server stopped with error")
        }
        Ok(Err(why)) => {
            tracing::error!(server = server_name, error = %why, "server task join failed")
        }
        Err(_) => {
            tracing::warn!(
                server = server_name,
                "server graceful shutdown timed out; aborting task"
            );
            handle.abort();
            let _ = handle.await;
        }
    }
}

/// 啟動 `stock_crawler` 主流程。
///
/// `#[tokio::main]` 宏在底層建立多執行緒 Tokio runtime（`features = ["full"]` 啟用全部功能），
/// 並將 `async fn main()` 包在 `runtime.block_on()` 裡執行。
/// 回傳 `Result<(), Box<dyn Error>>` 讓任何啟動錯誤都能顯示給使用者，
/// 並以非零 exit code 退出（讓 systemd / Docker restart policy 能偵測到異常並重啟）。
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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
    }

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

    // ── 8. 全域快取預熱（SHARE.load）────────────────────────────────────────
    // `infra::cache::SHARE` 是全域單例，儲存所有股票的即時快照（RealtimeSnapshot）
    // 及基本資訊（名稱、市場別、產業類別等）。
    // `.load()` 從 PostgreSQL 載入全部股票清單並建立 HashMap，耗時視股票數量而定
    // （台股約 1,800～2,000 支，通常 1～3 秒內完成）。
    // 後續爬蟲與排程任務都透過此快取直接查詢，避免頻繁打 DB；
    // 即時股價更新（set_stock_snapshot_price）也在記憶體內完成，不需回寫 DB。
    tracing::info!(
        "{}",
        "startup phase begin: crate::infra::cache::SHARE.load".to_string(),
    );
    let cache_load_timer = Instant::now();
    infra::cache::SHARE.load().await;
    tracing::info!(
        "startup phase done: crate::infra::cache::SHARE.load elapsed={:?}",
        cache_load_timer.elapsed()
    );

    // ── 9. 排程器建立 ───────────────────────────────────────────────────────
    // `JobScheduler::new()` 建立 tokio-cron-scheduler 實例，但尚未啟動任何 job。
    // 內部會建立 Tokio channel 與背景輪詢 task，因此需要 .await。
    // `?` 操作子：若建立失敗（極罕見，通常是 Tokio runtime 問題）直接向上回傳錯誤。
    tracing::info!("startup phase begin: JobScheduler::new");
    let scheduler_new_timer = Instant::now();
    let mut sched = JobScheduler::new().await?;
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
    let pong = crate::infra::nosql::redis::CLIENT.ping().await;
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

    // ── 15. 等待 OS 關機訊號 ─────────────────────────────────────────────────
    //
    // 新進開發者可把下方關機程式理解成五道閘門：
    //
    //   1. Server 閘門：先拒絕新 request，避免關機期間工作持續增加。
    //   2. Scheduler 閘門：停止 cron ticker，避免又啟動新的每日更新。
    //   3. Request 閘門：等待已進入 HTTP/gRPC handler 的 request 返回。
    //   4. 資料閘門：等待 handler/cron 已建立的 backfill 與 DB 寫入完成。
    //   5. 常駐 task 閘門：最後停止只負責盤中輪詢、可以安全重建的價格 task。
    //
    // 這個順序很重要：如果先停掉底層價格 task，再等待上層 request，正在處理的 request
    // 可能突然失去相依服務；如果先等背景工作但仍接受新 request，active count 又永遠可能增加。
    // 直接 await 訊號 future，不再以 AtomicBool 每 100ms 輪詢，收到訊號後立即進入流程。
    shutdown_signal().await?;
    tracing::info!("graceful shutdown begin");

    // 第 1 道閘門：watch 的值從 false 改成 true。
    // Axum/Tonic 收到後停止 accept 新連線；已經進入 handler 的 request 不會被硬切斷。
    let _ = shutdown_tx.send(true);

    // 第 2 道閘門：只停止「未來的排程觸發」。已開始的 cron future 不會被 scheduler
    // 強制 abort，而是由下方 BACKGROUND_OPERATIONS 負責等待，避免資料只寫一半。
    if let Err(why) = sched.shutdown().await {
        tracing::error!(error = %why, "scheduler shutdown failed");
    }

    // 自我測試不是必要的資料操作；若關機時仍未完成，主動取消以免延長服務停止時間。
    if !grpc_self_test.is_finished() {
        grpc_self_test.abort();
    }
    let _ = grpc_self_test.await;

    // 第 3 道閘門：HTTP/gRPC 各自排空既有 request。
    // 30 秒是 server 連線層的上限；它與下方資料工作五分鐘 timeout 是不同層級。
    let server_shutdown_timeout = tokio::time::Duration::from_secs(30);
    if let Some(rpc_server) = rpc_server {
        wait_for_server_shutdown("grpc", rpc_server, server_shutdown_timeout).await;
    }
    wait_for_server_shutdown("web", web_server, server_shutdown_timeout).await;

    // 第 4 道閘門：Server 已停止處理 request，scheduler 也不會再派發工作，因此 active
    // operation 只會減少、不會持續增加。先等待既有操作，避免提早停止其相依的價格服務。
    // 五分鐘到期後只記錄警告並繼續關機，避免單一故障 upstream 讓部署永遠無法停止。
    let background_timeout = tokio::time::Duration::from_secs(5 * 60);
    let operations = &core::shutdown::BACKGROUND_OPERATIONS;
    if !operations.wait_for_idle(background_timeout).await {
        tracing::warn!(
            active_operations = operations.active_count(),
            "graceful shutdown timed out while waiting for background operations"
        );
    }

    // 第 5 道閘門：資料操作完成後才停止盤中長生命週期 task。
    // stop_price_tasks 會喚醒正在 sleep/ticker 的 task，無須等待下一個 5 分鐘週期自然到期。
    app::event::trace::price_tasks::stop_price_tasks().await;

    tracing::info!("graceful shutdown done");
    println!("Server stopped gracefully");

    Ok(())
}
