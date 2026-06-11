
### Taiwan stock crawler

台股資料採集、排程更新、手動回補、價格追蹤提醒與 gRPC/HTTP 管理介面服務。

UI Demo︰https://jiansoft.mooo.com/stock/revenues
API︰https://github.com/jiansoft/stock_api

## 專案功能與用途

+ 依排程採集台股主檔、營收、財報、股利、法人資金流向、ETF、收盤報價、指數與外資持股等資料。
+ 開盤期間啟動即時報價背景採集，依 `trace` 設定監控個股高低標，並透過 Telegram 發送提醒。
+ 提供 gRPC 服務給外部系統呼叫股票更新、即時報價、假日表與手動回補功能。
+ 提供本機 HTTP 手動回補管理頁與 API，可建立、查詢與追蹤回補工作。
+ 使用 PostgreSQL 保存業務資料，使用 Redis 保存跨程序通知去重與部分執行狀態，並使用記憶體快取加速查詢。
+ 日誌會寫入 `log/`，若設定 Seq 連線資訊也會同步送到 Seq。

## 技術棧

+ Rust 2024，主要 runtime 為 Tokio。
+ Web/API：Axum、Tonic gRPC、Prost。
+ 資料庫與快取：SQLx PostgreSQL、deadpool-redis、Moka memory cache。
+ 爬蟲與解析：Reqwest、Scraper、Regex、Serde/serde_json。
+ 排程：tokio-cron-scheduler。
+ 設定與環境變數：config、dotenv。
+ TLS/憑證：rustls、rustls-pemfile、x509-parser。

## 架構總覽

```text
src/
├─ main.rs           # 程式入口，載入設定、初始化日誌、DB/Redis、排程、gRPC 與 Web
├─ core/             # 共用基礎（config / declare / util / logging）
├─ domain/           # 領域模型與倉儲合約（config / dividend / financial / quote / trace 等）
├─ app/              # 應用層（scheduler / backfill / event / calculation）
├─ infra/            # 基礎設施（crawler / database / cache / nosql）
└─ interfaces/       # 對外介面（rpc / web / bot）

etc/
├─ proto/            # gRPC proto，build.rs 會產生 Rust stub 到 src/interfaces/rpc
└─ sql/              # PostgreSQL schema 與初始化 SQL

docs/                # 架構與工作紀錄文件
log/                 # runtime 檔案日誌輸出目錄
```

> 詳細分層規則與新模組放置規範請參閱 [docs/architecture.md](docs/architecture.md)。

## 領域驅動設計 (DDD) 重構狀態

+ 目前程式碼已採 `core`、`domain`、`app`、`infra`、`interfaces` 分層。
+ `domain/` 目前包含系統設定、股利、事件、財務、指數、資金流、投資組合、報價、證券主檔、價格追蹤與殖利率排行等領域。
+ `docs/checklists/` 保留 Phase 1~17 的歷史重構紀錄；這些文件是否仍完整代表目前待辦狀態：待確認（To Be Verified）。

## 開發環境需求

+ Rust stable toolchain；專案使用 Rust 2024 edition，CI 目前以 Rust `1.95.0` 驗證。
+ PostgreSQL 與 Redis；完整測試需先依 `.github/workflows/rust.yml` 的 SQL 順序初始化資料庫。
+ `build.rs` 使用 `protoc-bin-vendored` 取得 vendored `protoc`，並透過 `prost-build` config 指定 executable；一般情況不需要另外安裝 `protoc` 或設定 `PROTOC` 環境變數。
+ 跨平台 ARM Linux build 腳本會用到 Zig、CMake、`cargo-zigbuild` 或交叉編譯器。
+ Docker 部署需 Docker engine，實際 Rust runtime 映像檔以 `Dockerfile_live` 為準。

## 執行方式

+ 先準備 `app.json`，正式密碼、Token、API Key 建議放在 `.env` 或系統環境變數。
+ 初始化 PostgreSQL schema，並確認 Redis 可連線。
+ 本機開發可使用 `cargo run` 啟動服務。
+ 程式啟動後會先檢查 PostgreSQL 連線，再載入共享快取、啟動排程、gRPC server、HTTP 手動回補 server、Telegram/Redis 相關檢查。
+ `system.grpc_use_port` 不為 `0` 時會在 `0.0.0.0:{port}` 啟動 gRPC server；`app.json` 預設為 `9001`。
+ HTTP 手動回補 server 預設監聽 `127.0.0.1:9002`，可用 `MANUAL_BACKFILL_WEB_ADDR` 覆蓋。

## 建置、測試與格式化

+ `cargo build --verbose`：本機 debug build。
+ `cargo build --release`：release build。
+ `cargo fmt --all -- --check`：檢查格式，CI 使用此指令。
+ `cargo clippy -- -D warnings`：檢查 lint，CI 使用此指令。
+ `cargo test --release -- --nocapture --test-threads=1`：CI 測試指令；需要 PostgreSQL/Redis 與初始化 SQL。
+ `cargo test module::tests::test_name -- --nocapture`：執行單一測試。

## 部署方式

+ `control.sh build|start|stop|restart|update` 會以本機 release binary `stock_crawler` 啟停服務。
+ `control.sh docker_build|docker_start|docker_stop|docker_restart|docker_update` 會使用 `Dockerfile_live` 建立並啟停 Docker container。
+ `Dockerfile_live` 會複製 release binary、`.env`、`app.json` 到 `/app`，以 distroless nonroot runtime 執行，並 expose `9001`。
+ `control.sh docker_start` 預設建立 `stock-rust-container`，映射 `9001`、`9002`，並掛載 `log/` 與 SSL 憑證目錄。
+ `build.ps1` / `build.bat` 會使用 `cargo zigbuild --target aarch64-unknown-linux-musl --release` 建置 ARM Linux musl binary。
+ `build.sh` 會以 `aarch64-unknown-linux-gnu` target 建置 release binary。
+ 根目錄 `Dockerfile` 內容仍指向 Go 專案檔案，是否仍有使用場景：待確認（To Be Verified）。目前部署文件與腳本以 `Dockerfile_live` 為準。

## 排程時間

以下排程時間為台北時間（Asia/Taipei），依 `src/app/scheduler.rs` 為準。

+ 01:00 更新興櫃股票的每股淨值
+ 02:30 更新盈餘分配率
+ 03:00 更新台股季度 EPS
+ 04:00 更新季度財報中 ROE/ROA 為零的資料
+ 05:00 更新台股年度 EPS
+ 05:05 更新台股年度財報
+ 05:10 將未下市但每股淨值為零的股票更新其數據
+ 05:15 更新各股的當月營收
+ 05:20 更新台股國際證券識別碼
+ 05:25 更新下市股票
+ 05:30 更新 ETF 資料
+ 08:00 提醒本日與次一交易日除權息的股票（需自行架設本服務）
+ 08:02 提醒本日自持股票發放股利（需自行架設本服務）
+ 08:04 提醒本日開始公開申購的股票（需自行架設本服務）
+ 09:00 更新股票權值佔比
+ 09:02 啟動股票追蹤高低標提醒任務
+ 15:00 取得台股收盤報價數據並計算預估價格
+ 21:00 更新尚無年度配息資料的股票
+ 22:00 更新外資持股狀態
+ DDNS IP 自動更新功能已自本專案移除，相關功能請改用 https://github.com/jiansoft/dynip。

## 資料來源
1. 理財寶-股市爆料同學會 https://www.cmoney.tw/forum/popular
2. 鉅亨網 https://www.cnyes.com
3. 富邦證券 https://www.fbs.com.tw
4. Fugle 行情 API https://developer.fugle.tw/docs/data/http-api/getting-started/
5. 臺灣銀行 https://fund.bot.com.tw
6. 台灣股市資訊網 https://goodinfo.tw/tw
7. 嗨投資 https://histock.tw
8. PCHOME(大時科技) https://pchome.megatime.com.tw
9. 嘉實資訊-理財網 https://www.moneydj.com
10. 公開資訊觀測站 https://mopsfin.twse.com.tw
11. 恩投資 https://www.nstock.tw
12. 台灣期貨交易所 https://www.taifex.com.tw
13. 台灣證券櫃檯買賣中心 https://www.tpex.org.tw
14. 台灣證券交易所 https://www.twse.com.tw
15. 撿股讚 https://stock.wespai.com
16. Winvest https://winvest.tw
17. 雅虎股市 https://tw.stock.yahoo.com
18. 元大證券 https://www.yuanta.com.tw

## 主要設定

+ 所有設定可透過 `app.json` 提供，並可由 `.env` 或系統環境變數覆蓋。
+ `Cargo.toml` 的 package / binary 名稱目前仍為 `stock_crawler`；Seq 日誌服務識別則使用 `service=stock_rust`。
+ `app.json` 主要包含 `system`、`logging.seq`、資料來源 API、PostgreSQL、Telegram、Redis 與外部 Go gRPC 連線設定。
+ `logging.seq.serverUrl` 與 `logging.seq.apiKey` 可提供預設值，正式值建議放在 `.env` 的 `SEQ_SERVER_URL`、`SEQ_API_KEY`。
+ 未設定 `SEQ_SERVER_URL` 時會停用 Seq 轉送；有設定時會以 CLEF 格式送到 Seq `/api/events/raw?clef`。
+ Seq 事件只送出 `service=stock_rust` 作為服務識別，不送出額外的 `App` 或 `Application` 欄位。
+ 即時報價備援來源包含 Fugle 官方日內行情 API；若未設定 `FUGLE_API_KEY`，系統會略過 Fugle 並繼續嘗試其他來源。
+ `afraid`、`dynu`、`noip` 設定結構仍存在於 `app.json` / `core::config`，但目前主啟動流程未呼叫 DDNS 更新；實際使用狀態待確認（To Be Verified）。

## 對外介面

+ gRPC server 依 `system.grpc_use_port` 啟動，並註冊 `Control`、`ManualBackfill`、`Stock` 三個服務。
+ gRPC TLS 會在 `system.ssl_cert_file` 與 `system.ssl_key_file` 都有設定時啟用。
+ `Stock` gRPC 服務提供 `UpdateStockInfo`、`FetchCurrentStockQuotes`、`FetchHolidaySchedule`。
+ `ManualBackfill` gRPC 服務提供每日報價、收盤彙總、台股加權指數、持股股利重算、單檔/多檔歷史股利回補，以及 job 查詢。
+ HTTP 手動回補頁面位於 `/manual-backfill`，API 包含 `/api/manual-backfill/jobs`、`/api/manual-backfill/jobs/{id}` 與多個 `POST /api/manual-backfill/*` 回補入口。
+ Telegram bot 目前用於排程提醒、價格追蹤通知與部分錯誤告警。

## 盤中即時報價與追蹤

+ 開盤期間會同時啟動 HiStock 與 Yahoo 類股背景採集，將即時報價寫入共享記憶體快取。
+ Yahoo 類股採集使用 `StockServices.getClassQuotes` JSON API；同類股分頁之間節流 1 秒，類股之間使用 2 至 4 秒隨機延遲。
+ Yahoo 類股目前不採集認購、認售、指數類，避免將大量衍生性商品帶進盤中輪詢。
+ 股票追蹤高低標判斷統一從共享快取讀值；備援抓價只負責補快取並觸發重新判斷。
+ 若服務在開盤期間重啟，啟動排程時會先嘗試補啟動一次股票追蹤任務，避免錯過原本的 09:02 排程。
+ 單股最新成交價備援站點：Yahoo、Fugle、NStock、CMoney、CnYes、PcHome、Winvest。
+ 單股完整報價備援站點：Fugle、NStock、CMoney、CnYes、PcHome、Winvest。
+ `Yuanta` crawler module 仍存在，但目前不在最新成交價或完整報價備援池中，因程式註解記錄其資料曾觀察為前一交易日資料。

## 常用環境變數

+ `SEQ_SERVER_URL`、`SEQ_API_KEY`：Seq 日誌收集服務網址與 API Key；未設定 `SEQ_SERVER_URL` 時停用 Seq 轉送。
+ `FUGLE_API_KEY`：Fugle 日內行情 API 金鑰（即時報價備援）。
+ `TELEGRAM_TOKEN`、`TELEGRAM_ALLOWED`：Telegram Bot 與允許通知的 chat 設定。
+ `POSTGRESQL_HOST`、`POSTGRESQL_PORT`、`POSTGRESQL_USER`、`POSTGRESQL_PASSWORD`、`POSTGRESQL_DB`：PostgreSQL 連線設定。
+ `REDIS_ADDR`、`REDIS_ACCOUNT`、`REDIS_PASSWORD`、`REDIS_DB`：Redis 連線設定。
+ `SYSTEM_GRPC_USE_PORT`、`SYSTEM_SSL_CERT_FILE`、`SYSTEM_SSL_KEY_FILE`：本服務 gRPC 與 TLS 憑證設定。
+ `MANUAL_BACKFILL_WEB_ADDR`：HTTP 手動回補 server 監聽位址，預設 `127.0.0.1:9002`。
+ `GO_GRPC_TARGET`、`GO_GRPC_TLS_CERT_FILE`、`GO_GRPC_TLS_KEY_FILE`、`GO_GRPC_DOMAIN_NAME`：對外 Go gRPC 服務連線設定。


### 免責聲明
本網提供之所有資訊內容均僅供參考，不涉及買賣投資之依據。使用者在進行投資決策時，務必自行審慎評估，
並自負投資風險及盈虧，如依本網提供之資料交易致生損失，本網不負擔任何賠償及法律責任。您自行負責依據
自身投資目標及個人、財務狀況，確定任何投資、證券或任何其他投資產品服務是否適合自身的需要。
本網站所載或本網站上、通過本網站提供的任何服務、內容、資訊及/或資料在任何情況下均不得被解釋為提供投資、
法律意見或提供投資服務。特請訪問此類網頁的人士就有關任何本網資料是否適合其投資需求徵詢適當獨立專業意見。
