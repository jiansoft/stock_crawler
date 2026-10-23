
### Taiwan stock crawler

台股資料採集、排程更新、衍生指標計算、Telegram 通知、手動回補、唯讀 Data API 與 gRPC/HTTP 管理介面服務。

UI Demo︰https://jiansoft.mooo.com/stock/revenues
API︰https://github.com/jiansoft/stock_api

## 相關專案

+ [stock_go](https://github.com/jiansoft/stock_go)：網頁前端（Next.js）與 Go API，直接讀取本服務寫入的 PostgreSQL；手動回補經 gRPC 轉發到本服務。
+ stock-mcp-go：MCP server，透過本服務的 Data API（`/api/v1/*`）提供 AI 助理查詢工具。

## 專案功能與用途

+ 依排程採集台股主檔、營收、財報（含損益表／資產負債表／現金流量表三大報表）、股利、減資、三大法人與融資融券、主力進出、集保股權分散、董監持股與設質、ETF、收盤報價、指數與外資持股等資料。
+ 計算預估價格（便宜／合理／昂貴價）、移動平均、各期間年化報酬率（CAGR）、技術訊號、相對強度、未來報酬、市場廣度、市場估值水位與財務體質（Piotroski F 分數），並預先存成衍生資料表（見「衍生資料表」）。
+ 開盤期間啟動即時報價背景採集，依 `trace` 設定監控個股高低標，並透過 Telegram 發送提醒。
+ 每天、每週以 Telegram 通知持股的除權息、股利、月營收、財報、籌碼、技術訊號、估值與策略換股等變化（見「Telegram 通知」）。
+ 提供唯讀 Data API（`/api/v1/*`，附 OpenAPI 文件與 Swagger UI），含個股與市場查詢、條件選股、選股回測與持股組合分析。
+ 提供 gRPC 服務給外部系統呼叫股票更新、即時報價、假日表與手動回補功能，以及 HTTP 手動回補管理頁與 API。
+ 使用 PostgreSQL 保存業務資料，使用 Redis 保存跨程序通知去重與部分執行狀態，並使用記憶體快取加速查詢。
+ 日誌會寫入 `log/`（依日期與大小輪替），若設定 Seq 連線資訊也會同步送到 Seq。

## 技術棧

+ Rust 2024，主要 runtime 為 Tokio；musl 建置使用 mimalloc 作為記憶體配置器。
+ Web/API：Axum、Tonic gRPC、Prost、utoipa 6（OpenAPI）、utoipa-swagger-ui。
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
│                    #   chip / config / dividend / events / financial / foreign_holding / health
│                    #   market_index / money_flow / performance / portfolio / quote / registry
│                    #   strategy / trace / yield_rank
├─ app/              # 應用層（scheduler / backfill / event / calculation / manual_backfill / ports）
├─ infra/            # 基礎設施（crawler / database / cache / nosql）
└─ interfaces/       # 對外介面（rpc / web：backfill_admin、data_api / bot）

benches/             # criterion 效能基準（twse_quote_parser / daily_quote_mapper / cache_snapshot_lookup）
etc/
├─ proto/            # gRPC proto（basic / control / manual_backfill / stock），build.rs 會產生 Rust stub 到 src/interfaces/rpc
└─ sql/              # PostgreSQL schema 與初始化 SQL（CI 依此建立測試資料庫）

docs/                # 架構文件（architecture.md）
log/                 # runtime 檔案日誌輸出目錄
```

+ 採 `core`、`domain`、`app`、`infra`、`interfaces` 分層，依賴方向由外向內（interfaces/infra → app → domain）；`domain/` 只放實體、倉儲 trait 與純計算，不做 I/O。
+ 分層規則與新模組放置規範詳見 [docs/architecture.md](docs/architecture.md)。

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
+ `RUSTDOCFLAGS="-D warnings -A rustdoc::private_intra_doc_links" cargo doc --no-deps --locked --document-private-items`：檢查文件連結（CI gate）；子模組的說明寫在模組檔的 `//!`，`mod x;` 上方不要再加 `///`（rustdoc 會把兩段合併並改用上層範圍解析連結）。
+ `cargo deny check advisories licenses bans`：檢查相依套件的安全通報、授權與禁用套件（CI gate）。
+ `cargo test --locked -- --test-threads=1`：只跑單元測試；需要 PostgreSQL／Redis 的測試會自動略過，不需外部服務。
+ `cargo test --locked --features integration-tests -- --test-threads=1`：連同整合測試一起跑。**請先在 `.env` 設定 `TEST_POSTGRESQL_*` 指向獨立的測試資料庫，不要對正式庫執行**；寫入資料的測試使用固定歷史日期與假代號並會自行清除。
+ CI 以 `cargo nextest run --release --locked --features integration-tests --test-threads=1 --profile ci` 對 PostgreSQL／Redis service container 執行整合測試，並以 `cargo llvm-cov` 產出覆蓋率（PR 需通過 codecov/patch）。
+ `cargo test --locked <測試名稱> -- --ignored --nocapture`：執行手動回補入口（見「手動回補入口」）。
+ `cargo bench`：執行 criterion 效能基準。

## 部署方式

+ `scripts\deploy-armv7.ps1` 會把 ARMv7 執行檔一次部署到 Raspberry Pi 3：本機 ELF 預檢（32-bit ARM
  hard-float、靜態連結）→ scp 到 `/tmp` 並比對 sha256 → ssh 執行 `control.sh update` → 驗證 `9001`／`9002`
  是否監聽與 `/api/v1/healthz` 是否 200。加 `-Build` 會先建 armv7；加 `-StageOnly` 只上傳不重啟服務；
  驗證失敗時會印出回滾指令。不會同步 `.env` 與 `app.json`（裝置上那兩份是手動維護的）。建議在收盤後部署，並避開 15:00 收盤排程。
+ 新增資料表時，部署前先在正式庫執行 `etc/sql/` 對應的建表 SQL，必要時再跑對應的手動回補入口。
+ `build.ps1` / `build.bat` 會以 `cargo zigbuild --release` 建置 `aarch64-unknown-linux-musl` 與 `armv7-unknown-linux-musleabihf` 兩個靜態連結 binary，輸出為 `stock_crawler_arm64`、`stock_crawler_armv7`。
+ `build.sh` 會以 `aarch64-unknown-linux-gnu` target 建置 release binary。
+ `control.sh start|stop|restart|update|move|build` 以本機模式啟停服務，依 `uname -m` 自動選用 `stock_crawler_arm64` 或 `stock_crawler_armv7`。
+ `control.sh docker_build|docker_start|docker_stop|docker_restart|docker_update` 會使用 `Dockerfile` 建立並啟停 Docker container。
+ `Dockerfile` 會依 BuildKit 的目標平台（`linux/arm64`、`linux/arm/v7`）選用對應 binary，連同 `.env`、`app.json` 複製到 `/app`，以 distroless static nonroot 映像執行（時區 `Asia/Taipei`），並 expose `9001`。
+ `control.sh docker_start` 預設建立 `stock-rust-container`，映射 `9001`、`9002`，並掛載 `log/` 與 SSL 憑證目錄。

## 排程時間

以下時間為台北時間（Asia/Taipei，排程固定以 UTC+8 解讀，與主機時區無關），依 `src/app/scheduler.rs` 為準。
每輪排程最長 3 小時，逾時會中止並以 Telegram 告警。

### 每天

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
+ 05:32 回補交易所公告的減資、變更面額與 ETF 分割（反分割）事件（排在 CAGR 之前，價格跳動才不會被算成漲跌）
+ 05:35 掃描交易所除權息公告，補齊資料庫漏抓的股利事件與除權息日、現金股利發放日
+ 05:37 以交易所除權除息計算結果核對近 45 天的股利日期與金額；資料庫缺的事件先從 Yahoo 補，Yahoo 也沒有的 ETF 現金配息（除息已滿 30 天）改用交易所日期與金額補登
+ 05:40 計算各期間年化報酬率（CAGR）
+ 06:00 採集三大財務報表（損益表、資產負債表、現金流量表）
  - 每輪最多 400 檔，每檔約每週重抓一次（7 天 Redis 旗標）
  - 官方季報 30 天內新公布、損益表還沒有那一季的股票優先，不受每週輪替限制；Yahoo 還沒更新時隔天再試
  - Yahoo 失敗（404 除外）時改從 BigGo 財經備援，以 `source = 'biggo'` 寫入
+ 07:10 以三大財務報表重算全市場財務體質（`financial_health`：年度 Piotroski F 分數、最新一季毛利率與負債比、近四季自由現金流）；持股的 F 分數有新年報、同年度下滑 2 分以上或落到 3 分以下時通知
+ 08:00 提醒本日與次一交易日除權息的股票；月配、季配、半年配者另列年化殖利率（單次殖利率 × 一年配息次數）
+ 08:02 提醒本日自持股票發放股利
+ 08:04 提醒本日開始公開申購的股票
+ 09:00 更新股票權值佔比
+ 09:02 啟動股票追蹤高低標提醒任務（服務在開盤期間重啟時會先補啟動一次）
+ 15:00 收盤：取得台股收盤報價（除權息股當天的漲跌以交易所開盤競價基準重算）並計算預估價格，接著依序：
  - 為上市櫃有成交的股票計算當日技術訊號（均線／KD／MACD 交叉、站上或跌破均線、超買超賣、52 週新高新低）
  - 回填 5／20／60／120／240 個交易日前的還原報酬（`daily_forward_return`）
  - 寫入當天上市櫃普通股（不含 ETF、特別股）的相對強度分數與名次（`daily_relative_strength`），分數向上或向下穿越 80、50 分的股票另寫成技術訊號（來源 `rs80`、`rs50`）
+ 20:30 通知持股近 7 天內法說會的摘要、展望與 Q&A 重點（BigGo 財經整理；摘要尚未產生的場次之後再通知，每場只通知一次）
+ 21:00 更新尚無年度配息資料的股票
+ 22:00 更新外資持股比例與狀態（`qfii_history`）

### 週一至週五

+ 15:30 大盤趨勢變化通知：加權指數當天站上或跌破 60／240 日均線、均線多空排列改變，或上市櫃站上 60 日均線的股票比例跨過 30% 才發（附均線值、距離、之前在另一側的天數、當天的市場廣度與估值水位；官方本益比 17:30 才補齊，估值多半是前一個交易日）；休市日不發
+ 17:00、19:00 檢查當日收盤資料：上市或上櫃有成交的檔數低於近期交易日的一半時重跑收盤匯總並告警；行情完整後再檢查當天的技術訊號與 5 個交易日前的未來報酬，沒寫入就只重算缺的那一步並告警；大盤指數沒寫入也會重抓
+ 17:20 持股填息通知：持股（含 ETF）近一年內的除權息，收盤今天第一次回到除權息前一日收盤的才發（附股利、前一日與今日收盤、第幾個交易日填息；規則同 `/dividend-fill`；休市日不發）
+ 17:30、20:30 以證交所 BWIBBU_d、櫃買中心 peQryDate 的官方值補最近 7 天日線為 0 的本益比與股價淨值比（只填 0、不覆寫；上櫃本益比以官方值為準覆寫；櫃買當天資料 17:30 常未公布，20:30 再補一次），補值後重算同 7 天的市場估值水位（`daily_market_valuation`）
+ 17:30 ETF 專區：證交所 MIS 預估淨值（已發行單位、單位增減、預估淨值、折溢價；前一營業日正式淨值回填）寫入 `etf_daily_nav`，沒有基本資料的 ETF 以代號尾碼與名稱補上推斷的類型、地區、題材，再以還原日線重算含息報酬（1 週～10 年）與近 1 年波動、最大回撤、對 0050 的 beta，並以正式淨值（現金配息還原，分割前的配息換算成現行單位）算淨值含息報酬（`etf_performance`，`nav_ret_*`），最後重建 stock_go `/stock/etfs` 讀的清單快照（`etf_snapshot`）；淨值抓取失敗時其餘步驟照做並告警
+ 17:45 持有 ETF 折溢價與申贖通知：今天剛轉為溢價 ≥ 2%，或剛好連續 5 天折價 ≤ -1% 的持有 ETF（持續溢價或折價不重發）；單日受益單位增減達前一營業日單位數 ±3% 的大額申購或贖回（附約略金額）
+ 19:15 通知持股（含 ETF）的重要技術訊號：52 週新高新低、收盤站上或跌破季線與年線、20 日與 60 日均線交叉、相對強度分數突破或跌破 80、50 分（附名次）；每個訊號附近一年全市場同一訊號出現後 20 個交易日的勝率與平均還原報酬（樣本 30 次以上）
+ 20:40 通知持股主力進出：主力（買超、賣超前 15 名分點）買賣超佔成交量達 20% 且達 100 張的持股，附買超、賣超前 3 名分點與近 5 個交易日的主力累計（富邦證券主力進出頁；休市日不發）；抓到的當日資料（不論是否達門檻）寫入 `broker_flow`
+ 21:40 通知持股三大法人與融資融券的明顯動作：外資買賣超佔成交量 30% 且 1,000 張以上、投信 15% 且 200 張以上、外資連 5 日或投信連 3 日同向買賣超、融資餘額增減 10% 且 500 張以上、融券 50% 且 200 張以上
+ 21:50 全市場三大法人買賣超與融資融券餘額寫入 `chip_daily`（證交所 T86／MI_MARGN、櫃買三大法人與融資融券），接著更新每檔外資／投信／自營商／合計的連續買賣超天數（存在同一列，排行、選股、回測與報酬統計直接讀取）
+ 22:05 重建 ETF 專區快照，帶進 21:50 入庫的外資買賣超
+ 22:10 策略換股通知：對每個 `alert` 為 true 的具名策略（`strategy_preset`）各跑一次回測，最後一次換股是今天時通知新買進、續抱、賣出的股票與權重、換手與 0050 核心部位，並對照目前實際持股；換股日另把持股寫進 `strategy_rebalance`，通知附上上一期實盤報酬與追蹤以來的累積及超額報酬；策略持股有基本面警示時另列。`.env` 的 `STRATEGY_ALERT_QUERY` 仍可另設一組
+ 23:00 檢查盤後資料：大盤指數、`chip_daily`、`qfii_history` 當天筆數為 0 或低於近期交易日的一半時重跑對應排程並告警；另重跑一次官方本益比補寫，有補到資料才告警

### 每月 10–28 日

+ 20:45 通知持股董監質押變動：與前一期比較設質、關係人設質增減與持股大幅增減（交易所董監事持股開放資料，約每月 18 日出表；第一次執行列出目前有設質的人作為基準）
+ 20:50 全市場董監事持股與設質寫入 `insider_holding`（開放資料只有最新月份，同月份重跑只更新有變的列、移除已不在名單的人）

### 週六

+ 10:20 每週市場摘要：
  - 大盤趨勢：加權指數收盤、近 5 日漲跌、均線排列、相對 60／240 日均線的位置與方向、這週穿越 20／60／240 日均線的日子、市場廣度與一週前比較、這週 52 週新高／新低家次
  - 估值水位：上市櫃本益比、股價淨值比中位數與在近 5 年的百分位
  - 產業相對強度（成分股 RS 中位數）最強、最弱與一週轉強轉弱的產業；近 5 個交易日相對強度站上、跌破 80 與 50 分的家數與代表股
  - 外資與投信近 5 個交易日在各產業的估計淨買賣金額（流入、流出最多）
  - 持股 RS 一週上升、下降最多的幾檔；外資、投信與主力近 5 個交易日在持股的累計買賣超（張數、佔成交量，法人附連續天數、主力附買超與賣超天數，買超、賣超各列最多 3 檔）；持股（含 ETF）近 7 天填息的除權息與尚未填息、離填息價最遠的 3 檔；持股基本面警示（營收連續 3 個月以上年減、三率三降、營業虧損、近四季自由現金流為負，最多列 10 檔）
  - ETF 資金流向：近 5 個交易日淨申購（受益單位增減 × 淨值）流入、流出最多的各 5 檔，持有 ETF 的淨申購與折溢價週平均、最新值；持有 ETF 這週收到新一期前十大持股時，列出與前一期相比的新進、剔除與權重變化最大的 3 檔
  - 有啟用通知的策略：這一期的換股日、本期至今報酬與 0050 對照、表現最好與最差的持股
+ 10:30 持股千張大戶週報：每檔持股的千張大戶持股比例、大戶人數與股東人數及週變化（集保戶股權分散表，第一次只列現況）
+ 10:40 全市場集保股權分散週摘要（千張大戶人數、比例、股東總數）寫入 `holder_distribution`（開放資料只有最新一週，無法回補）
+ 10:50 持股估值週報：本益比或股價淨值比位於自己近 5 年 ≥ 第 90 百分位（偏高）或 ≤ 第 10 百分位（偏低）的持股，附目前值與中位數（有效樣本不足 250 天或報價過期的不列入；沒有就不發）
+ 11:00 資料健康週報：檢查近 7 天的
  - 交易日完整性（整天缺漏、單一市場成交檔數偏少）、均線是否漏算、漲跌幅是否與參考價一致、年月日欄位
  - 衍生資料（估價、殖利率排行、市場統計、CAGR、外資持股、技術訊號、相對強度、三大法人與融資融券、持股主力進出、未來報酬、市場估值水位）是否更新到最新交易日
  - 月營收是否到位、過期的股利佔位列與年度合計不符
  - 本益比／股價淨值比為 0 的比例（淨值比超過 20%、本益比超過 60% 代表官方補值沒跑）
  - 未來報酬「長天期有值、短天期卻是空的」的基準日、籌碼連續買賣超天數是否漏算
  - 官方季報公布滿 7 天、單季損益表仍沒有那一季的上市櫃股票（財報落後）
  - 千張大戶超過 9 天沒更新、董監持股的最新月份早於 55 天前那個月（兩者來源都只有最新一期，停更就補不回來）
  - 全部正常也會發一則簡短週報
+ 12:10 ETF 基本資料與持股：逐檔抓 Yahoo ETF 頁（全名、發行投信、追蹤指數、費率、配息頻率、投資地區、淨值報酬、前十大持股、行業比重、資產分佈），寫入 `etf_profile`、`etf_holding`、`etf_allocation` 後重建快照（約 15 分鐘；分類可用 `etf_profile.manual_override` 人工覆寫）
+ 13:05 補 ETF 淨值缺日：抓 MoneyDJ 近 30 天的正式淨值與市價，補上 MIS 排程失敗那天的資料列與還沒回填的正式淨值（MIS 寫入的市價與預估折溢價不動），再重建快照（約 15 分鐘）

> DDNS IP 自動更新功能已自本專案移除，相關功能請改用 https://github.com/jiansoft/dynip。

## 衍生資料表

由排程預先計算、供 Data API 與通知直接讀取的表；歷史資料以「手動回補入口」補齊。

| 資料表 | 內容 | 寫入時機 | 歷史回補 |
|---|---|---|---|
| `daily_technical_signal` | 每日技術訊號（含相對強度穿越 `rs80`／`rs50`） | 15:00 收盤後 | `test_backfill_technical_signals` |
| `daily_forward_return` | 訊號基準日之後 5～240 個交易日的還原報酬 | 15:00 收盤後 | `test_backfill_forward_returns` |
| `daily_relative_strength` | 上市櫃普通股相對強度分數（1–99）與名次 | 15:00 收盤後 | `test_backfill_relative_strength` |
| `daily_stock_price_stats` | 市場廣度（漲跌家數、均線上下、52 週新高／新低） | 15:00 收盤後 | `test_backfill_market_breadth` |
| `daily_market_valuation` | 上市、上櫃與合計的本益比、股價淨值比中位數 | 17:30、20:30 | `test_backfill_market_valuation`（2012 起） |
| `chip_daily` | 三大法人買賣超、融資融券餘額與連續天數 | 21:50 | `test_backfill_chip_daily`、`test_backfill_chip_streaks` |
| `broker_flow` | 持股的主力進出（買賣超前幾名分點） | 20:40 | 無法回補（來源只有當天） |
| `holder_distribution` | 集保千張大戶週摘要 | 週六 10:40 | 無法回補（來源只有最新一週） |
| `insider_holding` | 董監事持股與設質 | 每月 10–28 日 20:50 | 無法回補（來源只有最新月份） |
| `financial_health` | F 分數、毛利率、負債比、近四季自由現金流 | 07:10 | `test_rebuild_financial_health` |
| `qfii_history` | 外資持股比例 | 22:00 | `test_backfill_foreign_holding_history` |
| `strategy_rebalance` | 具名策略每次換股的持股紀錄 | 22:10（換股日） | `test_record_strategy_rebalance` |
| `etf_daily_nav` | ETF 預估淨值、折溢價、已發行單位與單位增減 | 17:30、週六 13:05 補缺日 | `test_backfill_etf_nav_history`：MoneyDJ 近 3 年正式淨值與市價（折溢價由兩者算出、單位數留空），只補沒有的日期 |
| `etf_performance` | ETF 市價含息報酬、淨值含息報酬（`nav_ret_*`）與近 1 年風險指標 | 17:30 | `test_backfill_etf_daily` |
| `etf_profile`／`etf_holding`／`etf_allocation` | ETF 基本資料與分類、前十大持股、行業比重與資產分佈 | 週六 12:10 | `test_backfill_etf_profiles` |
| `etf_holding_history` | ETF 前十大持股的每一期（`recorded_at` 為第一次收到的時間） | 週六 12:10 | 隨 `test_backfill_etf_profiles` 寫入；建表時由 `etf_holding` 帶入目前一期 |
| `etf_snapshot` | stock_go ETF 專區清單頁的預先彙整（每檔一列） | 17:30、22:05、週六 12:10／13:05 | `test_refresh_etf_snapshot` |

技術訊號、未來報酬、相對強度與籌碼從 2019 年起；2022-02-17 以前沒有估價，市場廣度只有均線上下、漲跌家數與 52 週新高／新低。

## 手動回補入口

`src/app/manual_backfill/` 內以 `#[ignore]` 標記的測試，依資料類別分成子模組；測試名稱不重複，只用名稱篩選即可，例如
`cargo test --locked test_backfill_daily_quotes_for_date -- --ignored --nocapture`。參數與日期範圍寫在各測試的常數與說明中。

| 類別 | 入口 |
|---|---|
| quote（報價） | `test_backfill_daily_quotes_for_date`、`test_backfill_closing_aggregate_for_date`、`test_recalculate_moving_averages`、`test_backfill_taiwan_stock_index`、`test_backfill_quote_history_for_symbols`、`test_backfill_official_valuation_ratios`、`test_backfill_ex_right_changes` |
| dividend（股利） | `test_backfill_received_dividend_records_for_stock`、`test_backfill_historical_dividends_for_stock`、`test_backfill_historical_dividends_for_multiple_dividend_stocks`、`test_repair_stale_annual_total_dividends`、`test_reconcile_ex_right_results` |
| cagr | `test_backfill_cagr_for_date`、`test_backfill_cagr_period` |
| capital_reduction（減資） | `test_backfill_listed_capital_reductions`、`test_backfill_otc_capital_reductions`、`test_scan_otc_capital_reduction_gaps` |
| financial（財報） | `test_backfill_financial_reports_for_symbols`、`test_backfill_financial_reports_from_biggo_for_symbols`、`test_backfill_financial_reports_all`、`test_rebuild_financial_health` |
| chip（籌碼） | `test_backfill_chip_daily`、`test_backfill_chip_streaks`、`test_backfill_foreign_holding_history`、`test_save_holder_distributions`、`test_save_insider_holdings` |
| derived（衍生資料） | `test_backfill_technical_signals`、`test_backfill_forward_returns`、`test_backfill_relative_strength`、`test_backfill_market_breadth`、`test_backfill_market_valuation`、`test_record_strategy_rebalance` |

本機 `.env` 指向哪個資料庫，回補就寫到哪個資料庫；跑之前先確認。

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
16. 臺灣集中保管結算所 https://www.tdcc.com.tw（集保戶股權分散表開放資料）

+ `winvest`、`yuanta`、`bank_of_taiwan`（臺灣銀行）crawler module 仍在程式碼中，但目前沒有排程或介面使用；前兩者移出即時報價備援池的原因見「盤中即時報價與追蹤」。
+ `ipify`、`ipinfo`、`ipconfig`、`myip`、`seeip`、`bigdatacloud` 只用於啟動時查詢本機對外 IP。

## 主要設定

+ 所有設定可透過 `app.json` 提供，並可由 `.env` 或系統環境變數覆蓋。
+ `Cargo.toml` 的 package / binary 名稱目前仍為 `stock_crawler`；Seq 日誌服務識別則使用 `service=stock_rust`。
+ `app.json` 主要包含 `system`、`logging`、資料來源 API、PostgreSQL、Telegram、Redis 與外部 Go gRPC 連線設定。
+ `logging.seq.serverUrl` 與 `logging.seq.apiKey` 可提供預設值，正式值建議放在 `.env` 的 `SEQ_SERVER_URL`、`SEQ_API_KEY`。
+ 未設定 `SEQ_SERVER_URL` 時會停用 Seq 轉送；有設定時會以 CLEF 格式送到 Seq `/api/events/raw?clef`，只送出 `service=stock_rust` 作為服務識別。
+ `logging.file.maxSizeMb`／`logging.file.maxAgeDays` 設定檔案日誌的大小上限與保留天數（`0` 代表用預設值 10 MB／7 天；限制在 1 MB–1 GB、1–365 天）。使用中的日誌檔固定為 `log/YYYY-MM-DD_default_{level}.log`，超過大小時改名為帶最後一筆時間的 `…{level}.HH-MM-SS.log` 再開新檔。
+ 即時報價備援來源包含 Fugle 官方日內行情 API；若未設定 `FUGLE_API_KEY`，系統會略過 Fugle 並繼續嘗試其他來源。

## 對外介面

### gRPC

+ 依 `system.grpc_use_port` 啟動，註冊 `ControlService`、`ManualBackfillService`、`StockService` 三個服務；`system.ssl_cert_file` 與 `system.ssl_key_file` 都有設定時啟用 TLS。
+ `StockService`：`UpdateStockInfo`、`FetchCurrentStockQuotes`、`FetchHolidaySchedule`。
+ `ManualBackfillService`：每日報價、收盤彙總、台股加權指數、持股股利重算、單檔／多檔歷史股利、個股歷史報價、公司行動（減資等）登錄、CAGR 全期間／單一期間計算，以及 job 查詢。

### HTTP 手動回補與策略管理

+ 管理頁 `/manual-backfill`；API 包含 `/api/manual-backfill/jobs`、`/api/manual-backfill/jobs/{id}` 與 `POST /api/manual-backfill/*`（daily-quotes、closing-aggregate、taiwan-stock-index、received-dividend-records、historical-dividends、multiple-dividend-historical-dividends、quote-history、corporate-action、annual-total-repair、cagr、cagr-period）；工作在背景執行，請求會立即回傳 job id。
+ 具名策略的新增、修改與刪除走 `/api/strategy-presets`、`/api/strategy-presets/{name}`，存之前以回測驗證條件。

### Data API（`/api/v1/*`）

與手動回補共用 HTTP server。除 `/api/v1/healthz` 外都需要 `Authorization: Bearer <DATA_API_KEY>`（未設定 `DATA_API_KEY` 時一律拒絕）；
OpenAPI 文件在 `/api-docs/openapi.json`，Swagger UI 在 `/swagger-ui`（不需驗證），各參數的範圍與預設值以 Swagger 為準。

**個股**

| 端點 | 內容 |
|---|---|
| `/stocks/search` | 依代號或名稱搜尋股票 |
| `/stocks/{symbol}/profile` | 基本資料 |
| `/stocks/{symbol}/latest-quote`、`/realtime-snapshot` | 最新收盤報價、盤中即時快照 |
| `/stocks/{symbol}/price-history` | 歷史日線，可附依除權息、減資、分割還原的價格 |
| `/stocks/{symbol}/monthly-revenues` | 月營收 |
| `/stocks/{symbol}/financial-reports` | 季報與年報摘要（EPS、ROE 等） |
| `/stocks/{symbol}/financial-statements` | 三大財務報表（損益表／資產負債表／現金流量表） |
| `/stocks/{symbol}/financial-health` | 財務體質（Piotroski F 分數、毛利率、負債比、自由現金流） |
| `/stocks/{symbol}/dividends`、`/dividend-fill` | 股利；填息分析（除權息後幾個交易日回到除權息前收盤） |
| `/stocks/{symbol}/valuation`、`/valuation-band` | 估值；本益比／股價淨值比在近幾年的百分位區間 |
| `/stocks/{symbol}/peers` | 同產業比較（市值、本益比、股價淨值比、殖利率、F 分數、毛利率、營收年增率，含產業中位數與名次） |
| `/stocks/{symbol}/technicals` | 均線與乖離率、RSI、KD、MACD 與各類訊號（價格依除權息、減資、分割還原） |
| `/stocks/{symbol}/signals` | 個股技術訊號歷史，附每次訊號之後的報酬 |
| `/stocks/{symbol}/relative-strength` | 相對強度歷史（分數、名次、期間高低點與穿越 50／80 的轉折） |
| `/stocks/{symbol}/chip` | 籌碼面（三大法人、融資融券與每日主力買賣超、千張大戶、董監設質、最新一天的主力分點） |
| `/stocks/{symbol}/timeline` | 事件時間軸：除權息與股利發放、減資／分割、月營收與營收創新高、季報與三率三升／三降、技術訊號、法人連續買賣超里程碑與董監持股變化，另列已公告的未來除權息與減資 |

**市場**

| 端點 | 內容 |
|---|---|
| `/market/index-history` | 大盤指數歷史 |
| `/market/regime` | 大盤趨勢狀態：加權指數相對 5～240 日均線的位置與方向、均線排列、52 週高低點距離、近期漲跌與均線穿越，附市場廣度與估值水位（本益比、股價淨值比中位數與近 5 年百分位） |
| `/market/breadth` | 市場廣度（漲跌家數、均線上下、52 週新高／新低家數） |
| `/market/valuation` | 市場估值水位：上市櫃本益比與股價淨值比中位數近 1–10 年的分布、最新百分位與每月走勢 |
| `/etfs` | ETF 清單：依分類、地區、配息頻率、市場、關鍵字篩選，依成交值、規模、折溢價、殖利率、淨申購、含息報酬或受益人數排序 |
| `/etfs/{symbol}` | 單檔 ETF：概況、基本資料、市價與淨值含息報酬、風險指標、前十大持股、行業比重、資產分佈與近期每日折溢價 |
| `/etfs/compare` | 2 至 4 檔 ETF 並排比較：費率、規模、折溢價、殖利率、市價與淨值含息報酬、風險指標，以及兩兩前十大持股的重疊權重 |
| `/etfs/{symbol}/holdings/history` | ETF 近幾期（預設 6、最多 24）的前十大持股，每期附上與前一期相比的新進、剔除與權重變化（`etf_holding_history`，收到新一期才新增） |
| `/market/industries` | 產業表現（漲跌家數、漲跌幅中位數、市值加權漲跌幅、成交值占比、估值中位數與最大漲跌個股） |
| `/market/industry-strength` | 產業相對強度（成分股 RS 中位數、強弱勢占比、領先股與約 20 個交易日前比較） |
| `/market/relative-strength` | 相對強度排行（近 60／120／240 日還原報酬加權的百分位分數，可依市場、產業、代號篩選） |
| `/market/signals` | 技術訊號選股（某交易日出現指定訊號的上市櫃股票） |
| `/market/signals/performance` | 訊號績效統計：各訊號出現後 5～240 個交易日的還原報酬、勝率與超額報酬，可依相對強度區間或法人連續買超篩選，`group_by=year` 依年度拆分 |
| `/market/institutional-flow` | 三大法人買賣超排行（估計淨買賣金額、占成交量比例與連續天數） |
| `/market/institutional-streak/performance` | 法人連續買超或賣超滿 N 天之後的報酬統計 |
| `/market/industry-flow` | 產業法人資金流向 |
| `/market/margin-ranking` | 融資融券排行（增減最多、券資比最高） |
| `/market/movers` | 當日漲跌幅／成交量排行 |
| `/market/qfii-holding-ranking` | 外資持股排行 |
| `/market/dividend-calendar`、`/dividend-yield-ranking` | 股利行事曆；殖利率排行 |
| `/market/cagr-ranking`、`/cagr-ranking/{stock_symbol}` | CAGR 排行與個股 CAGR |

**選股與回測**

| 端點 | 內容 |
|---|---|
| `/stocks/screen` | 條件選股：F 分數、毛利率、負債比、自由現金流、營業利益率、三率三升、營收創新高、營收連續年增、相對強度、外資／投信連續買超、平均成交金額、本益比／股價淨值比上限，可排除產業 |
| `/stocks/screen/backtest` | 條件選股回測（見下方說明） |
| `/stocks/screen/backtest/offsets` | 換股日偏移穩健度：同一組條件把換股日偏移 0 到 `hold_days` − 1 都跑一遍，回傳各指標的分布與勝過 0050 的比例；`split` 分樣本內外 |
| `/stocks/screen/backtest/frontier` | 0050 核心部位前緣：0%～100%（每 10%）各比例的偏移分布，標出夏普值、年化報酬中位數最高的比例 |
| `/stocks/screen/backtest/walk-forward` | 滾動前推最佳化：每年只用之前 `train_years` 年挑參數（36 組），串起樣本外年份並與不調參數及 0050 比較 |
| `/stocks/screen/backtest/attribution` | 報酬歸因（Brinson-Fachler）：相對類 0050 基準拆成產業配置、選股與交互效果 |
| `/strategy-presets`、`/strategy-presets/{name}/track` | 具名策略清單；依 `strategy_rebalance` 逐期追蹤實際表現並與 0050 比較 |

回測每 5／20／60／120／240 個交易日（`hold_days`）從上市櫃普通股挑出前 `limit` 檔持有，只用換股當天已知的資料（月營收、季報依法定公布期限），並依換手率扣除手續費（`fee_discount` 折扣）與證交稅。回傳每期報酬與持股、累積與年化報酬、最大回撤、勝率、年化波動率與夏普值、同期母體平均與 0050 比較、依年度彙總、個股貢獻與目前持股。回測、偏移、前緣、歸因與滾動前推都可用 `preset=名稱` 引用具名策略，另帶的參數覆蓋存的值。主要參數：

+ 選股條件：`market`、`industry`、`exclude_industry`、`min_rs_score`／`max_rs_score`、`min_foreign_streak`、`min_trust_streak`、`indicator`／`kind`（近期技術訊號）
+ 基本面門檻（只用於新買進）：`min_revenue_yoy_percent`、`min_revenue_yoy_streak`、`min_revenue_yoy_3m_percent`、`min_monthly_revenue`、`revenue_record`、`min_eps`、`min_roe_percent`、`min_gross_margin_percent`、`min_operating_margin_percent`、`margins_yoy_up`、`max_debt_ratio_percent`、`positive_free_cash_flow`、`max_pe`、`max_pb`
+ 排序與持股：`rank_by`（相對強度、平均成交金額、法人連續買超、營收年增率、營業利益率、本益比、股價淨值比）、`keep_rank`（換股緩衝）、`hold_min_rs_score`（續抱條件）、`max_per_industry`、`min_avg_trade_value`（流動性門檻）
+ 權重：`weighting`（等權、成交金額或其平方根加權）、`max_weight`（單檔上限）、`etf_weight`（0050 核心部位）
+ 大盤濾網（換股日不符就空手）：`market_filter_ma`（加權指數均線）、`market_filter_breadth`（站上 60 日均線比例）、`market_filter_max_pe`（市場本益比中位數）、`market_filter_pe_percentile`（市場本益比中位數在換股日前近 5 年的百分位，只用當天以前的資料）
+ 風控與檢驗：`stop_loss`、`trailing_stop`（期間停損／移動停利）、`rebalance_offset`（換股日偏移）、`split`（樣本內外分割日）

**持股組合**（屬個人持股資料，只有持有 `DATA_API_KEY` 者能查詢；`member_id=0`（預設）為全部會員合計）

| 端點 | 內容 |
|---|---|
| `/portfolio/summary` | 持股總覽：市值、權重、未實現損益、已領股利、產業分布、市值加權殖利率與 F 分數、組合本益比與股價淨值比；每檔附估值、相對強度、法人連續天數、月營收與三率、近四季自由現金流與基本面警示 |
| `/portfolio/history` | 近 1～120 個月的時間加權報酬、月報酬、最大回撤與加權指數比較，以及 beta、年化 alpha、相關係數、追蹤誤差與年化波動率 |
| `/portfolio/dividend-forecast` | 未來 1～24 個月每月的預估現金股利入帳與預估殖利率 |
| `/portfolio/chip` | 持股籌碼日報：某交易日收盤時仍持有的普通股（依持股批次與賣出紀錄推回）的三大法人買賣超（張、佔成交量、連續天數）、融資融券增減與主力進出（另附近 5 個主力交易日的累計），並以與 Telegram 通知相同的門檻標出達標原因 |
| `/portfolio/dividend-fill` | 持股填息狀態：目前持股（含 ETF）近 `days` 天（預設 365）每次除權息的填息結果，尚未填息的依距離填息價的缺口由大到小排在前面，附整體填息統計（規則同 `/dividend-fill`） |

### Telegram 通知

排程提醒（除權息、股利發放、公開申購、持股月營收（標示創歷史新高與連續 3 個月以上年減）、持股財報（EPS、ROE 與三率變化，標示三率三升／三降）、持股財務體質、外資持股變化、持股法說會摘要、持股主力進出、持股填息、持股董監質押變動、持股法人與信用交易、持股技術訊號、大盤趨勢變化、策略換股）、週報（每週市場摘要、持股千張大戶、持股估值、資料健康）、價格追蹤通知與錯誤告警。各通知的時間與門檻見「排程時間」。

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
+ `FILE_LOG_LEVEL`：檔案日誌等級過濾，預設 `info,html5ever=off,rustls::msgs::handshake=error,tokio_cron_scheduler=warn`。
+ `FUGLE_API_KEY`：Fugle 日內行情 API 金鑰（即時報價備援）。
+ `TELEGRAM_TOKEN`、`TELEGRAM_ALLOWED`：Telegram Bot 與允許通知的 chat 設定。
+ `POSTGRESQL_HOST`、`POSTGRESQL_PORT`、`POSTGRESQL_USER`、`POSTGRESQL_PASSWORD`、`POSTGRESQL_DB`：PostgreSQL 連線設定。
+ `TEST_POSTGRESQL_HOST`、`TEST_POSTGRESQL_PORT`、`TEST_POSTGRESQL_USER`、`TEST_POSTGRESQL_PASSWORD`、`TEST_POSTGRESQL_DB`：只在測試建置生效，覆蓋 `POSTGRESQL_*`，讓整合測試連到獨立測試庫。
+ `REDIS_ADDR`、`REDIS_ACCOUNT`、`REDIS_PASSWORD`、`REDIS_DB`、`REDIS_POOL_SIZE`：Redis 連線設定。
+ `SYSTEM_GRPC_USE_PORT`、`SYSTEM_SSL_CERT_FILE`、`SYSTEM_SSL_KEY_FILE`：本服務 gRPC 與 TLS 憑證設定。
+ `MANUAL_BACKFILL_WEB_ADDR`：HTTP server（手動回補與 Data API）監聽位址，預設 `127.0.0.1:9002`。
+ `DATA_API_KEY`：Data API 的 Bearer 金鑰；未設定時 Data API 拒絕所有受保護請求。
+ `STRATEGY_ALERT_QUERY`、`STRATEGY_ALERT_NAME`：（相容用，建議改用具名策略的 `alert` 旗標）另一組策略換股通知的回測條件（與 `/api/v1/stocks/screen/backtest` 相同的 query string，必須含 `from`；起日到今天不可超過 3,653 天）與通知標題中的名稱；與具名策略條件相同時不重複通知。
+ `GO_GRPC_TARGET`、`GO_GRPC_TLS_CERT_FILE`、`GO_GRPC_TLS_KEY_FILE`、`GO_GRPC_DOMAIN_NAME`：對外 Go gRPC 服務連線設定。


### 免責聲明
本網提供之所有資訊內容均僅供參考，不涉及買賣投資之依據。使用者在進行投資決策時，務必自行審慎評估，
並自負投資風險及盈虧，如依本網提供之資料交易致生損失，本網不負擔任何賠償及法律責任。您自行負責依據
自身投資目標及個人、財務狀況，確定任何投資、證券或任何其他投資產品服務是否適合自身的需要。
本網站所載或本網站上、通過本網站提供的任何服務、內容、資訊及/或資料在任何情況下均不得被解釋為提供投資、
法律意見或提供投資服務。特請訪問此類網頁的人士就有關任何本網資料是否適合其投資需求徵詢適當獨立專業意見。
