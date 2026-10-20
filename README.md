
### Taiwan stock crawler

台股資料採集、排程更新、手動回補、價格追蹤提醒、唯讀 Data API 與 gRPC/HTTP 管理介面服務。

UI Demo︰https://jiansoft.mooo.com/stock/revenues
API︰https://github.com/jiansoft/stock_api

## 專案功能與用途

+ 依排程採集台股主檔、營收、財報（含損益表／資產負債表／現金流量表三大報表）、股利、減資、法人資金流向、ETF、收盤報價、指數與外資持股等資料。
+ 計算預估價格（便宜／合理／昂貴價）、移動平均、各期間年化報酬率（CAGR）與市場廣度統計（2019 起；2022-02-17 以前沒有估價，只有均線上下、漲跌家數與 52 週新高／新低）。
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
+ `RUSTDOCFLAGS="-D warnings -A rustdoc::private_intra_doc_links" cargo doc --no-deps --locked --document-private-items`：檢查文件連結（CI gate）；子模組的說明寫在模組檔的 `//!`，`mod x;` 上方不要再加 `///`（rustdoc 會把兩段合併並改用上層範圍解析連結）。
+ `cargo deny check advisories licenses bans`：檢查相依套件的安全通報、授權與禁用套件（CI gate）。
+ `cargo test --locked -- --test-threads=1`：只跑單元測試；需要 PostgreSQL／Redis 的測試會自動略過，不需外部服務。
+ `cargo test --locked --features integration-tests -- --test-threads=1`：連同整合測試一起跑。**請先在 `.env` 設定 `TEST_POSTGRESQL_*` 指向獨立的測試資料庫，不要對正式庫執行**；寫入資料的測試使用固定歷史日期與假代號並會自行清除。
+ CI 以 `cargo nextest run --release --locked --features integration-tests --test-threads=1 --profile ci` 對 PostgreSQL／Redis service container 執行整合測試，並以 `cargo llvm-cov` 產出覆蓋率。
+ `cargo test --locked <測試名稱> -- --ignored --nocapture`：手動回補入口（`src/app/manual_backfill/` 內以 `#[ignore]` 標記的測試，依資料類別分成 quote、dividend、cagr、capital_reduction、financial、chip、derived 子模組；測試名稱不重複，只用名稱篩選即可）。
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
+ 05:32 回補交易所公告的減資、變更面額與 ETF 分割（反分割）事件
+ 05:35 掃描交易所除權息公告，補齊資料庫漏抓的股利事件與除權息日、現金股利發放日
+ 05:37 以交易所除權除息計算結果核對近 45 天的股利日期與金額；資料庫缺的事件先從 Yahoo 補，Yahoo 也沒有的 ETF 現金配息（除息已滿 30 天）改用交易所日期與金額補登
+ 05:40 計算各期間年化報酬率（CAGR）
+ 06:00 採集三大財務報表（損益表、資產負債表、現金流量表），每輪最多 400 檔、每檔約每週重抓一次；官方季報 30 天內新公布、損益表還沒有那一季的股票優先處理，不受每週輪替限制，Yahoo 還沒更新時隔天再試；Yahoo 失敗（404 除外）時改從 BigGo 財經備援，以 `source = 'biggo'` 寫入
+ 07:10 以三大財務報表重算全市場財務體質（年度 Piotroski F 分數、最新一季毛利率與負債比、近四季自由現金流），供條件選股篩選；持股的 F 分數有新年報、同年度下滑 2 分以上或落到 3 分以下時發 Telegram 通知
+ 08:00 提醒本日與次一交易日除權息的股票（需自行架設本服務）；月配、季配、半年配者另列年化殖利率（單次殖利率 × 一年配息次數）
+ 08:02 提醒本日自持股票發放股利（需自行架設本服務）
+ 08:04 提醒本日開始公開申購的股票（需自行架設本服務）
+ 09:00 更新股票權值佔比
+ 09:02 啟動股票追蹤高低標提醒任務
+ 15:00 取得台股收盤報價數據（除權息股當天的漲跌以交易所開盤競價基準重算）並計算預估價格，並為上市櫃有成交的股票預先計算當日技術訊號（均線／KD／MACD 交叉、站上或跌破均線、超買超賣、52 週新高新低），並回填 5／20／60／120／240 個交易日前的還原報酬（供訊號績效統計與相對強度排行），再寫入當天上市櫃普通股（不含 ETF、特別股）的相對強度分數與名次（`daily_relative_strength`，個股相對強度歷史讀這張表；歷史可用 `test_backfill_relative_strength` 回填），並把分數比前一個交易日向上或向下穿越 80、50 分的股票寫成技術訊號（來源 `rs80`、`rs50`，可用於訊號選股與訊號績效）
+ 15:30（週一至週五）大盤趨勢變化通知：加權指數當天站上或跌破 60／240 日均線、、均線多空排列改變、或上市櫃站上 60 日均線的股票比例跨過 30%（回測市場廣度濾網的門檻）才發（附均線值、距離、之前在另一側的天數與當天的市場廣度）；休市日不發
+ 17:00、19:00（週一至週五）檢查當日收盤資料：上市或上櫃有成交的檔數低於近期交易日的一半時重跑收盤匯總並以 Telegram 告警（只有一個市場有資料時收盤匯總也會拒絕寫入）；行情完整後再檢查當天的技術訊號與 5 個交易日前的未來報酬，沒寫入就只重算缺的那一步並告警；同時段寫入的大盤指數沒寫入也會重抓
+ 19:15（週一至週五）通知持股（含 ETF）的重要技術訊號：52 週新高新低、收盤站上或跌破季線與年線、20 日與 60 日均線交叉、相對強度分數比前一個交易日突破或跌破 80、50 分（附名次），每個訊號附近一年全市場同一訊號出現後 20 個交易日的勝率與平均還原報酬（樣本 30 次以上）
+ 17:30、20:30（週一至週五）以證交所 BWIBBU_d、櫃買中心 peQryDate 的官方值補最近 7 天日線為 0 的本益比與股價淨值比（只填 0、不覆寫；上櫃本益比例外，以官方值為準覆寫；櫃買當天資料 17:30 常未公布，20:30 再補一次）；歷史缺值另以手動回補 `test_backfill_official_valuation_ratios` 逐日補齊；補值後重算同 7 天上市、上櫃與合計的本益比、股價淨值比中位數（`daily_market_valuation`，歷史以 `test_backfill_market_valuation` 自 2012 年起回補）
+ 20:30 通知持股近 7 天內法說會的摘要、展望與 Q&A 重點（BigGo 財經整理；摘要尚未產生的場次之後再通知，每場只通知一次）
+ 20:40（週一至週五）通知持股主力進出：主力（買超、賣超前 15 名分點）買賣超佔成交量達 20% 且達 100 張的持股，附買超、賣超前 3 名分點（富邦證券主力進出頁；休市日不發）
  - 抓到的當日主力進出（不論是否達門檻）同時寫入 `broker_flow`（只有持股；頁面只有最近一天，無法回補）
+ 20:45（每月 10–28 日）通知持股董監質押變動：與前一期比較設質、關係人設質增減與持股大幅增減（交易所董監事持股開放資料，約每月 18 日出表；第一次執行列出目前有設質的人作為基準）
+ 20:50（每月 10–28 日）全市場董監事持股與設質寫入 `insider_holding`（開放資料只有最新月份，同月份重跑只更新有變的列、移除已不在名單的人）
+ 21:00 更新尚無年度配息資料的股票
+ 21:40（週一至週五）通知持股三大法人與融資融券的明顯動作：外資買賣超佔成交量 30% 且 1,000 張以上、投信 15% 且 200 張以上、外資連 5 日或投信連 3 日同向買賣超、融資餘額增減 10% 且 500 張以上、融券 50% 且 200 張以上（證交所 T86／MI_MARGN、櫃買三大法人與融資融券）
+ 21:50（週一至週五）全市場三大法人買賣超與融資融券餘額寫入 `chip_daily`；歷史可用 `test_backfill_chip_daily` 回補，入庫後接著更新每檔外資／投信／自營商／合計的連續買賣超天數（存在同一列，排行、選股、回測與報酬統計直接讀取，不限回看長度；可用 `test_backfill_chip_streaks` 整段重算）
+ 22:10（週一至週五）策略換股通知：以 `STRATEGY_ALERT_QUERY` 設定的條件選股回測條件跑一次回測，最後一次換股是今天時通知新買進、續抱、賣出的股票與權重、換手與 0050 核心部位，並對照目前實際持股（策略持股中已持有／尚未持有、策略賣出但仍持有、是否持有 0050）；沒有設定就不做事，條件用到外資／投信連續買超而當天籌碼還沒入庫時告警。換股日另把當天通知的持股寫進 `strategy_rebalance`，`/api/v1/strategy-presets/{name}/track` 依這些紀錄逐期計算實際表現（未扣成本）並與 0050 比較，換股通知也附上依紀錄結算的上一期實盤報酬（組合、策略部位、0050）與追蹤以來的累積及超額報酬；啟用通知前的那一期可用 `test_record_strategy_rebalance` 補記
+ 10:20（週六）每週市場摘要：大盤趨勢（加權指數收盤、近 5 日漲跌、均線排列、相對 60／240 日均線的位置與方向，這週穿越 20／60／240 日均線的日子，以及市場廣度與一週前比較和這週 52 週新高／新低家次，與上市櫃本益比、股價淨值比中位數在近 5 年的百分位）、產業相對強度（成分股 RS 中位數）最強、最弱與一週轉強轉弱的產業，近 5 個交易日相對強度站上、跌破 80 與 50 分的家數與代表股，外資與投信近 5 個交易日在各產業的估計淨買賣金額（流入、流出最多），以及持股 RS 一週上升、下降最多的幾檔、持股基本面警示（營收連續 3 個月以上年減、三率三降、營業虧損、近四季自由現金流為負，最多列 10 檔）；有設定 `STRATEGY_ALERT_QUERY` 時另附策略這一期的換股日、本期至今報酬與 0050 對照、表現最好與最差的持股
+ 10:30（週六）持股千張大戶週報：列出每檔持股的千張大戶持股比例、大戶人數與股東人數及週變化（集保戶股權分散表，第一次只列現況）
+ 10:40（週六）全市場集保股權分散週摘要（千張大戶人數、比例、股東總數）寫入 `holder_distribution`（開放資料只有最新一週，無法回補）
+ 10:50（週六）持股估值週報：列出本益比或股價淨值比位於自己近 5 年 ≥ 第 90 百分位（偏高）或 ≤ 第 10 百分位（偏低）的持股，附目前值與中位數（有效樣本不足 250 天或報價過期的不列入；沒有就不發）
+ 11:00（週六）資料健康週報：檢查近 7 天的交易日完整性（整天缺漏、單一市場成交檔數偏少）、均線是否漏算、漲跌幅是否與參考價一致、年月日欄位、衍生資料（估價、殖利率排行、市場統計、CAGR、外資持股、技術訊號、相對強度、三大法人與融資融券、未來報酬）是否更新到最新交易日、月營收是否到位、過期的股利佔位列與年度合計不符，以及本益比／股價淨值比為 0 的比例（淨值比超過 20%、本益比超過 60% 代表官方補值沒跑），以及未來報酬整張表中「長天期有值、短天期卻是空的」的基準日（回補範圍沒涵蓋到）、籌碼連續買賣超天數是否漏算，以及官方季報公布滿 7 天、Yahoo／BigGo 單季損益表仍沒有那一季的上市櫃股票（財報落後）；全部正常也會發一則簡短週報
+ 22:00 更新外資持股比例與狀態
+ 23:00（週一至週五）檢查盤後資料：大盤指數、三大法人與融資融券（`chip_daily`）、外資持股（`qfii_history`）當天筆數為 0 或低於近期交易日的一半時重跑對應排程並以 Telegram 告警（例如資料庫停機讓當天的排程全部失敗）；另重跑一次官方本益比補寫，有補到資料才告警
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
16. 臺灣集中保管結算所 https://www.tdcc.com.tw（集保戶股權分散表開放資料）

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
+ Data API 位於 `/api/v1/*`（與手動回補共用 HTTP server），提供股票搜尋、最新與歷史報價（歷史日線可附依除權息、減資、分割還原的價格）、即時快照、基本資料、月營收、財報、三大財務報表（損益表／資產負債表／現金流量表）、財務體質評分（Piotroski F 分數）、股利、填息分析（除權息後幾個交易日回到除權息前收盤）、估值、估值歷史區間（本益比／股價淨值比在近幾年的百分位）、同業比較（同產業個股的市值、本益比、股價淨值比、殖利率、F 分數、毛利率、營收年增率，含產業中位數與名次）、技術指標（5／10／20／60／120／240 日均線與乖離率、RSI、KD、MACD，以及均線／KD／MACD 交叉、站上或跌破均線、超買超賣與 52 週新高新低訊號；價格依除權息、減資、分割還原權值）、技術訊號選股（某交易日出現指定訊號的上市櫃股票，可依市場、產業篩選並依成交值排序）、訊號績效統計（各技術訊號出現後 5／20／60／120／240 個交易日的還原報酬平均、中位數、勝率與相對大盤的超額報酬，可只看訊號當天相對強度分數在指定區間、或外資／投信已連續買超 N 天的訊號，`group_by=year` 另附各年度的統計與大盤基準，用來看訊號在多頭、空頭年份是否穩定；訊號、報酬與相對強度從 2019 年起，期間最長 3,653 天，超過 1,096 天需指定訊號來源）、個股訊號歷史（附每次訊號之後的報酬）、個股事件時間軸（`/stocks/{symbol}/timeline`：除權息與股利發放、減資／分割、月營收與營收創歷史新高、季報與三率三升／三降（金融保險業不列）、技術訊號、法人連續買賣超里程碑與董監持股變化依日期合併，另列迄日之後已公告的除權息與減資）、相對強度排行（近 60／120／240 日還原報酬加權後在上市櫃普通股（不含 ETF、ETN、受益證券與特別股）中的百分位分數 1–99，可依市場、產業、代號篩選）、個股相對強度歷史（近 N 個交易日的分數、名次、期間高低點與穿越 50／80 的轉折）、產業相對強度（各產業成分股 RS 中位數、強弱勢占比、領先股與約 20 個交易日前的比較，用來看資金輪動）、籌碼面、三大法人買賣超排行（外資／投信／自營商／合計近 N 個交易日的估計淨買賣金額、占成交量比例與連續買賣超天數，可依市場、產業篩選）、法人連續買賣超後報酬統計（外資／投信／自營商／合計連續買超或賣超滿 N 天之後 5／20／60／120／240 個交易日的還原報酬平均、中位數、勝率與相對大盤的超額報酬，同樣可依年度拆分；籌碼資料從 2019 年起）、產業法人資金流向（各產業近 N 個交易日的法人淨買賣金額、買賣超家數、占產業成交金額比例與主要買賣超個股）、融資融券排行（近 N 個交易日融資或融券增減最多、券資比最高的股票，可依市場、產業篩選）、市場廣度（含 52 週新高／新低家數）、殖利率排行、選股（可依 F 分數、毛利率、負債比、自由現金流、營業利益率、三率三升（最新一季毛利率、營業利益率、稅後淨利率都高於去年同季）、最新月營收創歷史新高、連續營收年增月數、近 3 月累計營收年增率、相對強度分數、外資／投信連續買超天數、近 20 筆日線平均成交金額、官方本益比上限 `max_pe` 與股價淨值比上限 `max_pb` 篩選（可依本益比、股價淨值比排序），可用 `exclude_industry_ids` 排除產業（例如金融保險業 17），並輸出平均成交金額）、條件選股回測（每 5／20／60／120／240 個交易日依相對強度分數、外資／投信連續買超天數、近期技術訊號、市場與產業，從上市櫃普通股挑出相對強度最高的 N 檔等權持有，可設換股緩衝 `keep_rank`（上一期持股只要仍在前 N 名就續抱，降低換手），可設大盤濾網 `market_filter_ma`（換股日加權指數低於 20／60／120／240 日均線就空手），可設市場廣度濾網 `market_filter_breadth`（換股日站上 60 日均線的股票比例低於 N% 就空手，可與均線濾網併用），可設續抱條件 `hold_min_rs_score`（上一期持股不再符合選股條件時，相對強度仍達門檻就續抱），可設產業上限 `max_per_industry`（每期同一產業最多 N 檔，超過的由其他產業依相對強度補上），可設換股日偏移 `rebalance_offset`（起日之後先跳過幾個交易日才第一次換股，換不同偏移重跑可看結果是否只是碰巧遇上某些換股日），可設流動性門檻 `min_avg_trade_value`（換股日前 20 個交易日平均成交金額低於 N 百萬元的股票不新買進，排除實際買不到量的小型股），可設排序方式 `rank_by`（符合條件者改依近 20 個交易日平均成交金額、外資／投信連續買超天數、換股日已公布的最新月營收年增率、最近一季營業利益率，或換股當天官方本益比、股價淨值比由低到高排序取前 N 檔，預設相對強度），可設 0050 核心部位 `etf_weight`（組合固定以 N% 持有 0050、其餘跑策略，看策略搭配大盤的效果），可設持股加權 `weighting`（預設等權，可改依近 20 個交易日平均成交金額或其平方根加權，接近 0050 的市值加權、偏重大型股；目前持股附各檔權重），可設排除產業 `exclude_industry`（逗號分隔的產業名稱，例如排除金融保險業，大盤基準不受影響），可設單檔權重上限 `max_weight`（每檔最多占策略部位 N%，超出的分給其他持股，檔數不足時剩下的放現金，避免集中在少數大型股），可設期間停損 `stop_loss`／移動停利 `trailing_stop`（持有期間每天以還原收盤檢查，相對買進價或期間最高收盤跌幅達 N% 就當天收盤賣出、到下次換股前放現金，賣出成本計入，每期與彙總附停損檔數），回傳每期報酬與持股、累積與年化報酬、最大回撤、勝率、年化波動率與夏普值，與同期母體平均及實際買得到的 0050 比較，另附依年度的彙總與個股貢獻（整段報酬貢獻最多、拖累最多的前 10 檔）與目前持股（最後一次換股、還沒滿持有天數的持股、是否為新買進、這次賣出的股票、換股日收盤至今的還原報酬（各檔與依權重的策略部位，附 0050 同期）與距下次換股的交易日數，不計入彙總），並依換手率扣除手續費（可設折扣）與證交稅（續抱股票漲跌後調回目標權重、0050 核心部位每期調回固定比例的買賣也計入，0050 證交稅 0.1%）；只用當天已知的資料：可設基本面門檻（月營收年增率、連續年增月數、近 3 月年增率、季報 EPS 與 ROE，以及 Yahoo 單季財報的毛利率、營業利益率、三率三升 `margins_yoy_up` 與負債比上限，以及最新月營收創歷史新高 `revenue_record`、最新月營收下限 `min_monthly_revenue`（千元，排除低基期失真的小型股）與近四季自由現金流為正 `positive_free_cash_flow`，以及換股當天官方本益比上限 `max_pe` 與股價淨值比上限 `max_pb`（虧損或沒有資料的不會通過）），月營收與季報依法定公布期限只用換股當天已公布的那一期（月營收次月 11 日起、季報 Q1 5/16、Q2 8/15、Q3 11/15、Q4 隔年 4/1 起），只用於新買進；期間內下市的股票算到最後一筆收盤）、換股日偏移穩健度（`/stocks/screen/backtest/offsets`：同一組回測條件把換股日偏移 0 到 `hold_days` − 1 都跑一遍（超過 20 個時平均抽 20 個），回傳每個偏移的彙總與扣成本後年化報酬、夏普值、最大回撤、年化波動率（與同期 0050 對照）的最小／四分位／中位數／平均／最大值，以及年化報酬、夏普值勝過同期 0050、最大回撤比 0050 淺的偏移比例，另附依年度的分布（各年扣成本後報酬與同期 0050 的分布、勝過 0050 的偏移比例），用來分辨好結果是條件本身還是碰巧遇上某些換股日；可設樣本內外分割日 `split`，換股日早於這天的期數與其餘期數各算一次分布，看前段調出來的條件到後段是否仍然成立）、0050 核心部位前緣（`/stocks/screen/backtest/frontier`：同一組回測條件搭配 0%～100%（每 10%）的 0050 核心部位，每個比例回傳各換股日偏移的績效分布，並標出扣成本後夏普值中位數、年化報酬中位數最高的比例；設 `split` 時每個比例另附樣本內外的分布，並標出只看樣本內時夏普值最高的比例；每個偏移只查一次候選股）、具名策略（`/strategy-presets` 列出存好的回測條件；回測、偏移、前緣、歸因、滾動前推都可用 `preset=名稱` 引用，請求另帶的參數覆蓋存的值；新增、修改與刪除走手動回補管理介面的 `/api/strategy-presets/{name}`，存之前以回測驗證檢查條件）、滾動前推最佳化（`/stocks/screen/backtest/walk-forward`：每年年初只用之前 `train_years`（1–5，預設 3）年的期數，在 0050 核心部位 0／30／50／70%、單檔上限不設／10／20%、換股緩衝 `limit` 的 1／1.5／2 倍共 36 組中挑扣成本後夏普值最高的一組套用到這一年，回傳逐年挑出的參數與報酬、串起樣本外年份的彙總，並與不調參數的同期結果及 0050 比較；各換股日偏移都跑一次並回傳分布，每個偏移只查一次候選股）、回測報酬歸因（`/stocks/screen/backtest/attribution`：以 Brinson-Fachler 把策略部位相對「類 0050 基準」（每個換股日母體中近 20 個交易日平均成交金額最高的 50 檔、依成交金額加權）的超額報酬拆成產業配置、選股與交互三項，附整段、依年度（每年影響最大的 5 個產業）與各產業的效果及平均權重，以及同期 0050 報酬對照基準的貼近程度；策略的現金部位視為報酬 0 的類別，效果加總不複利、未扣成本）、大盤指數、市場估值水位（`/market/valuation`：上市櫃本益比與股價淨值比中位數（只計大於 0 的官方值，每日 17:30、20:30 官方補值後預先算好存在 `daily_market_valuation`）近 1–10 年的每日分布、最新一天的百分位與每月走勢）、大盤趨勢狀態（加權指數相對 5／20／60／120／240 日均線的位置與方向、均線多空排列、站在 60 日均線同一側的天數、52 週高低點距離、近 5／20／60／240 日漲跌幅與近 60 個交易日站上或跌破 20／60／120／240 日均線的日子，並附市場廣度：上市櫃站上 20／60／240 日均線的股票比例、5 個交易日前的 60 日線比例與近 5 日創 52 週新高／新低家次）、股利行事曆、外資持股排行、當日漲跌幅／成交量排行、產業表現（各產業單日或近 N 個交易日的漲跌家數、漲跌幅中位數、市值加權漲跌幅、成交值占比、本益比／股價淨值比中位數與最大漲跌個股）、CAGR 排行等唯讀查詢。
  - 除 `/api/v1/healthz` 外都需要 `Authorization: Bearer <DATA_API_KEY>`；未設定 `DATA_API_KEY` 時一律拒絕。
  - OpenAPI 文件在 `/api-docs/openapi.json`，Swagger UI 在 `/swagger-ui`（不需驗證）。
  - `/api/v1/portfolio/summary` 回傳目前持股的組合總覽（市值、權重、未實現損益、已領股利、產業分布、市值加權殖利率與 F 分數、組合本益比與股價淨值比（市值加權的調和平均，虧損不列入），每檔附官方本益比、股價淨值比、相對強度分數、外資／投信連續買賣超天數、最新月營收年增率與連續年減月數、最近一季毛利率／營業利益率／稅後淨利率與三率方向、近四季自由現金流（金融保險業不計三率與自由現金流），以及基本面警示：營收連續 3 個月以上年減、三率三降、營業虧損、近四季自由現金流為負），`member_id=0`（預設）為全部會員合計；屬個人持股資料，只有持有 `DATA_API_KEY` 者能查詢。
  - `/api/v1/portfolio/history` 回傳持股近 1～120 個月的時間加權報酬（扣除買賣金額、不含現金股利）、月報酬、最大回撤與同期加權指數比較，並以每日報酬計算相對加權指數的 beta、年化 alpha、相關係數、追蹤誤差與年化波動率（成對交易日至少 20 天）。
  - `/api/v1/portfolio/dividend-forecast` 以目前持股預估未來 1～24 個月每月的現金股利入帳（已公告配息加上去年同期推估，不扣二代健保與稅），並回傳預估殖利率。
+ Telegram bot 用於排程提醒（除權息、股利發放、公開申購、持股月營收（標示創歷史新高與連續 3 個月以上年減）、持股財報（EPS、ROE 與三率對去年同季的變化，標示三率三升／三降）、外資持股變化、持股法說會摘要、持股主力進出、持股董監質押變動、持股法人與信用交易、持股千張大戶週報、持股技術訊號、資料健康週報、大盤趨勢變化、策略換股（策略持股有基本面警示時另列））、價格追蹤通知與錯誤告警。

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
+ `STRATEGY_ALERT_QUERY`、`STRATEGY_ALERT_NAME`：（相容用，建議改用具名策略：`alert` 為 true 的策略每個交易日 22:10 逐一通知）另一組策略換股通知的回測條件（與 `/api/v1/stocks/screen/backtest` 相同的 query string，必須含 `from`，換股日從起日起算；起日到今天不可超過 3,653 天）與通知標題中的名稱；未設定條件時不通知。
+ `GO_GRPC_TARGET`、`GO_GRPC_TLS_CERT_FILE`、`GO_GRPC_TLS_KEY_FILE`、`GO_GRPC_DOMAIN_NAME`：對外 Go gRPC 服務連線設定。


### 免責聲明
本網提供之所有資訊內容均僅供參考，不涉及買賣投資之依據。使用者在進行投資決策時，務必自行審慎評估，
並自負投資風險及盈虧，如依本網提供之資料交易致生損失，本網不負擔任何賠償及法律責任。您自行負責依據
自身投資目標及個人、財務狀況，確定任何投資、證券或任何其他投資產品服務是否適合自身的需要。
本網站所載或本網站上、通過本網站提供的任何服務、內容、資訊及/或資料在任何情況下均不得被解釋為提供投資、
法律意見或提供投資服務。特請訪問此類網頁的人士就有關任何本網資料是否適合其投資需求徵詢適當獨立專業意見。
