# stock_crawler 架構指南

更新日期：2026-06-08

## 目錄結構

```text
src/
├─ main.rs           # 程式入口：載入設定、檢查外部服務、啟動排程與服務
├─ core/             # 共用基礎設施（無業務邏輯）
│  ├─ config.rs      # 應用程式設定（app.json + 環境變數）
│  ├─ declare.rs     # 共用型別與常數定義
│  ├─ logging/       # 非同步檔案日誌與輪替
│  └─ util/          # 通用工具（HTTP client、日期、文字、原子操作、診斷）
├─ domain/           # 領域層：核心概念、實體、事件與倉儲合約
│  ├─ dividend/      # 股利領域實體與 DividendRepository
│  ├─ financial/     # 財務報表、月營收實體與 FinancialRepository
│  ├─ portfolio/     # 投資組合領域實體與倉儲合約
│  ├─ registry/      # 證券登錄/主檔領域實體與倉儲合約
│  └─ events.rs      # 領域事件定義
├─ app/              # 應用層：業務流程編排與用例
│  ├─ scheduler.rs   # cron 排程器（所有定時任務的註冊點）
│  ├─ backfill/      # 歷史資料回補流程
│  ├─ event/         # 盤後事件處理（收盤計算、追蹤觸發）
│  ├─ calculation/   # 衍生計算（資金歷史、估值、股利計算等）
│  └─ manual_backfill.rs  # 手動回補（#[cfg(test)] 測試入口）
├─ infra/            # 基礎設施層：外部系統存取與技術實作
│  ├─ cache/         # 共享記憶體快取（含 Redis 背景同步）
│  ├─ nosql/         # Redis 客戶端
│  ├─ crawler/       # 外部資料來源爬蟲
│  │  ├─ twse/       # 台灣證券交易所
│  │  ├─ tpex/       # 櫃買中心
│  │  ├─ yahoo/      # Yahoo 股市
│  │  ├─ goodinfo/   # 台灣股市資訊網
│  │  └─ ...         # 其他資料來源
│  └─ database/      # PostgreSQL 資料存取
│     ├─ repository/ # domain repository trait 的 PostgreSQL 實作與 entity/table 映射
│     │  ├─ dividend.rs
│     │  ├─ financial.rs
│     │  ├─ portfolio.rs
│     │  └─ stock.rs
│     └─ table/      # 各資料表 CRUD 與 SQL 查詢（依資料表/領域分組）
│        ├─ stock/       # 股票主檔、交易所、市場、產業、持股明細與延伸資料
│        ├─ quote/       # 日報價、最後報價、歷史報價、統計
│        ├─ dividend/    # 股利主檔、配發日、明細與延伸查詢
│        ├─ financial/   # 財報、估值、營收
│        ├─ money_flow/  # 資金流向與法人
│        └─ (根層)       # config、trace、yield_rank、index 等橫切輔助表
└─ interfaces/       # 對外介面層
   ├─ rpc/           # gRPC 服務、client 與 proto 產碼
   ├─ web/           # HTTP/Axum Web 服務
   └─ bot/           # Telegram Bot 通知
```

## 分層規則

### 依賴方向（嚴格單向）

```text
main.rs    → core / domain / app / infra / interfaces
interfaces → core / domain / app / infra
app        → core / domain / infra
infra      → core / domain
domain     → core
core       → （不依賴其他層）
```

`main.rs` 是組裝根，允許連接各層啟動流程。其他模組應遵守單向依賴，避免低層反向知道高層流程。

### 禁止的依賴

| 來源層 | 禁止依賴 | 原因 |
|--------|----------|------|
| `core` | `domain`, `app`, `infra`, `interfaces` | core 是最底層共用能力，不可帶入業務或外部系統細節 |
| `domain` | `app`, `infra`, `interfaces` | 領域層只描述核心概念與合約，不直接存取資料庫/API/服務入口 |
| `infra` | `app`, `interfaces` | 基礎設施實作不應知道業務流程或對外介面 |
| `app` | `interfaces` | 應用用例不應知道 gRPC/HTTP/Bot 的入口細節 |

## 層級職責

### `core/`

`core` 放跨領域、跨層共用的技術能力，例如設定、日誌、日期處理、HTTP helper、文字處理與診斷工具。這一層不應知道股票、股利、財報等業務流程。

### `domain/`

`domain` 放領域實體、領域事件與 repository trait。這裡定義「系統認為資料是什麼」與「用例需要哪些讀寫合約」，但不決定 PostgreSQL、Redis 或外部 API 怎麼實作。

目前主要模式：

- `domain/<領域>/entity.rs`：領域實體。
- `domain/<領域>/repository.rs`：倉儲合約 trait。
- `domain/events.rs`：跨領域可觀察的重要事實。

### `app/`

`app` 放業務用例與流程編排，負責把爬蟲、資料庫、快取、領域 entity/repository 組合成可執行流程。排程任務、回補流程、盤後事件、衍生計算都屬於這一層。

### `infra/`

`infra` 封裝外部系統與技術細節，包含 PostgreSQL、Redis、外部資料來源爬蟲與快取同步。

`infra/database/repository/` 是目前重構後新增的邊界層：它實作 `domain/*/repository.rs` 的 trait，並負責領域實體與 `infra/database/table/` 資料表模型之間的轉換。

`infra/database/table/` 保留資料表導向的 SQL/CRUD。新增業務流程時，優先透過 repository trait 使用資料庫；只有在維護既有表格查詢、建立低階 SQL helper 或尚未抽象成領域合約時，才直接放在 `table/`。

### `interfaces/`

`interfaces` 放外部入口與傳輸協定，例如 gRPC、HTTP/Axum、Telegram Bot。這一層可以呼叫 `app` 用例，也可以做必要的 DTO/transport 轉換，但不應承載核心業務規則。

## 新模組放置規範

### 放入 `core/` 的條件

- 不含任何業務邏輯。
- 被 2 個以上的其他層使用。
- 例：新的通用工具函式、跨層共用型別、日誌/診斷輔助。

### 放入 `domain/` 的條件

- 描述股票、股利、財報、投資組合等核心概念。
- 定義 repository trait 或領域事件。
- 不直接使用 SQLx、Redis client、HTTP client 或 crawler。
- 例：新的領域實體、領域事件、資料讀寫合約。

### 放入 `app/` 的條件

- 編排多個 `infra` 模組完成一個業務用例。
- 需要呼叫 crawler、repository、table、cache 或 bot 來完成流程。
- 例：新的排程任務、新的回補流程、新的盤後事件處理、新的批次計算。

### 放入 `infra/` 的條件

- 封裝對外部系統的存取（資料庫、API、快取、Redis）。
- 實作 `domain` 定義的 repository trait。
- 例：新的爬蟲來源、新的 PostgreSQL repository 實作、新的資料表 CRUD。

### 放入 `interfaces/` 的條件

- 提供對外服務入口或通訊協定轉換。
- 例：新的 gRPC service、HTTP endpoint、Bot 指令、transport DTO。

## Repository 與 Table 使用規則

新增或重構資料存取時，優先順序如下：

1. 若資料屬於明確領域，先在 `domain/<領域>/repository.rs` 定義 trait。
2. 在 `infra/database/repository/<領域>.rs` 實作 trait，處理 SQL/table model 與 domain entity 的映射。
3. 將 SQLx 查詢、upsert 與資料表導向 helper 放在 `infra/database/table/`。
4. `app` 優先依賴 repository trait 或具體 repository 實作；避免在新的高階流程中散落大量 table-level SQL 呼叫。

例外情境：

- 既有流程尚未抽象成領域合約，可暫時直接呼叫 `infra/database/table/*`。
- 純資料表維護、索引重建、橫切輔助表（如 `trace`、`config`、`yield_rank`）可留在 table 根層或既有分類。
- 若 query 結果是特定畫面/API/報表 DTO，不一定要放進 domain entity；可放在 table extension 或 interface/app 專用 DTO。

## `infra/database/table/` 領域分組

新增資料表時，依業務領域放入對應子目錄：

| 領域 | 子目錄 | 包含內容 |
|------|--------|----------|
| 股票主檔 | `stock/` | 股票、交易所、市場、產業、持股明細、ETF/下市/權重等延伸資料 |
| 報價 | `quote/` | 日報價、最後報價、歷史報價、統計 |
| 股利 | `dividend/` | 股利主檔、配發日、明細與延伸查詢 |
| 財務 | `financial/` | 財報、估值、營收 |
| 資金流向 | `money_flow/` | 法人買賣超、資金歷史與明細 |
| 橫切/輔助 | `table/` 根層 | config、trace、yield_rank、index |

## 建置相關

### Proto 產碼

- `.proto` 檔案位於 `etc/proto/`。
- `build.rs` 會將產碼輸出至 `src/interfaces/rpc/`。
- 修改 `.proto` 後需執行 `cargo build` 重新產生。
- 不要手動編輯 `src/interfaces/rpc/basic.rs`、`control.rs`、`stock.rs`、`manual_backfill.rs` 等產碼。

### 跨平台建置

- **Windows 開發**：`cargo build` 或 `cargo check`。
- **Linux musl 交叉編譯**：使用 `build.bat` / `build.ps1`（需 Zig + CMake）。
- **容器部署**：使用 `Dockerfile_live`（distroless 映像）。

