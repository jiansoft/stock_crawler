
### Taiwan stock crawler

台股資料採集、排程更新、手動回補、價格追蹤提醒、唯讀 Data API 與 gRPC/HTTP 管理介面服務。

UI Demo︰https://jiansoft.mooo.com/stock/revenues
API︰https://github.com/jiansoft/stock_api

## 專案功能與用途

+ 依排程採集台股主檔、營收、財報（含損益表／資產負債表／現金流量表三大報表）、股利、減資、法人資金流向、ETF、收盤報價、指數與外資持股等資料。
+ 計算預估價格（便宜／合理／昂貴價）、移動平均、各期間年化報酬率（CAGR）與市場廣度統計。
+ 開盤期間啟動即時報價背景採集，依 `trace` 設定監控個股高低標，並透過 Telegram 發送提醒。
+ 提供 gRPC 服務給外部系統呼叫股票更新、即時報價、假日表與手動回補功能。
+ 提供 HTTP 手動回補管理頁與 API，可建立、查詢與追蹤回補工作。
+ 提供唯讀 Data API（`/api/v1/*`，附 OpenAPI 文件與 Swagger UI），供外部服務（例如 MCP server）查詢股票資料。
+ 使用 PostgreSQL 保存業務資料，使用 Redis 保存跨程序通知去重與部分執行狀態，並使用記憶體快取加速查詢。
+ 日誌會寫入 `log/`（依日期與大小輪替），若設定 Seq 連線資訊也會同步送到 Seq。

## 技術棧

+ Rust 2024，主要 runtime 為 Tokio；musl 建置使用 mimalloc 作為記憶體配置器。
+ Web/API：Axum、Tonic gRPC、Prost、utoipa（OpenAPI）、utoipa-swagger-ui。
+ 資料庫與快取：SQLx PostgreSQL、deadpool-redis、Moka memory cache。
+ 爬蟲與解析：Reqwest、Scraper、Regex、Serde/serde_json。
+ 排程：tokio-cron-scheduler。
+ 日誌：tracing、tracing-subscriber（FileLogLayer 輪轉日誌 + Seq 轉發）。
+ 錯誤型別：anyhow（應用層）、thiserror（infra 層結構化錯誤）。
+ 設定與環境變數：`app.json` + dotenvy（`.env`）。
+ TLS/憑證：rustls、rustls-pki-types、x509-parser。

## 架構總覽

```text
src/
├─ main.rs           # 程式入口，載入設定、初始化日誌、DB/Redis、排程、gRPC 與 Web，並處理優雅關機
├─ core/             # 共用基礎（config / declare / util / logging / shutdown / alert）
├─ domain/           # 領域模型與倉儲合約
│                    #   config / dividend / events / financial / foreign_holding / market_index
│                    #   money_flow / performance / portfolio / quote / registry / trace / yield_rank
├─ app/              # 應用層（scheduler / backfill / event / calculation / manual_backfill）
├─ infra/            # 基礎設施（crawler / database / cache / nosql）
└─ interfaces/       # 對外介面（rpc / web：backfill_admin、data_api / bot）

benches/             # criterion 效能基準（twse_quote_parser / daily_quote_mapper / cache_snapshot_lookup）
etc/
├─ proto/            # gRPC proto（basic / control / manual_backfill / stock），build.rs 會產生 Rust stub 到 src/interfaces/rpc
└─ sql/              # PostgreSQL schema 與初始化 SQL（CI 依此建立測試資料庫）

docs/                # 架構文件（architecture.md）
log/                 # runtime 檔案日誌輸出目錄
```

> 詳細分層規則與新模組放置規範請參閱 [docs/architecture.md](docs/architecture.md)。

## 領域驅動設計 (DDD) 重構狀態

+ 目前程式碼已採 `core`、`domain`、`app`、`infra`、`interfaces` 分層，依賴方向由外向內（interfaces/infra → app → domain）。
+ `domain/` 包含 13 個領域：config、dividend、events、financial、foreign_holding、market_index、money_flow、performance、portfolio、quote、registry、trace、yield_rank。
+ 分層規則與模組放置規範詳見 [docs/architecture.md](docs/architecture.md)。

## 開發環境需求

+ Rust 1.95 以上（`Cargo.toml` 的 `rust-version`）；CI 以 Rust `1.95.0` 與最新 stable 兩條 toolchain 驗證。
+ PostgreSQL 與 Redis；整合測試需先依 `.github/workflows/rust.yml` 的 SQL 順序（`etc/sql/*.sql`）初始化資料庫。
+ `build.rs` 使用 `protoc-bin-vendored` 取得 vendored `protoc`，並透過 `tonic-prost-build` config 指定 executable；一般情況不需要另外安裝 `protoc` 或設定 `PROTOC` 環境變數。
+ 跨平台 ARM Linux build 腳本會用到 Zig、CMake、`cargo-zigbuild` 或交叉編譯器。
+ Docker 部署需 Docker engine（BuildKit），實際 Rust runtime 映像檔以 `Dockerfile` 為準。

## 執行方式

+ 先準備 `app.json`，正式密碼、Token、API Key 建議放在 `.env` 或系統環境變數。
+ 初始化 PostgreSQL schema，並確認 Redis 可連線。
+ 本機開發可使用 `cargo run` 啟動服務。
+ 程式啟動後會先檢查 PostgreSQL 連線，再載入共享快取、啟動排程、gRPC server、HTTP server（手動回補與 Data API）、Telegram/Redis 相關檢查。
+ `system.grpc_use_port` 不為 `0` 時會在 `0.0.0.0:{port}` 啟動 gRPC server；`app.json` 預設為 `9001`。
+ HTTP server 預設監聽 `127.0.0.1:9002`，可用 `MANUAL_BACKFILL_WEB_ADDR` 覆蓋（例如 `0.0.0.0:9002` 讓其他主機呼叫 Data API）。
+ 收到 `SIGINT`／`SIGTERM`（Windows 為 Ctrl+C）時優雅關機：HTTP 與 gRPC 停止接收新請求並處理完進行中的請求，並等待進行中的排程與回補工作結束（皆有等待上限）。

## 建置、測試與格式化

+ `cargo build --release`：release build。
+ `cargo fmt --all -- --check`：檢查格式（CI gate）。
+ `cargo clippy --all-targets --all-features --locked -- -D warnings`：檢查 lint（CI gate）。
+ `cargo deny check advisories licenses bans`：檢查相依套件的安全通報、授權與禁用套件（CI gate）。
+ `cargo test --locked -- --test-threads=1`：只跑單元測試；需要 PostgreSQL／Redis 的測試會自動略過，不需外部服務。
+ `cargo test --locked --features integration-tests -- --test-threads=1`：連同整合測試一起跑。**請先在 `.env` 設定 `TEST_POSTGRESQL_*` 指向獨立的測試資料庫，不要對正式庫執行**；寫入資料的測試使用固定歷史日期與假代號並會自行清除。
+ CI 以 `cargo nextest run --release --locked --features integration-tests --test-threads=1 --profile ci` 對 PostgreSQL／Redis service container 執行整合測試，並以 `cargo llvm-cov` 產出覆蓋率。
+ `cargo test --locked app::manual_backfill::<測試名稱> -- --ignored --nocapture`：手動回補入口（`src/app/manual_backfill.rs` 內以 `#[ignore]` 標記的測試）。
+ `cargo bench`：執行 criterion 效能基準。

## 部署方式

+ `scripts\deploy-armv7.ps1` 會把 ARMv7 執行檔一次部署到 Raspberry Pi 3：本機 ELF 預檢（32-bit ARM
  hard-float、靜態連結）→ scp 到 `/tmp` 並比對 sha256 → ssh 執行 `control.sh update` → 驗證 `9001`／`9002`
  是否監聽與 `/api/v1/healthz` 是否 200。加 `-Build` 會先建 armv7；加 `-StageOnly` 只上傳不重啟服務；
  驗證失敗時會印出回滾指令。不會同步 `.env` 與 `app.json`（裝置上那兩份是手動維護的）。建議在收盤後部署。
+ `build.ps1` / `build.bat` 會以 `cargo zigbuild --release` 建置 `aarch64-unknown-linux-musl` 與 `armv7-unknown-linux-musleabihf` 兩個靜態連結 binary，輸出為 `stock_crawler_arm64`、`stock_crawler_armv7`。
+ `build.sh` 會以 `aarch64-unknown-linux-gnu` target 建置 release binary。
+ `control.sh start|stop|restart|update|move|build` 以本機模式啟停服務，依 `uname -m` 自動選用 `stock_crawler_arm64` 或 `stock_crawler_armv7`。
+ `control.sh docker_build|docker_start|docker_stop|docker_restart|docker_update` 會使用 `Dockerfile` 建立並啟停 Docker container。
+ `Dockerfile` 會依 BuildKit 的目標平台（`linux/arm64`、`linux/arm/v7`）選用對應 binary，連同 `.env`、`app.json` 複製到 `/app`，以 distroless static nonroot 映像執行（時區 `Asia/Taipei`），並 expose `9001`。
+ `control.sh docker_start` 預設建立 `stock-rust-container`，映射 `9001`、`9002`，並掛載 `log/` 與 SSL 憑證目錄。

## 排程時間

以下排程時間為台北時間（Asia/Taipei，排程固定以 UTC+8 解讀，與主機時區無關），依 `src/app/scheduler.rs` 為準。

+ 01:00 更新興櫃股票的每股淨值
+ 02:30 更新盈餘分配率
+ 03:00 更新台股季度財報（EPS 等）
+ 04:00 補齊季度財報中 ROE/ROA 為零的資料
+ 05:00 更新台股年度 EPS
+ 05:05 更新台股年度財報
+ 05:10 將未下市但每股淨值為零的股票更新其數據
+ 05:15 更新各股的當月營收
+ 05:20 更新台股國際證券識別碼
+ 05:25 更新下市股票
+ 05:30 更新 ETF 資料
+ 05:32 回補交易所公告的減資事件
+ 05:35 掃描交易所除權息公告，補齊資料庫漏抓的股利事件與除權息日、現金股利發放日
+ 05:37 以交易所除權除息計算結果核對近 45 天的股利日期與金額；資料庫缺的事件先從 Yahoo 補，Yahoo 也沒有的 ETF 現金配息（除息已滿 30 天）改用交易所日期與金額補登
+ 05:40 計算各期間年化報酬率（CAGR）
+ 06:00 採集三大財務報表（損益表、資產負債表、現金流量表），每輪最多 400 檔、每檔約每週重抓一次；Yahoo 失敗（404 除外）時改從 BigGo 財經備援，以 `source = 'biggo'` 寫入
+ 08:00 提醒本日與次一交易日除權息的股票（需自行架設本服務）；月配、季配、半年配者另列年化殖利率（單次殖利率 × 一年配息次數）
+ 08:02 提醒本日自持股票發放股利（需自行架設本服務）
+ 08:04 提醒本日開始公開申購的股票（需自行架設本服務）
+ 09:00 更新股票權值佔比
+ 09:02 啟動股票追蹤高低標提醒任務
+ 15:00 取得台股收盤報價數據並計算預估價格
+ 17:00、19:00（週一至週五）檢查當日收盤資料：上市或上櫃有成交的檔數低於近期交易日的一半時重跑收盤匯總並以 Telegram 告警（只有一個市場有資料時收盤匯總也會拒絕寫入）
+ 20:30 通知持股近 7 天內法說會的摘要、展望與 Q&A 重點（BigGo 財經整理；摘要尚未產生的場次之後再通知，每場只通知一次）
+ 20:40（週一至週五）通知持股主力進出：主力（買超、賣超前 15 名分點）買賣超佔成交量達 20% 且達 100 張的持股，附買超、賣超前 3 名分點（富邦證券主力進出頁；休市日不發）
+ 20:45（每月 10–28 日）通知持股董監質押變動：與前一期比較設質、關係人設質增減與持股大幅增減（交易所董監事持股開放資料，約每月 18 日出表；第一次執行列出目前有設質的人作為基準）
+ 21:00 更新尚無年度配息資料的股票
+ 22:00 更新外資持股比例與狀態
+ 若服務在開盤期間重啟，啟動排程時會先補啟動一次股票追蹤任務，避免錯過 09:02 的排程。
+ DDNS IP 自動更新功能已自本專案移除，相關功能請改用 https://github.com/jiansoft/dynip。

## 資料來源
1. 理財寶-股市爆料同學會 https://www.cmoney.tw/forum/popular
2. 鉅亨網 https://www.cnyes.com
3. 富邦證券 https://www.fbs.com.tw（年度獲利、持股主力進出）
4. Fugle 行情 API https://developer.fugle.tw/docs/data/http-api/getting-started/
5. 嗨投資 https://histock.tw
6. PCHOME(大時科技) https://pchome.megatime.com.tw
7. 嘉實資訊-理財網 https://www.moneydj.com
8. 公開資訊觀測站 https://mops.twse.com.tw（經交易所開放資料取得股利分派情形、董監事持股與設質）
9. 恩投資 https://www.nstock.tw
10. 台灣期貨交易所 https://www.taifex.com.tw
11. 台灣證券櫃檯買賣中心 https://www.tpex.org.tw
12. 台灣證券交易所 https://www.twse.com.tw
13. 撿股讚 https://stock.wespai.com
14. 雅虎股市 https://tw.stock.yahoo.com
15. BigGo 財經 https://finance.biggo.com.tw（三大財務報表與即時報價的備援來源、持股法說會摘要）

+ `winvest`、`yuanta`、`bank_of_taiwan`（臺灣銀行）crawler module 仍在程式碼中，但目前沒有排程或介面使用；前兩者移出即時報價備援池的原因見「盤中即時報價與追蹤」。

## 主要設定

+ 所有設定可透過 `app.json` 提供，並可由 `.env` 或系統環境變數覆蓋。
+ `Cargo.toml` 的 package / binary 名稱目前仍為 `stock_crawler`；Seq 日誌服務識別則使用 `service=stock_rust`。
+ `app.json` 主要包含 `system`、`logging`、資料來源 API、PostgreSQL、Telegram、Redis 與外部 Go gRPC 連線設定。
+ `logging.seq.serverUrl` 與 `logging.seq.apiKey` 可提供預設值，正式值建議放在 `.env` 的 `SEQ_SERVER_URL`、`SEQ_API_KEY`。
+ 未設定 `SEQ_SERVER_URL` 時會停用 Seq 轉送；有設定時會以 CLEF 格式送到 Seq `/api/events/raw?clef`。
+ Seq 事件只送出 `service=stock_rust` 作為服務識別，不送出額外的 `App` 或 `Application` 欄位。
+ `logging.file.maxSizeMb`／`logging.file.maxAgeDays` 設定檔案日誌的大小上限與保留天數（`0` 代表用預設值 10 MB／7 天；限制在 1 MB–1 GB、1–365 天）。使用中的日誌檔固定為 `log/YYYY-MM-DD_default_{level}.log`，超過大小時改名為帶最後一筆時間的 `…{level}.HH-MM-SS.log` 再開新檔。
+ 即時報價備援來源包含 Fugle 官方日內行情 API；若未設定 `FUGLE_API_KEY`，系統會略過 Fugle 並繼續嘗試其他來源。

## 對外介面

+ gRPC server 依 `system.grpc_use_port` 啟動，並註冊 `ControlService`、`ManualBackfillService`、`StockService` 三個服務。
+ gRPC TLS 會在 `system.ssl_cert_file` 與 `system.ssl_key_file` 都有設定時啟用。
+ `StockService` gRPC 服務提供 `UpdateStockInfo`、`FetchCurrentStockQuotes`、`FetchHolidaySchedule`。
+ `ManualBackfillService` gRPC 服務提供每日報價、收盤彙總、台股加權指數、持股股利重算、單檔／多檔歷史股利、個股歷史報價、公司行動（減資等）登錄、CAGR 全期間／單一期間計算的回補，以及 job 查詢。
+ HTTP 手動回補頁面位於 `/manual-backfill`，API 包含 `/api/manual-backfill/jobs`、`/api/manual-backfill/jobs/{id}` 與多個 `POST /api/manual-backfill/*` 回補入口；工作在背景執行，請求會立即回傳 job id。
+ Data API 位於 `/api/v1/*`（與手動回補共用 HTTP server），提供股票搜尋、最新與歷史報價、即時快照、基本資料、月營收、財報、股利、估值、市場廣度、殖利率排行、選股、大盤指數、股利行事曆、外資持股排行、當日漲跌幅／成交量排行、CAGR 排行等唯讀查詢。
  - 除 `/api/v1/healthz` 外都需要 `Authorization: Bearer <DATA_API_KEY>`；未設定 `DATA_API_KEY` 時一律拒絕。
  - OpenAPI 文件在 `/api-docs/openapi.json`，Swagger UI 在 `/swagger-ui`（不需驗證）。
+ Telegram bot 用於排程提醒（除權息、股利發放、公開申購、持股財報、外資持股變化、持股法說會摘要、持股主力進出、持股董監質押變動）、價格追蹤通知與錯誤告警。

## 盤中即時報價與追蹤

+ 開盤期間會同時啟動 HiStock 與 Yahoo 類股背景採集，將即時報價寫入共享記憶體快取。
+ Yahoo 類股採集使用 `StockServices.getClassQuotes` JSON API；同類股分頁之間節流 1 秒，類股之間使用 2 至 4 秒隨機延遲。
+ Yahoo 類股不採集認購、認售、指數類、公司債與牛熊證等分類，避免將大量衍生性商品帶進盤中輪詢。
+ 寫入快取前會驗證價格：有當日漲跌停價（取自 Yahoo 類股報價）時以漲跌停區間判斷，無漲跌幅限制的標的改用較寬的門檻，其餘沿用固定漲跌幅門檻，避免異常報價觸發追蹤通知。
+ 股票追蹤高低標判斷統一從共享快取讀值；備援抓價只負責補快取並觸發重新判斷。
+ 單股最新成交價備援站點：Yahoo、Fugle、NStock、CMoney、CnYes、PcHome、BigGo。
+ 單股完整報價備援站點：Fugle、NStock、CMoney、CnYes、PcHome、BigGo。
+ BigGo 於 2026-10-05 盤中比對後加入：95% 報價落在證交所 MIS 的最佳買賣價之間，其餘只差一檔；當天未成交的股票 BigGo 會回前一個交易日的快照，爬蟲會拒收並改問下一個站點。
+ `Winvest` crawler module 仍存在，但 2026-09 改版後只提供盤後日 K（盤中實測回傳前一交易日資料），已移出兩個備援池。
+ `Yuanta` crawler module 仍存在，但目前不在最新成交價或完整報價備援池中，因程式註解記錄其資料曾觀察為前一交易日資料。

## 常用環境變數

+ `SEQ_SERVER_URL`、`SEQ_API_KEY`：Seq 日誌收集服務網址與 API Key；未設定 `SEQ_SERVER_URL` 時停用 Seq 轉送。
+ `LOG_FILE_MAX_SIZE_MB`、`LOG_FILE_MAX_AGE_DAYS`：覆蓋檔案日誌的大小上限與保留天數。
+ `FILE_LOG_LEVEL`：檔案日誌等級過濾，預設 `info,html5ever=off,rustls::msgs::handshake=error`。
+ `FUGLE_API_KEY`：Fugle 日內行情 API 金鑰（即時報價備援）。
+ `TELEGRAM_TOKEN`、`TELEGRAM_ALLOWED`：Telegram Bot 與允許通知的 chat 設定。
+ `POSTGRESQL_HOST`、`POSTGRESQL_PORT`、`POSTGRESQL_USER`、`POSTGRESQL_PASSWORD`、`POSTGRESQL_DB`：PostgreSQL 連線設定。
+ `TEST_POSTGRESQL_HOST`、`TEST_POSTGRESQL_PORT`、`TEST_POSTGRESQL_USER`、`TEST_POSTGRESQL_PASSWORD`、`TEST_POSTGRESQL_DB`：只在測試建置生效，覆蓋 `POSTGRESQL_*`，讓整合測試連到獨立測試庫。
+ `REDIS_ADDR`、`REDIS_ACCOUNT`、`REDIS_PASSWORD`、`REDIS_DB`、`REDIS_POOL_SIZE`：Redis 連線設定。
+ `SYSTEM_GRPC_USE_PORT`、`SYSTEM_SSL_CERT_FILE`、`SYSTEM_SSL_KEY_FILE`：本服務 gRPC 與 TLS 憑證設定。
+ `MANUAL_BACKFILL_WEB_ADDR`：HTTP server（手動回補與 Data API）監聽位址，預設 `127.0.0.1:9002`。
+ `DATA_API_KEY`：Data API 的 Bearer 金鑰；未設定時 Data API 拒絕所有受保護請求。
+ `GO_GRPC_TARGET`、`GO_GRPC_TLS_CERT_FILE`、`GO_GRPC_TLS_KEY_FILE`、`GO_GRPC_DOMAIN_NAME`：對外 Go gRPC 服務連線設定。


### 免責聲明
本網提供之所有資訊內容均僅供參考，不涉及買賣投資之依據。使用者在進行投資決策時，務必自行審慎評估，
並自負投資風險及盈虧，如依本網提供之資料交易致生損失，本網不負擔任何賠償及法律責任。您自行負責依據
自身投資目標及個人、財務狀況，確定任何投資、證券或任何其他投資產品服務是否適合自身的需要。
本網站所載或本網站上、通過本網站提供的任何服務、內容、資訊及/或資料在任何情況下均不得被解釋為提供投資、
法律意見或提供投資服務。特請訪問此類網頁的人士就有關任何本網資料是否適合其投資需求徵詢適當獨立專業意見。
