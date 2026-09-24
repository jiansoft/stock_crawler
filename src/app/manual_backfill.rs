//! 手動資料回補測試入口。
//!
//! 這個檔案集中放置平常不應自動執行、但缺資料時可用 `cargo test ... -- --ignored`
//! 直接觸發的回補測試。每個測試都依賴本機 `.env`、資料庫與外部資料來源，
//! 因此一律標記為 `#[ignore]`。
//!
//! 目前提供下列手動回補操作：
//!
//! - `test_backfill_daily_quotes_for_date`：
//!   依 [`MANUAL_DAILY_QUOTE_DATE`] 重新抓取上市櫃各股每日收盤報價，寫入 `DailyQuotes`。
//! - `test_backfill_closing_aggregate_for_date`：
//!   依 [`MANUAL_CLOSING_AGGREGATE_DATE`] 重跑每日收盤事件匯總，包含收盤報價回補、
//!   缺漏補齊、均線、最後交易日報價、估價、殖利率排行與市值重算。
//! - `test_backfill_taiwan_stock_index`：
//!   依 [`MANUAL_TAIWAN_STOCK_INDEX_DATE`] 回補指定日期的台股加權指數，
//!   跳過快取檢查後 upsert 寫入 `Index` 並更新快取。
//! - `test_backfill_received_dividend_records_for_stock`：
//!   依 [`MANUAL_DIVIDEND_RECORD_SECURITY_CODE`] 重算指定股票目前持股的已領股利總表與明細。
//! - `test_backfill_historical_dividends_for_stock`：
//!   依 [`MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE`] 從 Yahoo 回補單檔股票歷年股利，
//!   寫入 `dividend` 表、重算年度彙總列，並同步回補已領股利紀錄。
//! - `test_backfill_cagr_for_date`：
//!   依 [`MANUAL_CAGR_DATE`] 重算指定基準日的全市場各期間年化報酬率，寫入 `stock_cagr`。
//! - `test_backfill_quote_history_for_symbols`：
//!   依 [`MANUAL_QUOTE_HISTORY_SYMBOLS`] 與月份區間，從 TWSE 個股月行情回補
//!   歷史日報價缺口（只補空位，不覆寫既有資料）。
//! - `test_backfill_cagr_period`：
//!   依 [`MANUAL_CAGR_PERIOD`] 為既有的歷史基準日回填單一統計期間（新增期間後專用）。
//! - `test_backfill_listed_capital_reductions`：
//!   自 2010 年起全量回補**上市**減資事件到 `corporate_action`。TWSE 端點支援
//!   日期區間，單一請求即可取回十餘年，因此這是一次性的補歷史操作。
//! - `test_backfill_otc_capital_reductions`：
//!   回補**上櫃**當期公告的減資事件。TPEx 只給當週資料，這個入口僅用於
//!   排程漏跑時的補救，補不回更早的歷史。
//! - `test_scan_otc_capital_reduction_gaps`：
//!   **只讀**。列出上櫃仍未解釋的價格跳動，收斂成逐檔待辦清單，
//!   供人工登錄或未來接上逐檔來源時當目標清單。
//! - `test_backfill_financial_reports_for_symbols`：
//!   依 [`MANUAL_FINANCIAL_REPORT_SYMBOLS`] 從 Yahoo 採集指定股票的三大財務報表，
//!   不讀寫 Redis 略過旗標。
//! - `test_backfill_financial_reports_all`：
//!   一次採集全部上市櫃股票的三大財務報表（首次建檔用，約 3～4 小時）。

use chrono::NaiveDate;

use crate::{
    app::backfill::{
        capital_reduction, capital_reduction_history, dividend, financial_report, quote,
        quote_history, taiwan_stock_index,
    },
    app::calculation::{cagr, dividend_record},
    app::event::taiwan_stock::closing,
    domain::performance::CagrPeriod,
    infra::cache::SHARE,
};

/// 手動回補各股每日收盤報價時使用的預設交易日。
const MANUAL_DAILY_QUOTE_DATE: &str = "2026-04-30";

/// 掃描上櫃減資歷史缺口的起始日。
///
/// 與 CAGR 最長的十年期間對齊並多留數年緩衝。
const MANUAL_CAPITAL_REDUCTION_SCAN_FROM: &str = "2014-01-01";

/// 手動回補收盤事件匯總時使用的預設交易日。
const MANUAL_CLOSING_AGGREGATE_DATE: &str = "2026-04-30";

/// 手動回補已領股利紀錄時使用的預設股票代號。
const MANUAL_DIVIDEND_RECORD_SECURITY_CODE: &str = "0056";

/// 手動回補單檔歷年股利時使用的預設股票代號。
const MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE: &str = "2887";

/// 手動重算各期間年化報酬率時使用的預設基準日。
///
/// 空字串表示改用資料庫中最新的交易日。
const MANUAL_CAGR_DATE: &str = "";

/// 手動回補歷史日報價的代號清單。
///
/// 空陣列表示自動採用「目前未下市、代號以 `00` 開頭」的全部 ETF／ETN ——
/// 2015–2021 的缺口正是整類代號，逐一列舉會漏。
const MANUAL_QUOTE_HISTORY_SYMBOLS: &[&str] = &[];

/// 手動回補歷史日報價的月份區間（含頭尾，日期部分會被忽略）。
const MANUAL_QUOTE_HISTORY_FROM: &str = "2015-01-01";
const MANUAL_QUOTE_HISTORY_TO: &str = "2021-12-01";

/// 手動回填單一統計期間時使用的期間代碼。
///
/// 新增期間後把這裡改成該期間的代碼再執行 `test_backfill_cagr_period`。
const MANUAL_CAGR_PERIOD: &str = "Y7";

/// 手動採集三大財務報表的預設股票代號（逗號分隔）。
///
/// 可用環境變數 `MANUAL_FINANCIAL_REPORT_SYMBOLS` 覆蓋。
const MANUAL_FINANCIAL_REPORT_SYMBOLS: &str = "8042,2330,2881";

/// 手動回補指定交易日的各股每日收盤報價。
///
/// 此測試等同把原本的 `backfill::quote::tests::test_execute` 集中到手動回補檔。
/// 它會重新呼叫 TWSE 與 TPEx 來源抓取上市櫃各股開高低收、成交量與本益比等欄位，
/// 抓取成功後才在單一 transaction 內刪除同日舊資料並批次寫回資料庫、更新快取。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_daily_quotes_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_daily_quotes_for_date() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_DAILY_QUOTE_DATE, "%Y-%m-%d")
        .expect("manual daily quote date should be valid");

    tracing::debug!("開始 app::manual_backfill::test_backfill_daily_quotes_for_date date={date}");

    // quote::execute 內部採「先抓取、後原子替換」：同日舊資料的刪除與新資料的
    // COPY 寫入綁在同一個 transaction，抓取或寫入失敗都不會留下資料缺口，
    // 因此這裡不需要先手動刪除當日資料。
    let quote_count = quote::execute(date)
        .await
        .expect("manual daily quote backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_daily_quotes_for_date date={date} quote_count={quote_count}"
    );
}

/// 手動執行每日收盤事件主要匯總流程。
///
/// 此測試等同把原本的 `event::taiwan_stock::closing::tests::test_aggregate`
/// 集中到手動回補檔。它會依指定交易日重跑收盤報價回補、缺漏補齊、均線、
/// last daily quote、估價、殖利率排行、市值重算與通知前置資料。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_closing_aggregate_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_closing_aggregate_for_date() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_CLOSING_AGGREGATE_DATE, "%Y-%m-%d")
        .expect("manual closing aggregate date should be valid");

    tracing::debug!(
        "開始 app::manual_backfill::test_backfill_closing_aggregate_for_date date={date}"
    );

    closing::aggregate(date)
        .await
        .expect("manual closing aggregate backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_closing_aggregate_for_date date={date}"
    );
}

/// 手動回補台股加權指數時使用的預設日期。
///
/// TWSE API 會依此日期回傳該月份所有交易日的指數資料。
const MANUAL_TAIWAN_STOCK_INDEX_DATE: &str = "2026-04-15";

/// 手動回補指定月份的台股加權指數。
///
/// 此測試等同把原本的 `backfill::taiwan_stock_index::tests::test_execute`
/// 集中到手動回補檔。它會使用 [`MANUAL_TAIWAN_STOCK_INDEX_DATE`] 呼叫 TWSE
/// 加權股價指數來源，將該月份所有交易日的指數 upsert 回 `Index`，並更新記憶體快取。
/// 回補模式會跳過快取檢查，確保所有資料都寫入資料庫。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_taiwan_stock_index -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_taiwan_stock_index() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_TAIWAN_STOCK_INDEX_DATE, "%Y-%m-%d")
        .expect("manual taiwan stock index date should be valid");

    tracing::debug!("開始 app::manual_backfill::test_backfill_taiwan_stock_index date={date}");

    let upserted_count = taiwan_stock_index::execute_for_date(date)
        .await
        .expect("manual taiwan stock index backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_taiwan_stock_index date={date} upserted_count={upserted_count}"
    );
}

/// 手動回補指定股票目前持股的已領股利紀錄。
///
/// 此測試等同把原本的
/// `calculation::dividend_record::tests::test_backfill_received_dividend_records_for_stock_backfills_after_dividend_insert`
/// 集中到手動回補檔。它會依股票代號找出目前持股與既有股利年度，
/// 並重算 `dividend_record_detail` 與 `dividend_record_detail_more`。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_received_dividend_records_for_stock -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_received_dividend_records_for_stock() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let security_code = MANUAL_DIVIDEND_RECORD_SECURITY_CODE;
    tracing::debug!(
        "開始 app::manual_backfill::test_backfill_received_dividend_records_for_stock security_code={security_code}"
    );

    let summary = dividend_record::backfill_received_dividend_records_for_stock(security_code)
        .await
        .expect("manual received dividend records backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_received_dividend_records_for_stock security_code={security_code} summary={summary:?}"
    );
}

/// 手動回補指定股票在 Yahoo 可取得的歷年股利明細。
///
/// 此測試會呼叫股利回補子流程 [`dividend::backfill_historical_dividends_for_stock`]，
/// 將單檔股票的歷年股利資料 upsert 回 `dividend` 表；若來源含季配或半年配，
/// 也會重算年度彙總列，最後同步回補目前持股的已領股利紀錄。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_historical_dividends_for_stock -- --ignored --nocapture`
/// 可用 `MANUAL_DIVIDEND_SECURITY_CODE` 指定股票；未設定時沿用預設代號。
#[tokio::test]
#[ignore]
async fn test_backfill_historical_dividends_for_stock() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let security_code = std::env::var("MANUAL_DIVIDEND_SECURITY_CODE")
        .unwrap_or_else(|_| MANUAL_HISTORICAL_DIVIDEND_SECURITY_CODE.to_string());
    tracing::debug!(
        "開始 app::manual_backfill::test_backfill_historical_dividends_for_stock security_code={security_code}"
    );

    let upserted_count = dividend::backfill_historical_dividends_for_stock(&security_code)
        .await
        .expect("manual historical dividends backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_backfill_historical_dividends_for_stock security_code={security_code} upserted_count={upserted_count}"
    );
}

/// 手動修復帶有配息日期的年度合計列。
///
/// 年度合計列 (`quarter = ''`) 的日期應該一律是 `'-'`；股票從年配改成分期配發時，
/// 原本的年度配息明細會與合計列撞上同一組主鍵，舊版只覆寫金額而留下日期，
/// 讓合計被當成另一次真實配息重複計入持股的已領股利。
///
/// 此入口掃出所有受影響的 (代號, 發放年度)，重算合計列把日期清回 `'-'`，
/// 再重算對應股票目前持股的已領股利紀錄。全程只讀寫資料庫，不會請求 Yahoo。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_repair_stale_annual_total_dividends -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_repair_stale_annual_total_dividends() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    tracing::debug!("開始 app::manual_backfill::test_repair_stale_annual_total_dividends");

    let summary = dividend::repair_stale_annual_total_dividends()
        .await
        .expect("manual stale annual total dividend repair failed");

    tracing::debug!(
        "結束 app::manual_backfill::test_repair_stale_annual_total_dividends summary={summary:?}"
    );
}

/// 手動重算指定基準日的全市場各期間年化報酬率（CAGR）。
///
/// 排程本身每日 05:40（台北時間）自動執行，這個入口用於：
/// 股利資料事後回補後需要重算、或首次上線時補算某一天的結果。
///
/// 計算完全依賴資料庫既有的報價與股利，不會呼叫任何外部網站；
/// 同一 `(基準日, 股票, 期間)` 重複執行為冪等的 upsert 覆蓋。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_cagr_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_cagr_for_date() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    // 空字串代表交由 use case 自行採用資料庫中最新的交易日。
    let date = if MANUAL_CAGR_DATE.is_empty() {
        None
    } else {
        Some(
            NaiveDate::parse_from_str(MANUAL_CAGR_DATE, "%Y-%m-%d")
                .expect("manual cagr date should be valid"),
        )
    };

    println!("開始 test_backfill_cagr_for_date date={date:?}");

    let summary = cagr::execute(date)
        .await
        .expect("manual cagr backfill failed");

    println!(
        "結束 test_backfill_cagr_for_date date={:?} universe={} periods_calculated={} periods_skipped={} rows_written={} anomaly_symbols={}",
        summary.date,
        summary.universe,
        summary.periods_calculated,
        summary.periods_skipped,
        summary.rows_written,
        summary.anomaly_symbols
    );
}

/// 從 TWSE 個股月行情回補歷史日報價缺口。
///
/// 排程與 `test_backfill_daily_quotes_for_date` 都是「一天的全市場」，補七年份
/// 的缺口得跑一千七百多次、還會把不缺的股票一起重抓。這個入口改用個股月行情
/// （`STOCK_DAY`），一次要一檔股票的一整個月。
///
/// 寫入是 `ON CONFLICT DO NOTHING`：只填空位，既有資料不覆寫也不刪除，
/// 因此中途失敗直接重跑即可。單月抓取失敗只記錄並繼續。
///
/// 注意請求量：預設區間 84 個月 × 全部 ETF（約 250 檔）超過兩萬次請求，
/// 每次間隔 1.2 秒，實際會跑數小時。先把 [`MANUAL_QUOTE_HISTORY_SYMBOLS`]
/// 設成一兩檔小範圍驗證，再放大。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_quote_history_for_symbols -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_quote_history_for_symbols() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let from = NaiveDate::parse_from_str(MANUAL_QUOTE_HISTORY_FROM, "%Y-%m-%d")
        .expect("manual quote history from should be valid");
    let to = NaiveDate::parse_from_str(MANUAL_QUOTE_HISTORY_TO, "%Y-%m-%d")
        .expect("manual quote history to should be valid");

    let symbols: Vec<String> = if MANUAL_QUOTE_HISTORY_SYMBOLS.is_empty() {
        quote_history::fetch_etf_symbols()
            .await
            .expect("fetch etf symbols failed")
    } else {
        MANUAL_QUOTE_HISTORY_SYMBOLS
            .iter()
            .map(|symbol| (*symbol).to_owned())
            .collect()
    };

    println!(
        "開始 test_backfill_quote_history_for_symbols symbols={} from={from} to={to}",
        symbols.len()
    );

    let summary = quote_history::execute(&symbols, from, to)
        .await
        .expect("manual quote history backfill failed");

    println!(
        "結束 test_backfill_quote_history_for_symbols months_requested={} months_with_data={} months_failed={} quotes_fetched={} rows_inserted={}",
        summary.months_requested,
        summary.months_with_data,
        summary.months_failed,
        summary.quotes_fetched,
        summary.rows_inserted
    );
}

/// 為既有的歷史基準日回填單一統計期間（[`MANUAL_CAGR_PERIOD`]）。
///
/// 新增期間之後專用：排程只算當日，既有基準日不會自動長出新期間的資料。
/// 此入口掃出「已有其他期間結果、但缺少該期間」的基準日逐日補算，
/// **只算指定期間**，不重算已存在的其他期間。
///
/// 從未計算過的日期不在範圍內；要初始化那些日期請改用
/// [`test_backfill_cagr_for_date`] 逐日執行。
///
/// 全程只讀資料庫既有的報價與股利，不呼叫任何外部網站；重複執行為冪等。
///
/// 執行範例：
/// `cargo test app::manual_backfill::test_backfill_cagr_period -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_cagr_period() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let period =
        CagrPeriod::from_code(MANUAL_CAGR_PERIOD).expect("manual cagr period 應為合法代碼");

    println!("開始 test_backfill_cagr_period period={}", period.code());

    let summary = cagr::backfill_period(period)
        .await
        .expect("manual cagr period backfill failed");

    println!(
        "結束 test_backfill_cagr_period period={} dates_pending={} dates_processed={} dates_skipped={} rows_written={}",
        period.code(),
        summary.dates_pending,
        summary.dates_processed,
        summary.dates_skipped,
        summary.rows_written
    );
}

/// 全量回補上市減資事件。
///
/// TWSE 的 `TWTAUU` 端點支援 `startDate`／`endDate` 區間查詢，
/// **單一請求**就能取回 2010 年至今的全部事件（實測 2015–2026 為 308 筆），
/// 因此不需要逐日輪詢。跑完後建議重跑 `test_backfill_cagr_for_date`，
/// 讓新登錄的減資反映到年化報酬率上。
///
/// `cargo test app::manual_backfill::test_backfill_listed_capital_reductions -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_listed_capital_reductions() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    println!("開始 test_backfill_listed_capital_reductions");

    let saved = capital_reduction::backfill_listed_full()
        .await
        .expect("manual listed capital reduction backfill failed");

    println!("結束 test_backfill_listed_capital_reductions rows_written={saved}");
}

/// 回補上櫃當期公告的減資事件。
///
/// TPEx 的 `revivt` 只回當週公告且不接受日期參數，所以這個入口**補不回歷史**，
/// 僅供排程漏跑當週時補救。歷史缺口請見
/// [`crate::app::backfill::capital_reduction_history`]。
///
/// `cargo test app::manual_backfill::test_backfill_otc_capital_reductions -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_otc_capital_reductions() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    println!("開始 test_backfill_otc_capital_reductions");

    let saved = capital_reduction::backfill_otc_current()
        .await
        .expect("manual OTC capital reduction backfill failed");

    println!("結束 test_backfill_otc_capital_reductions rows_written={saved}");
}

/// 掃描上櫃減資的歷史缺口，列出待辦清單。
///
/// **只讀，不寫任何資料。** 上櫃沒有可回溯的減資來源（TPEx 的公告只涵蓋當週），
/// 而比例無法從報價反推——漲跌停只能把它框在 ±10% 區間，減資比例又不是整數倍，
/// 框不出唯一解。寧可留白也不要寫入猜測值。
///
/// 建議在 `test_backfill_listed_capital_reductions` 之後執行，此時清單中
/// 才不會混入已由 TWSE 全量覆蓋的上市部分。
///
/// `cargo test app::manual_backfill::test_scan_otc_capital_reduction_gaps -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_scan_otc_capital_reduction_gaps() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let to = chrono::Local::now().date_naive();
    let from = NaiveDate::parse_from_str(MANUAL_CAPITAL_REDUCTION_SCAN_FROM, "%Y-%m-%d")
        .expect("manual capital reduction scan from should be valid");

    println!("開始 test_scan_otc_capital_reduction_gaps from={from} to={to}");

    let report = capital_reduction_history::scan(from, to)
        .await
        .expect("manual capital reduction gap scan failed");

    println!("{}", capital_reduction_history::format_report(&report));
    println!(
        "結束 test_scan_otc_capital_reduction_gaps symbols={} events={}",
        report.symbol_count(),
        report.event_count()
    );
}

/// 採集指定股票的 Yahoo 三大財務報表（損益表、資產負債表、現金流量表）。
///
/// 每檔 5 次請求，不讀寫 Redis 略過旗標，適合補單檔或驗證資料。
/// 任一檔失敗即中止，方便看到錯誤原因。
///
/// `cargo test app::manual_backfill::test_backfill_financial_reports_for_symbols -- --ignored --nocapture`
/// 可用 `MANUAL_FINANCIAL_REPORT_SYMBOLS=2330,2317` 指定股票。
#[tokio::test]
#[ignore]
async fn test_backfill_financial_reports_for_symbols() {
    dotenvy::dotenv().ok();

    let symbols = std::env::var("MANUAL_FINANCIAL_REPORT_SYMBOLS")
        .unwrap_or_else(|_| MANUAL_FINANCIAL_REPORT_SYMBOLS.to_string());
    let repo =
        crate::infra::database::repository::financial_report::PgFinancialReportRepository::new();

    for symbol in symbols.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let counts = financial_report::backfill_for_stock(&repo, symbol)
            .await
            .unwrap_or_else(|why| panic!("financial report backfill failed for {symbol}: {why:#}"));
        println!(
            "{symbol}: income={} balance={} cash_flow={}",
            counts.income_statements, counts.balance_sheets, counts.cash_flow_statements
        );
    }
}

/// 一次採集全部上市櫃股票的 Yahoo 三大財務報表（首次建檔用）。
///
/// 約 1,800 檔、每檔 5 次請求，需要 3～4 小時。會寫入 7 天的 Redis 略過旗標，
/// 中斷後重跑會從未處理的股票接續；排程之後也會自然接手。
///
/// `cargo test app::manual_backfill::test_backfill_financial_reports_all -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_financial_reports_all() {
    dotenvy::dotenv().ok();

    let summary = financial_report::backfill_all()
        .await
        .expect("financial report backfill failed");

    println!("結束 test_backfill_financial_reports_all {summary:?}");
}
