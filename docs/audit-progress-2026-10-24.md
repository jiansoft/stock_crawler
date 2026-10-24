# 2026-10-24 跨專案盤查與開發結果

本輪功能與本機驗證已完成；程式以三個分支分別提交。正式資料庫 migration、正式部署與 Pi 實機驗證尚未執行。

## 分支與交付

| 專案 | 分支 | 基準分支 | 交付 |
|---|---|---|---|
| stock_crawler | `fix/audit-cache-and-etf-safety` | `main` | 本次 Rust 變更 |
| stock_go | `fix/audit-deploy-readiness` | `master` | [PR #19](https://github.com/jiansoft/stock_go/pull/19)，`3d1391e` |
| stock-mcp-go | `refactor/audit-currency-and-prompts` | `main` | [PR #85](https://github.com/jiansoft/stock-mcp-go/pull/85)，`081638a` |

兩個 Go 專案在 `.audit-work/` 的既有 worktree 開發，原工作目錄保留原分支。提交沒有 AI trailer；不重寫已發布的主線歷史。

## 盤查與處理

| 項目 | 結果 |
|---|---|
| Proto 產碼路徑與換行 | 實際路徑為 `src/interfaces/rpc/*.rs`，以 `.gitattributes` 固定 LF；四個產碼檔沒有實質內容差異 |
| ETF 回補快取 miss | 市場資料缺漏時記錄並跳過，不使整批回補失敗 |
| 異常行情與鎖 | 去重改用讀鎖快徑；批次寫入的驗價與日誌移出快照寫鎖；單筆寫入先在讀鎖外驗價，取得寫鎖後只在昨收變動時才重驗（此重驗仍在寫鎖內），驗價之後才變動的漲跌停區間不會重驗，下一筆報價會以新區間為準；增加 IPO Unlimited／恢復 Range 回歸測試 |
| IPO 漲跌限制 | 採用 Yahoo 當天的 Unlimited 資料；主檔沒有可靠上市日期，不以匯入日推定五個交易日 |
| ETF 查詢快取 | Moka future，容量 512、TTL 300 秒、日期與世代分鍵、同鍵合併、錯誤不快取；個人持股與市值每次重讀；成分股、績效及 snapshot 寫入成功後失效 |
| 回補 SSE | 清單及單一工作端點，重連先送快照、15 秒心跳、落後重讀最新狀態、完成／移除事件；原生管理頁顯示階段百分比 |
| 折溢價警示 | 保留既有單日 +2% 與連五日 -1%；新增連三交易日同向超過 ±1.5%，以市場交易日曆排除缺日且只在剛滿三日發出 |
| ETF 比較基準 | 使用者已選擇 0050 代理基準；API、網頁與 MCP 明確標示「代理基準報酬差異」，不可當成實際標的指數追蹤誤差 |
| 代理基準計算 | 近一年相同起訖交易日期的含息報酬區間，至少 120 組樣本；年化樣本標準差乘 √252、配對區間累積報酬差；不足為 null，附基準種類、資料日及樣本數 |
| Rust 對外契約 | 新增 `etf_tracking_error.sql`、每日績效寫入、tracking-error handler、OpenAPI schema/path 與契約測試；CI 兩套資料庫初始化已納入建表 SQL |
| Go backtest bridge | 白名單對齊 Rust 的正式參數名稱；固定功能、禁止 redirect、私有伺服器金鑰、30 秒逾時與 8 MiB 回應上限、錯誤遮罩 |
| 四個 Next.js 工作台 | ETF 曝險、ETF 比較與代理差異、填息進度、多因子回測與滾動前推；完成頁面入口、首頁與導覽、查詢／回應驗證、空值與錯誤顯示、取消舊請求 |
| Go 排程與畫面布局 | 正式與開發的 enableCrawler 原已為 false；既有 CSS 已採 Flex 與容器捲動，無須再改為 ResizeObserver 列數計算 |
| Legacy 資產 | 保留回切快照；新增 `STOCK_LEGACY_FRONTEND=false` 與 Docker `--target api`，可不載入、不部署 static 資產；未設定開關時維持既有行為 |
| Pi 部署探測 | 既有總上限原為 150 秒；改為立即探測、2／4／8／16／20 秒退避，補上 SSH 與單次 curl 逾時 |
| MCP 個股完整分析 | 先查 profile，至多四個並行請求聚合九個歷史分析區塊，整體 25 秒，部分失敗保留其他區塊與安全警示 |
| MCP 持股歷史 | 既有 changes 工具原已支援 1–24 期與完整 structuredContent；新增直接歷史查詢 alias、完整每期摘要及註冊／範圍測試 |
| MCP 金額與描述 | 統一來源單位換算與元／萬／億顯示，保留負號及結構化精度；精簡工具描述、保留關鍵語意與參數 schema |
| Windows 驗證 | CSS 文字契約先正規化 CRLF；既有 ETF 測試等待網址更新；限制兩個測試 worker，Windows 單項流程上限 20 秒，覆蓋率門檻維持 90% |

## 驗證結果

| 檢查 | 結果 |
|---|---|
| Rust `cargo test --locked --quiet -- --test-threads=1` | 1,405 通過、0 失敗、397 個外部服務／手動測試略過；doc-tests 5 通過、9 略過 |
| Rust fmt | `cargo fmt --all -- --check` 通過 |
| Rust Clippy | `--all-targets --all-features --locked -- -D warnings` 通過，包含新增整合測試的編譯 |
| Rust rustdoc | `-D warnings -A rustdoc::private_intra_doc_links`、`--document-private-items` 通過 |
| Go | `go test ./...`、`go test -race ./internal/frontend`、`go vet ./...` 通過 |
| MCP | `go test ./...`、`go test -race ./...`、`go vet ./...` 通過 |
| Next.js Node 22.23.4 | `npm run check` 通過：typecheck、lint、372 項測試、coverage、正式 build、client boundary |
| 前端覆蓋率 | statements 95.17%、branches 91.10%、functions 94.24%、lines 96.22%；新 analysis 模組 statements／functions／lines 100%、branches 94.85% |
| Playwright Node 22 | 桌面與手機完整 E2E 83 通過、1 項桌機列高案例於手機略過；四個新頁面的無障礙、溢位、BFF 邊界與失敗重試均通過 |
| 金鑰邊界 | 正式 client chunks 檢查包含 Go／Rust 伺服器環境名稱，未發現外洩；瀏覽器只呼叫同源 BFF |
| 部署腳本 | PowerShell 語法解析通過；未連線部署或驗證 Pi |

E2E 使用本機 fixture Go API；未對正式服務執行測試寫入。畫面截圖與測試 log 留在 `.audit-work/`，不提交可重建的產物。

## 部署順序與驗證限制

1. 先在部署資料庫套用 `etc/sql/etf_tracking_error.sql`，再部署 Rust 並執行每日 ETF 績效計算。既有歷史行情可重算，但本輪沒有對正式庫執行。
2. Go 設定 `RUST_DATA_API_BASE_URL`、`RUST_DATA_API_KEY`；Next 沿用 `GO_API_BASE_URL`。不得把 Rust 金鑰放入 `NEXT_PUBLIC_*`。
3. 採 Next 前端的 Go 部署可設 `STOCK_LEGACY_FRONTEND=false`；回切 legacy 前須啟用開關並補上資產。
4. 最後部署新版 Next 與 MCP，確認代理基準標示、樣本數與缺值。

本機沒有可用的獨立 PostgreSQL／Redis 測試服務，未執行 Rust 外部服務整合測試。已補資料庫 route 語意測試及 CI 建表流程，須由 CI 的隔離服務驗證。Docker image、ARM 產物與 Pi 實機部署也未在本輪執行。
