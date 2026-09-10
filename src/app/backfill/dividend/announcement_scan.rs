//! # 除權除息公告每日掃描
//!
//! 每天掃一次交易所的除權除息預告表，主動發現「資料庫沒收到」的股利事件，
//! 並把除權息日與現金股利發放日補到既有資料上。
//!
//! ## 為什麼需要這條流程
//!
//! 既有的 [`super::unannounced_ex_dividend_date`] 是 pull-by-symbol：
//! 它只會挑出**資料庫裡已經有、但日期還沒公布**的列去回補。換句話說，
//! 如果某次配息從頭到尾沒被寫進資料庫，那條路徑永遠不會發現它。
//! 本流程反過來從交易所的全市場公告出發，因此能找出漏抓的事件。
//!
//! ## 資料來源分工
//!
//! | 來源 | 補的欄位 |
//! |------|----------|
//! | TWSE `TWT48U_ALL`／TPEx `tpex_exright_prepost` | 除權息日、現金股利、無償配股率 |
//! | MOPS `t187ap45_L`（僅上市） | 股利所屬年度與季別、盈餘／公積的現金與配股拆分 |
//! | MoneyDJ 股利政策表 | 現金股利發放日 |
//! | Yahoo 股利政策（逐檔） | 上櫃與 ETF 的股利所屬年度、季別與發放年度 |
//!
//! ## 期別怎麼來：兩階段判定
//!
//! `dividend` 的唯一鍵是 `(security_code, year, quarter)`，而預告表只有除權息日期，
//! **沒有股利所屬期間**，無法決定一筆事件屬於哪一季。因此期別分兩階段解出：
//!
//! 1. **批次**：用 MOPS 的股利分派情形以「金額完全吻合」配對。一次請求涵蓋全部上市公司，
//!    成本最低，但櫃買中心沒有對應的開放資料（`t187ap45_O` 各種路徑都轉址到網頁，
//!    新版 MOPS 也只剩逐檔查詢的 SPA 端點），所以上櫃與 ETF 在這一階段一定落空。
//! 2. **逐檔補救**：第一階段配不出來的事件，才去抓 Yahoo 的股利政策頁，
//!    用**除權息日完全相符**找出對應的那一次配息，取它的所屬年度、季別與發放年度。
//!
//! 兩階段都判不出期別的事件只記錄成待處理項目，絕不猜季別寫進資料庫。
//!
//! ## 三個容易踩到的資料模型陷阱
//!
//! - **空季別在混合配息年度代表「年度合計」而不是事件**。同一發放年度若已有季配或
//!   半年配明細，空季別那一列是由 `upsert_annual_total_dividend` 聚合出來的合計列；
//!   真正的全年事件用 `A`（見 [`crate::infra::crawler::yahoo::dividend`]）。
//!   本流程一律先判斷是否為混合配息年度，合計列不參與任何比對，也不會被寫入日期。
//! - **發放年度不等於除權息日的年份**。季配、半年配跨年發放時（12 月除息、隔年 1 月發放），
//!   既有流程以**發放日**的年份分組。本流程沿用同一規則：Yahoo 已判定的發放年度優先，
//!   其次是現金發放日的年份，最後才退回除權息日的年份。
//! - **同一次配息可能拆成兩筆公告**（除息日與除權日不同天）。異動計畫以 serial 與唯一鍵
//!   彙整，讓兩筆公告累積套用到同一個實體上，否則後處理的那筆會把先前補好的日期洗掉。
//!
//! ## Yahoo 來源的金額不做拆分
//!
//! Yahoo 只給現金與股票股利的合計，沒有盈餘／公積的來源拆分。把公積配發誤記成盈餘
//! 會讓 [`super::payout_ratio`] 高估盈餘分配率，因此 Yahoo 補出來的資料列
//! 一律讓拆分欄位留 0（與既有 [`crate::app::backfill::acl::YahooDividendAclMapper`] 的作法一致），
//! 只填合計值。

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Result, anyhow};
use chrono::{Datelike, Local, NaiveDate};
use rand::RngExt;
use rust_decimal::Decimal;
use tokio_retry::{
    Retry,
    strategy::{ExponentialBackoff, jitter},
};

use crate::{
    app::calculation::dividend_record,
    core::declare::StockExchangeMarket,
    domain::dividend::{entity::Dividend, repository::DividendRepository},
    infra::{
        crawler::{
            moneydj::dividend_schedule::{self, DividendSchedule},
            mops::dividend_allotment::{self, DividendAllotment},
            share::ExDividendAnnouncement,
            tpex, twse,
            yahoo::{self, dividend::YahooDividend},
        },
        database::repository::dividend::PgDividendRepository,
    },
};

/// 日期尚未公布時寫入資料庫的值；與 Goodinfo 採集器保持一致。
const UNANNOUNCED_DATE: &str = "尚未公布";

/// 資料庫的日期字串格式。
const DATE_FORMAT: &str = "%Y-%m-%d";

/// 資料庫中代表「日期尚未公布」的兩種值。
///
/// 歷史資料同時存在 `-`（Goodinfo）與 `尚未公布`（Yahoo）兩種寫法，
/// 判斷是否已公布時兩者都要認得。
const UNANNOUNCED_DATE_VALUES: [&str; 2] = ["-", "尚未公布"];

/// 混合配息年度中，代表「全年事件」的季別代碼。
///
/// 空季別在這種年度是聚合出來的合計列，不是事件；兩者必須分開。
const FULL_YEAR_EVENT_QUARTER: &str = "A";

/// 逐筆列出「無法判定期別」事件的上限。
const UNRESOLVED_SAMPLE_LIMIT: usize = 10;

/// 單次掃描允許查詢 Yahoo 的股票檔數上限。
///
/// 正常情況下需要補救的只有個位數；設上限是為了避免資料庫剛清空或
/// 上游大量改版時，一次排程對 Yahoo 發出數百個請求而被擋。
const YAHOO_LOOKUP_LIMIT: usize = 60;

/// 事件無法判定期別的原因。
///
/// 分類是為了 log 的訊噪比與後續處理方式：能靠 Yahoo 補救的與真的沒救的要分開看。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum UnresolvedReason {
    /// 上櫃公司沒有對應的開放股利分派情形（櫃買中心未提供彙總資料）。
    OverTheCounterUnsupported,
    /// 上市公司找不到金額吻合的分派情形。
    ///
    /// 常見於 ETF 的收益分配（不適用公司股利分派）、公司尚未申報，
    /// 或同一年有兩期金額完全相同而無法分辨。
    NoMatchingAllotment,
    /// Yahoo 的股利政策裡找不到除權息日相符的紀錄。
    NoMatchingYahooDividend,
    /// Yahoo 頁面抓取失敗（含 404、連線失敗與重試耗盡）。
    YahooLookupFailed,
    /// 本次掃描的 Yahoo 查詢額度已用完，留待下次排程處理。
    YahooLookupSkipped,
    /// 資料庫中有多筆金額相同、日期未公布的列，無法確定對應哪一筆。
    AmbiguousExistingRow,
}

impl UnresolvedReason {
    /// 供 log 使用的說明文字。
    fn as_str(self) -> &'static str {
        match self {
            Self::OverTheCounterUnsupported => "上櫃無開放的股利分派情形",
            Self::NoMatchingAllotment => "找不到金額吻合的股利分派情形",
            Self::NoMatchingYahooDividend => "Yahoo 股利政策無除權息日相符的紀錄",
            Self::YahooLookupFailed => "Yahoo 股利政策抓取失敗",
            Self::YahooLookupSkipped => "本次 Yahoo 查詢額度已用完",
            Self::AmbiguousExistingRow => "資料庫有多筆金額相同且日期未公布的列",
        }
    }

    /// 是否還能靠逐檔查詢 Yahoo 補救。
    ///
    /// 第一階段（MOPS 配對）失敗的兩種原因都值得再試一次 Yahoo；
    /// 已經試過 Yahoo 的則不再重複。
    fn is_retryable_with_yahoo(self) -> bool {
        matches!(
            self,
            Self::OverTheCounterUnsupported | Self::NoMatchingAllotment
        )
    }
}

/// 無法對應到期別、因此不敢寫入的事件。
///
/// 保留整筆公告而不只是代號，第二階段才有足夠資訊組出要寫入的資料列。
#[derive(Debug, Clone, PartialEq)]
struct UnresolvedEvent {
    /// 來源公告。
    announcement: ExDividendAnnouncement,
    /// 無法處理的原因。
    reason: UnresolvedReason,
}

/// 已判定期別與金額的股利內容。
///
/// 這是「期別來源」的抽象：不論來自 MOPS 的分派情形還是 Yahoo 的股利政策，
/// 後續組資料列的流程都一樣。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedDividend {
    /// 股利所屬年度（西元）。
    year_of_dividend: i32,
    /// 來源已判定的發放年度；MOPS 沒有這項資訊時為 `None`。
    paid_year: Option<i32>,
    /// 所屬季度：空字串為年度，`Q1`～`Q4`、`H1`／`H2`，`A` 為混合配息年度的全年事件。
    quarter: String,
    /// 現金股利合計。
    cash_dividend: Decimal,
    /// 股票股利合計。
    stock_dividend: Decimal,
    /// 盈餘現金股利；無拆分來源時為 0。
    earnings_cash: Decimal,
    /// 公積現金股利；無拆分來源時為 0。
    capital_reserve_cash: Decimal,
    /// 盈餘股票股利；無拆分來源時為 0。
    earnings_stock: Decimal,
    /// 公積股票股利；無拆分來源時為 0。
    capital_reserve_stock: Decimal,
}

impl ResolvedDividend {
    /// 由 MOPS 的股利分派情形建立，帶完整的盈餘／公積拆分。
    ///
    /// MOPS 只揭露股利所屬期間，沒有發放年度，因此 `paid_year` 為 `None`，
    /// 交由 [`resolve_paid_year`] 依發放日或除權息日推定。
    fn from_allotment(allotment: &DividendAllotment) -> Self {
        Self {
            year_of_dividend: allotment.year_of_dividend,
            paid_year: None,
            quarter: allotment.quarter.clone(),
            cash_dividend: allotment.cash_dividend(),
            stock_dividend: allotment.stock_dividend(),
            earnings_cash: allotment.earnings_cash,
            capital_reserve_cash: allotment.capital_reserve_cash,
            earnings_stock: allotment.earnings_stock,
            capital_reserve_stock: allotment.capital_reserve_stock,
        }
    }

    /// 由 Yahoo 的單次配息紀錄建立；Yahoo 沒有拆分資料，拆分欄位留 0。
    ///
    /// Yahoo 本身就是依發放年度分組的，直接沿用它的年度，
    /// 才不會與既有回補流程對同一次配息各自寫出不同年度的資料列。
    fn from_yahoo(detail: &yahoo::dividend::YahooDividendDetail) -> Self {
        Self {
            year_of_dividend: detail.year_of_dividend,
            paid_year: Some(detail.year),
            quarter: detail.quarter.clone(),
            cash_dividend: detail.cash_dividend,
            stock_dividend: detail.stock_dividend,
            earnings_cash: Decimal::ZERO,
            capital_reserve_cash: Decimal::ZERO,
            earnings_stock: Decimal::ZERO,
            capital_reserve_stock: Decimal::ZERO,
        }
    }
}

/// 待新增資料列的唯一鍵：`(股票代號, 發放年度, 季別)`，與資料表的唯一索引一致。
type InsertKey = (String, i32, String);

/// 一次掃描要對資料庫做的所有異動。
///
/// 更新以 serial、新增以唯一鍵彙整：同一次配息可能拆成除息與除權兩筆公告，
/// 兩筆都必須累積套用到同一個實體上，否則後處理的那筆會把先前補好的日期洗掉。
#[derive(Debug, Default, PartialEq)]
struct ScanPlan {
    /// 既有資料列的日期補正；key 為 serial。
    updates: BTreeMap<i64, Dividend>,
    /// 資料庫沒有、需要新增的事件；key 為唯一鍵。
    inserts: BTreeMap<InsertKey, Dividend>,
    /// 無法判定期別的事件。
    unresolved: Vec<UnresolvedEvent>,
}

/// 掃描結果統計。
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ScanOutcome {
    /// 實際更新日期的筆數。
    pub updated: usize,
    /// 實際新增的筆數。
    pub inserted: usize,
    /// 無法判定期別而略過的筆數。
    pub unresolved: usize,
}

/// 執行除權除息公告掃描。
///
/// 三個批次來源以 `tokio::join!` 併發抓取。預告表是流程主體，兩個市場都失敗時
/// 直接回傳錯誤；MOPS 與 MoneyDJ 屬於補強來源，任一失敗只會記錄並降級
/// （少了 MOPS 就多依賴第二階段的 Yahoo，少了 MoneyDJ 就不補現金發放日），
/// 讓「補除權息日」這個最重要的工作仍然完成。
pub(super) async fn scan_ex_dividend_announcements() -> Result<ScanOutcome> {
    let (listed, otc, allotments, schedules) = tokio::join!(
        twse::ex_dividend_announcement::visit(),
        tpex::ex_dividend_announcement::visit(),
        dividend_allotment::visit(),
        dividend_schedule::visit(),
    );

    let mut announcements = Vec::with_capacity(1024);
    let listed_failed = collect_or_log(listed, "上市除權息預告表", &mut announcements);
    let otc_failed = collect_or_log(otc, "上櫃除權息預告表", &mut announcements);
    if listed_failed && otc_failed {
        return Err(anyhow!("上市與上櫃的除權息預告表都無法取得"));
    }

    let allotments = match allotments {
        Ok(data) => index_allotments(data),
        Err(why) => {
            tracing::error!(
                "取得上市公司股利分派情形失敗，期別全部改由 Yahoo 判定: {:?}",
                why
            );
            HashMap::new()
        }
    };
    let schedules = match schedules {
        Ok(data) => index_schedules(data),
        Err(why) => {
            tracing::error!("取得 MoneyDJ 股利政策表失敗，本次不補現金發放日: {:?}", why);
            HashMap::new()
        }
    };

    let repository = PgDividendRepository::new();
    let existing = fetch_existing(&repository, &announcements).await?;

    // 第一階段：用 MOPS 的分派情形批次判定期別。
    let mut plan = build_scan_plan(&announcements, &allotments, &schedules, &existing);
    // 第二階段：剩下的逐檔問 Yahoo。
    resolve_with_yahoo(&mut plan, &schedules, &existing).await;

    apply_plan(&repository, plan).await
}

/// 把採集結果併入清單；失敗時記錄錯誤並回傳 `true`。
fn collect_or_log(
    result: Result<Vec<ExDividendAnnouncement>>,
    source: &str,
    sink: &mut Vec<ExDividendAnnouncement>,
) -> bool {
    match result {
        Ok(data) => {
            tracing::info!("{} 取得 {} 筆", source, data.len());
            sink.extend(data);
            false
        }
        Err(why) => {
            tracing::error!("取得{}失敗: {:?}", source, why);
            true
        }
    }
}

/// 以股票代號索引 MOPS 的股利分派情形。
fn index_allotments(data: Vec<DividendAllotment>) -> HashMap<String, Vec<DividendAllotment>> {
    let mut map: HashMap<String, Vec<DividendAllotment>> = HashMap::with_capacity(data.len());
    for item in data {
        map.entry(item.stock_symbol.clone()).or_default().push(item);
    }
    map
}

/// 以「股票代號 + 除權息日」索引 MoneyDJ 的日程資料。
fn index_schedules(data: Vec<DividendSchedule>) -> HashMap<(String, NaiveDate), DividendSchedule> {
    data.into_iter()
        .map(|item| ((item.stock_symbol.clone(), item.ex_date), item))
        .collect()
}

/// 載入公告涉及年度的既有股利資料，並以股票代號分組。
///
/// 除權息日的年份之外還要一併載入次年：季配、半年配跨年發放時，
/// 資料列是掛在**發放年度**底下的（12 月除息、隔年 1 月發放的事件記在隔年）。
async fn fetch_existing(
    repository: &dyn DividendRepository,
    announcements: &[ExDividendAnnouncement],
) -> Result<HashMap<String, Vec<Dividend>>> {
    let mut years: Vec<i32> = announcements
        .iter()
        .flat_map(|item| [item.ex_date.year(), item.ex_date.year() + 1])
        .collect();
    years.sort_unstable();
    years.dedup();

    let rows = repository.fetch_by_years(&years).await?;
    let mut map: HashMap<String, Vec<Dividend>> = HashMap::with_capacity(rows.len());
    for row in rows {
        map.entry(row.security_code.clone()).or_default().push(row);
    }

    Ok(map)
}

/// 依採集結果與資料庫現況產生異動計畫（第一階段）。
///
/// 這是純函式，所有判斷邏輯都在這裡，方便以組好的資料做單元測試。
fn build_scan_plan(
    announcements: &[ExDividendAnnouncement],
    allotments: &HashMap<String, Vec<DividendAllotment>>,
    schedules: &HashMap<(String, NaiveDate), DividendSchedule>,
    existing: &HashMap<String, Vec<Dividend>>,
) -> ScanPlan {
    let mut plan = ScanPlan::default();

    for announcement in announcements {
        let resolved =
            match_allotment(announcement, allotments).map(ResolvedDividend::from_allotment);
        let fallback_reason = match announcement.market {
            StockExchangeMarket::OverTheCounter => UnresolvedReason::OverTheCounterUnsupported,
            _ => UnresolvedReason::NoMatchingAllotment,
        };

        apply_event(
            &mut plan,
            announcement,
            resolved.as_ref(),
            schedules,
            existing,
            fallback_reason,
        );
    }

    plan
}

/// 把單一公告事件併入異動計畫。
///
/// 先看資料庫是否已經有這次事件；找得到就只補日期，不動金額——
/// 金額以既有採集器（Goodinfo）的拆分為準，預告表只有合計值。
/// 資料庫沒有時才需要期別，判不出來就記成待處理。
///
/// `fallback_reason` 是判不出期別時要記錄的原因，讓第一、二階段能標示不同來由。
fn apply_event(
    plan: &mut ScanPlan,
    announcement: &ExDividendAnnouncement,
    resolved: Option<&ResolvedDividend>,
    schedules: &HashMap<(String, NaiveDate), DividendSchedule>,
    existing: &HashMap<String, Vec<Dividend>>,
    fallback_reason: UnresolvedReason,
) {
    let ex_date_text = announcement.ex_date.format(DATE_FORMAT).to_string();
    let payable_date_cash = schedules
        .get(&(announcement.stock_symbol.clone(), announcement.ex_date))
        .and_then(|schedule| schedule.cash_payable_date)
        .map(|date| date.format(DATE_FORMAT).to_string());
    let rows = existing
        .get(&announcement.stock_symbol)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let paid_year = resolve_paid_year(announcement, resolved, payable_date_cash.as_deref());
    let quarter = resolved.map(|item| effective_quarter(&item.quarter, rows, paid_year));

    match match_existing_row(
        announcement,
        &ex_date_text,
        quarter.as_deref(),
        resolved,
        rows,
        paid_year,
    ) {
        RowMatch::Matched(row) => {
            // 同一批公告可能已經動過這一列（除息、除權分兩筆公告），
            // 要以計畫中的暫存值為基礎繼續套用，不能每次都從資料庫快照重來。
            let mut candidate = plan
                .updates
                .get(&row.serial)
                .cloned()
                .unwrap_or_else(|| row.clone());
            if merge_announcement_dates(
                &mut candidate,
                announcement,
                &ex_date_text,
                payable_date_cash.as_deref(),
            ) {
                plan.updates.insert(row.serial, candidate);
            }
        }
        RowMatch::Ambiguous => plan.unresolved.push(UnresolvedEvent {
            announcement: announcement.clone(),
            reason: UnresolvedReason::AmbiguousExistingRow,
        }),
        RowMatch::NotFound => {
            let (Some(resolved), Some(quarter)) = (resolved, quarter) else {
                plan.unresolved.push(UnresolvedEvent {
                    announcement: announcement.clone(),
                    reason: fallback_reason,
                });
                return;
            };

            let key = (
                announcement.stock_symbol.clone(),
                paid_year,
                quarter.clone(),
            );
            let entry = plan
                .inserts
                .entry(key)
                .or_insert_with(|| build_new_dividend(announcement, resolved, paid_year, &quarter));
            merge_announcement_dates(
                entry,
                announcement,
                &ex_date_text,
                payable_date_cash.as_deref(),
            );
        }
    }
}

/// 決定這次事件應該落在哪一個發放年度。
///
/// 順序刻意與既有回補流程一致，避免同一次配息在不同流程被寫成不同年度：
/// 1. Yahoo 已依發放年度分組，直接沿用。
/// 2. 季配／半年配以現金發放日的年份為準（跨年發放時與除權息日不同年）。
/// 3. 其餘退回除權息日的年份。
fn resolve_paid_year(
    announcement: &ExDividendAnnouncement,
    resolved: Option<&ResolvedDividend>,
    payable_date_cash: Option<&str>,
) -> i32 {
    if let Some(year) = resolved.and_then(|item| item.paid_year) {
        return year;
    }

    if resolved.is_some_and(|item| !item.quarter.is_empty())
        && let Some(year) = payable_date_cash.and_then(parse_year)
    {
        return year;
    }

    announcement.ex_date.year()
}

/// 從 `YYYY-MM-DD` 取出年份。
fn parse_year(date: &str) -> Option<i32> {
    date.split('-').next()?.parse::<i32>().ok()
}

/// 判斷指定發放年度是否為「混合配息年度」（已有季配或半年配明細）。
///
/// 這種年度的空季別資料列是 `upsert_annual_total_dividend` 聚合出來的**年度合計**，
/// 不是一次真實的配息事件。
fn is_mixed_dividend_year(rows: &[Dividend], year: i32) -> bool {
    rows.iter().any(|row| {
        row.year == year && !row.quarter.is_empty() && row.quarter != FULL_YEAR_EVENT_QUARTER
    })
}

/// 判斷某一列是否為年度合計列。
fn is_annual_total_row(row: &Dividend, rows: &[Dividend]) -> bool {
    row.quarter.is_empty() && is_mixed_dividend_year(rows, row.year)
}

/// 決定事件實際要用的季別代碼。
///
/// 來源判定為「年度」（空季別）時，若該發放年度同時有季配／半年配明細，
/// 空季別已經被年度合計列占用，全年事件必須改用 `A`，
/// 與 [`crate::infra::crawler::yahoo::dividend`] 的規則一致。
fn effective_quarter(quarter: &str, rows: &[Dividend], paid_year: i32) -> String {
    if quarter.is_empty() && is_mixed_dividend_year(rows, paid_year) {
        return FULL_YEAR_EVENT_QUARTER.to_string();
    }

    quarter.to_string()
}

/// 第二階段：對第一階段判不出期別的事件逐檔查詢 Yahoo 股利政策。
///
/// 同一檔股票只查一次（同一天可能有多筆事件），查詢之間加 1.5～3.0 秒隨機延遲，
/// 與既有的 [`super::unannounced_ex_dividend_date`] 一致，降低被 Yahoo WAF 擋下的機率。
async fn resolve_with_yahoo(
    plan: &mut ScanPlan,
    schedules: &HashMap<(String, NaiveDate), DividendSchedule>,
    existing: &HashMap<String, Vec<Dividend>>,
) {
    let pending: Vec<UnresolvedEvent> = std::mem::take(&mut plan.unresolved);
    if pending.is_empty() {
        return;
    }

    // 逐檔快取本次查詢結果，避免同一檔股票的多筆事件重複請求。
    let mut fetched: HashMap<String, Option<YahooDividend>> = HashMap::new();
    let mut lookups = 0usize;

    for event in pending {
        let symbol = event.announcement.stock_symbol.clone();

        if !event.reason.is_retryable_with_yahoo() {
            plan.unresolved.push(event);
            continue;
        }

        if !fetched.contains_key(&symbol) {
            if lookups >= YAHOO_LOOKUP_LIMIT {
                plan.unresolved.push(UnresolvedEvent {
                    reason: UnresolvedReason::YahooLookupSkipped,
                    ..event
                });
                continue;
            }

            // 第二筆以後才需要間隔；第一筆直接查，不必平白等待。
            if lookups > 0 {
                let jitter_ms = rand::rng().random_range(1500..=3000);
                tokio::time::sleep(std::time::Duration::from_millis(jitter_ms)).await;
            }
            lookups += 1;
            fetched.insert(symbol.clone(), fetch_yahoo_dividend(&symbol).await);
        }

        let Some(Some(yahoo)) = fetched.get(&symbol) else {
            plan.unresolved.push(UnresolvedEvent {
                reason: UnresolvedReason::YahooLookupFailed,
                ..event
            });
            continue;
        };

        let Some(resolved) = resolve_from_yahoo(&event.announcement, yahoo) else {
            plan.unresolved.push(UnresolvedEvent {
                reason: UnresolvedReason::NoMatchingYahooDividend,
                ..event
            });
            continue;
        };

        apply_event(
            plan,
            &event.announcement,
            Some(&resolved),
            schedules,
            existing,
            UnresolvedReason::NoMatchingYahooDividend,
        );
    }

    tracing::info!(lookups, "Yahoo 逐檔補救查詢完成");
}

/// 抓取單一股票的 Yahoo 股利政策，失敗時記錄並回傳 `None`。
async fn fetch_yahoo_dividend(stock_symbol: &str) -> Option<YahooDividend> {
    let strategy = ExponentialBackoff::from_millis(100).map(jitter).take(3);
    match Retry::start(strategy, || yahoo::dividend::visit(stock_symbol)).await {
        Ok(data) => Some(data),
        Err(why) => {
            tracing::warn!("取得 {} 的 Yahoo 股利政策失敗: {:?}", stock_symbol, why);
            None
        }
    }
}

/// 從 Yahoo 的股利政策中找出除權息日相符的那一次配息。
///
/// 用日期而不是金額比對：Yahoo 的明細本身就帶除息日與除權日，
/// 日期相符是比金額吻合更強的證據，也不會被「同年兩期同額」難倒。
///
/// **必須掃過所有發放年度分組**：Yahoo 是依發放年度分組的，12 月除息、隔年 1 月發放的
/// 事件會被放進隔年，只查除權息日那一年會整個漏掉。
fn resolve_from_yahoo(
    announcement: &ExDividendAnnouncement,
    yahoo: &YahooDividend,
) -> Option<ResolvedDividend> {
    let ex_date_text = announcement.ex_date.format(DATE_FORMAT).to_string();

    yahoo
        .dividend
        .iter()
        .flat_map(|(_, details)| details.iter())
        .find(|detail| {
            (announcement.is_cash && detail.ex_dividend_date1 == ex_date_text)
                || (announcement.is_stock && detail.ex_dividend_date2 == ex_date_text)
        })
        .map(ResolvedDividend::from_yahoo)
}

/// 從 MOPS 的分派情形中找出與這次除權息事件對應的那一筆。
///
/// 一家季配公司同一年會有多次除權息，也會有多筆分派情形，因此不能只用代號比對。
/// 這裡以「金額完全相符」作為配對條件：現金與股票股利同時吻合，且**只有一筆**吻合時
/// 才算配對成功。有兩筆以上同額的期別時無法分辨，一律視為配對失敗，交給第二階段。
fn match_allotment<'a>(
    announcement: &ExDividendAnnouncement,
    allotments: &'a HashMap<String, Vec<DividendAllotment>>,
) -> Option<&'a DividendAllotment> {
    let candidates = allotments.get(&announcement.stock_symbol)?;
    let year = announcement.ex_date.year();

    // 股利所屬年度不是除權息當年（季配、半年配），就是前一年（年配隔年發放）。
    let mut matched = candidates.iter().filter(|allotment| {
        (allotment.year_of_dividend == year || allotment.year_of_dividend == year - 1)
            && amount_matches(announcement, allotment)
    });

    let first = matched.next()?;
    if matched.next().is_some() {
        return None;
    }

    Some(first)
}

/// 判斷公告金額與分派情形是否吻合。
///
/// 公告未提供金額（ETF 的「待公告」）時無從比對，一律視為不吻合，
/// 避免把未公布的事件硬塞進某個期別。
fn amount_matches(announcement: &ExDividendAnnouncement, allotment: &DividendAllotment) -> bool {
    let cash = if announcement.is_cash {
        match announcement.cash_dividend {
            Some(value) => value,
            None => return false,
        }
    } else {
        Decimal::ZERO
    };

    let stock = if announcement.is_stock {
        match announcement.stock_dividend() {
            Some(value) => value,
            None => return false,
        }
    } else {
        Decimal::ZERO
    };

    cash == allotment.cash_dividend() && stock == allotment.stock_dividend()
}

/// 既有資料列的比對結果。
#[derive(Debug, PartialEq)]
enum RowMatch<'a> {
    /// 找到唯一對應的資料列。
    Matched(&'a Dividend),
    /// 沒有對應的資料列。
    NotFound,
    /// 有多筆可能對應的資料列，無法分辨。
    Ambiguous,
}

/// 在既有資料列中找出這次事件對應的那一列。
///
/// 三種比對方式依序嘗試，**年度合計列一律排除**——它是聚合結果而非事件，
/// 把除權息日寫上去會讓合計列看起來像一次配息。
fn match_existing_row<'a>(
    announcement: &ExDividendAnnouncement,
    ex_date_text: &str,
    quarter: Option<&str>,
    resolved: Option<&ResolvedDividend>,
    rows: &'a [Dividend],
    paid_year: i32,
) -> RowMatch<'a> {
    let candidates: Vec<&Dividend> = rows
        .iter()
        .filter(|row| !is_annual_total_row(row, rows))
        .collect();

    // 1. 除權息日已經寫在資料列上（先前已收錄過這次事件）。
    //    這一步不限定發放年度：既有資料可能把跨年事件記在另一年，
    //    以日期認人才不會又新增一筆重複的。
    if let Some(row) = candidates.iter().find(|row| {
        (announcement.is_cash && row.ex_dividend_date_cash == ex_date_text)
            || (announcement.is_stock && row.ex_dividend_date_stock == ex_date_text)
    }) {
        return RowMatch::Matched(row);
    }

    let (Some(quarter), Some(resolved)) = (quarter, resolved) else {
        return RowMatch::NotFound;
    };

    // 2. 發放年度與季別都相同。
    if let Some(row) = candidates
        .iter()
        .find(|row| row.year == paid_year && row.quarter == quarter)
    {
        return RowMatch::Matched(row);
    }

    // 3. 金額相同、且日期還沒公布的列。
    //
    // 這一步是防重複列的關鍵：Goodinfo 可能已經收錄了這次配息、但期別的認定
    // 與 MOPS／Yahoo 不同（例如一邊記成年度、一邊記成 H2），日期又還是「尚未公布」。
    // 金額吻合足以判定是同一次事件，改走更新既有列，把日期補上去。
    let mut matched = candidates.iter().filter(|row| {
        row.year == paid_year
            && is_unannounced(&row.ex_dividend_date_cash)
            && is_unannounced(&row.ex_dividend_date_stock)
            && row.cash_dividend == resolved.cash_dividend
            && row.stock_dividend == resolved.stock_dividend
    });

    match (matched.next(), matched.next()) {
        // 有兩筆以上完全同額又都沒公布日期時無從分辨，寧可不動，也不要新增。
        (Some(_), Some(_)) => RowMatch::Ambiguous,
        (Some(row), None) => RowMatch::Matched(row),
        _ => RowMatch::NotFound,
    }
}

/// 判斷日期欄位是否為「尚未公布」。
fn is_unannounced(value: &str) -> bool {
    UNANNOUNCED_DATE_VALUES.contains(&value)
}

/// 把一筆公告提供的日期套用到目標實體上，回傳是否真的有變動。
///
/// 只覆寫「這次公告確實涵蓋」的欄位：除息事件不會去動除權日，
/// MoneyDJ 沒有提供發放日時也保留原值，避免把已知資料洗成未公布。
/// 股票股利發放日目前沒有任何來源可用，一律維持原值。
fn merge_announcement_dates(
    target: &mut Dividend,
    announcement: &ExDividendAnnouncement,
    ex_date_text: &str,
    payable_date_cash: Option<&str>,
) -> bool {
    let mut changed = false;

    if announcement.is_cash && target.ex_dividend_date_cash != ex_date_text {
        target.ex_dividend_date_cash = ex_date_text.to_string();
        changed = true;
    }
    if announcement.is_stock && target.ex_dividend_date_stock != ex_date_text {
        target.ex_dividend_date_stock = ex_date_text.to_string();
        changed = true;
    }
    if let Some(date) = payable_date_cash
        && target.payable_date_cash != date
    {
        target.payable_date_cash = date.to_string();
        changed = true;
    }

    changed
}

/// 以公告與已解出的期別組出要新增的股利資料。
///
/// 金額採用期別來源（MOPS 或 Yahoo）而非預告表：前者才有拆分與完整合計。
/// 日期欄位先填「尚未公布」，再由 [`merge_announcement_dates`] 依公告覆寫。
/// 盈餘分配率留白，由既有的 [`super::payout_ratio`] 流程負責計算。
fn build_new_dividend(
    announcement: &ExDividendAnnouncement,
    resolved: &ResolvedDividend,
    paid_year: i32,
    quarter: &str,
) -> Dividend {
    let now = Local::now();

    Dividend {
        serial: 0,
        year: paid_year,
        year_of_dividend: resolved.year_of_dividend,
        quarter: quarter.to_string(),
        security_code: announcement.stock_symbol.clone(),
        earnings_cash_dividend: resolved.earnings_cash,
        capital_reserve_cash_dividend: resolved.capital_reserve_cash,
        cash_dividend: resolved.cash_dividend,
        earnings_stock_dividend: resolved.earnings_stock,
        capital_reserve_stock_dividend: resolved.capital_reserve_stock,
        stock_dividend: resolved.stock_dividend,
        sum: resolved.cash_dividend + resolved.stock_dividend,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: UNANNOUNCED_DATE.to_string(),
        ex_dividend_date_stock: UNANNOUNCED_DATE.to_string(),
        payable_date_cash: UNANNOUNCED_DATE.to_string(),
        payable_date_stock: UNANNOUNCED_DATE.to_string(),
        created_time: now,
        updated_time: now,
    }
}

/// 逐筆執行異動計畫。
///
/// 單筆失敗只記錄錯誤並繼續處理下一筆：一次掃描涵蓋全市場，
/// 不該因為某一檔的問題就讓其餘更新全部落空。
///
/// 新增分期事件後必須跟著重算年度合計與持股股利明細，
/// 與既有的 [`super::missing_or_multiple`] 流程一致；少了這兩步，
/// 年度合計會停在補進來之前的數字。
async fn apply_plan(repository: &dyn DividendRepository, plan: ScanPlan) -> Result<ScanOutcome> {
    let mut outcome = ScanOutcome {
        unresolved: plan.unresolved.len(),
        ..Default::default()
    };

    for update in plan.updates.values() {
        match repository.update_dividend_date(update).await {
            Ok(_) => outcome.updated += 1,
            Err(why) => tracing::error!(
                "更新 {} (serial {}) 的除權息日期失敗: {:?}",
                update.security_code,
                update.serial,
                why
            ),
        }
    }

    // 新增成功後要重算的目標；年度合計以 (代號, 發放年度) 為單位，持股明細以代號為單位。
    let mut annual_total_targets: HashSet<(String, i32)> = HashSet::new();
    let mut record_targets: HashSet<String> = HashSet::new();

    for dividend in plan.inserts.values() {
        match repository.save(dividend).await {
            Ok(_) => {
                outcome.inserted += 1;
                record_targets.insert(dividend.security_code.clone());
                if !dividend.quarter.is_empty() {
                    // 分期事件會改變年度合計列，記下來稍後統一重算。
                    annual_total_targets.insert((dividend.security_code.clone(), dividend.year));
                }
                tracing::info!(
                    "新增漏抓的股利事件 {} {}{} 除權息日 {}",
                    dividend.security_code,
                    dividend.year_of_dividend,
                    if dividend.quarter.is_empty() {
                        "年度".to_string()
                    } else {
                        dividend.quarter.clone()
                    },
                    dividend.ex_dividend_date_cash
                );
            }
            Err(why) => {
                tracing::error!("新增 {} 的股利資料失敗: {:?}", dividend.security_code, why)
            }
        }
    }

    for (security_code, year) in &annual_total_targets {
        if let Err(why) = repository
            .upsert_annual_total_dividend(security_code, *year)
            .await
        {
            tracing::error!(
                "重算 {} {} 年度合計股利失敗: {:?}",
                security_code,
                year,
                why
            );
        }
    }

    for security_code in &record_targets {
        if let Err(why) =
            dividend_record::backfill_received_dividend_records_for_stock(security_code).await
        {
            tracing::error!("重算 {} 的持股已領股利失敗: {:?}", security_code, why);
        }
    }

    log_unresolved(&plan.unresolved);

    Ok(outcome)
}

/// 依原因彙總無法判定期別的事件。
///
/// 兩階段都判不出來的事件才會走到這裡。逐筆最多列
/// [`UNRESOLVED_SAMPLE_LIMIT`] 筆，其餘只記總數，避免一次塞爆 log。
fn log_unresolved(events: &[UnresolvedEvent]) {
    if events.is_empty() {
        return;
    }

    let mut counts: HashMap<UnresolvedReason, usize> = HashMap::new();
    for event in events {
        *counts.entry(event.reason).or_default() += 1;
    }
    let summary: Vec<String> = counts
        .iter()
        .map(|(reason, count)| format!("{}={}", reason.as_str(), count))
        .collect();

    tracing::info!(
        total = events.len(),
        detail = summary.join("、"),
        "除權息事件無法判定期別，未寫入資料庫"
    );

    for event in events.iter().take(UNRESOLVED_SAMPLE_LIMIT) {
        tracing::warn!(
            "{} {} 無法判定期別：{}",
            event.announcement.stock_symbol,
            event.announcement.ex_date,
            event.reason.as_str()
        );
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::infra::crawler::yahoo::dividend::YahooDividendDetail;

    fn announcement(
        symbol: &str,
        ex_date: (i32, u32, u32),
        is_cash: bool,
        is_stock: bool,
        cash: Option<Decimal>,
        stock_ratio: Option<Decimal>,
    ) -> ExDividendAnnouncement {
        ExDividendAnnouncement {
            stock_symbol: symbol.to_string(),
            name: symbol.to_string(),
            ex_date: NaiveDate::from_ymd_opt(ex_date.0, ex_date.1, ex_date.2).unwrap(),
            is_cash,
            is_stock,
            cash_dividend: cash,
            stock_dividend_ratio: stock_ratio,
            market: StockExchangeMarket::Listed,
        }
    }

    fn allotment(
        symbol: &str,
        year_of_dividend: i32,
        quarter: &str,
        earnings_cash: Decimal,
        earnings_stock: Decimal,
    ) -> DividendAllotment {
        DividendAllotment {
            stock_symbol: symbol.to_string(),
            year_of_dividend,
            quarter: quarter.to_string(),
            period_start: None,
            period_end: None,
            earnings_cash,
            capital_reserve_cash: Decimal::ZERO,
            earnings_stock,
            capital_reserve_stock: Decimal::ZERO,
            progress: "董事會決議".to_string(),
        }
    }

    fn resolved(quarter: &str, cash: Decimal, paid_year: Option<i32>) -> ResolvedDividend {
        ResolvedDividend {
            year_of_dividend: 2025,
            paid_year,
            quarter: quarter.to_string(),
            cash_dividend: cash,
            stock_dividend: Decimal::ZERO,
            earnings_cash: cash,
            capital_reserve_cash: Decimal::ZERO,
            earnings_stock: Decimal::ZERO,
            capital_reserve_stock: Decimal::ZERO,
        }
    }

    fn existing_row(
        serial: i64,
        symbol: &str,
        year: i32,
        quarter: &str,
        ex_cash: &str,
        payable_cash: &str,
    ) -> Dividend {
        let created = Local.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Dividend {
            serial,
            year,
            year_of_dividend: year - 1,
            quarter: quarter.to_string(),
            security_code: symbol.to_string(),
            earnings_cash_dividend: Decimal::ZERO,
            capital_reserve_cash_dividend: Decimal::ZERO,
            cash_dividend: Decimal::ZERO,
            earnings_stock_dividend: Decimal::ZERO,
            capital_reserve_stock_dividend: Decimal::ZERO,
            stock_dividend: Decimal::ZERO,
            sum: Decimal::ZERO,
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            ex_dividend_date_cash: ex_cash.to_string(),
            ex_dividend_date_stock: UNANNOUNCED_DATE.to_string(),
            payable_date_cash: payable_cash.to_string(),
            payable_date_stock: UNANNOUNCED_DATE.to_string(),
            created_time: created,
            updated_time: created,
        }
    }

    fn yahoo_detail(
        year: i32,
        year_of_dividend: i32,
        quarter: &str,
        cash: Decimal,
        stock: Decimal,
        ex_cash: &str,
        ex_stock: &str,
    ) -> YahooDividendDetail {
        YahooDividendDetail {
            year,
            year_of_dividend,
            quarter: quarter.to_string(),
            cash_dividend: cash,
            stock_dividend: stock,
            ex_dividend_date1: ex_cash.to_string(),
            ex_dividend_date2: ex_stock.to_string(),
            payable_date1: UNANNOUNCED_DATE.to_string(),
            payable_date2: UNANNOUNCED_DATE.to_string(),
        }
    }

    fn index(rows: Vec<Dividend>) -> HashMap<String, Vec<Dividend>> {
        let mut map: HashMap<String, Vec<Dividend>> = HashMap::new();
        for row in rows {
            map.entry(row.security_code.clone()).or_default().push(row);
        }
        map
    }

    fn schedule(
        symbol: &str,
        ex_date: (i32, u32, u32),
        payable: (i32, u32, u32),
    ) -> DividendSchedule {
        DividendSchedule {
            stock_symbol: symbol.to_string(),
            name: symbol.to_string(),
            ex_date: NaiveDate::from_ymd_opt(ex_date.0, ex_date.1, ex_date.2).unwrap(),
            is_cash: true,
            is_stock: false,
            cash_dividend: None,
            stock_dividend: None,
            cash_payable_date: NaiveDate::from_ymd_opt(payable.0, payable.1, payable.2),
        }
    }

    /// 資料庫已有該次事件、但發放日還是「尚未公布」時，只補日期不新增資料列。
    #[test]
    fn test_build_scan_plan_updates_payable_date_only() {
        let announcements = vec![announcement(
            "2330",
            (2026, 9, 17),
            true,
            false,
            Some(dec!(5.0)),
            None,
        )];
        let existing = index(vec![existing_row(
            11,
            "2330",
            2026,
            "Q1",
            "2026-09-17",
            UNANNOUNCED_DATE,
        )]);
        let schedules = index_schedules(vec![schedule("2330", (2026, 9, 17), (2026, 10, 8))]);

        let plan = build_scan_plan(&announcements, &HashMap::new(), &schedules, &existing);

        assert!(plan.inserts.is_empty());
        assert!(plan.unresolved.is_empty());
        assert_eq!(plan.updates.len(), 1);
        let update = &plan.updates[&11];
        assert_eq!(update.payable_date_cash, "2026-10-08");
        // 除息事件不能把除權日蓋掉。
        assert_eq!(update.ex_dividend_date_stock, UNANNOUNCED_DATE);
    }

    /// 所有日期都已經正確時不該產生任何異動。
    #[test]
    fn test_build_scan_plan_skips_when_nothing_changed() {
        let announcements = vec![announcement(
            "2330",
            (2026, 9, 17),
            true,
            false,
            Some(dec!(5.0)),
            None,
        )];
        let existing = index(vec![existing_row(
            11,
            "2330",
            2026,
            "Q1",
            "2026-09-17",
            "2026-10-08",
        )]);
        let schedules = index_schedules(vec![schedule("2330", (2026, 9, 17), (2026, 10, 8))]);

        let plan = build_scan_plan(&announcements, &HashMap::new(), &schedules, &existing);

        assert_eq!(plan, ScanPlan::default());
    }

    /// 資料庫沒有的事件、且能從分派情形配對出期別時，要新增並帶上拆分金額。
    #[test]
    fn test_build_scan_plan_inserts_missing_event() {
        let announcements = vec![announcement(
            "1231",
            (2026, 8, 20),
            true,
            true,
            Some(dec!(1.5)),
            Some(dec!(0.1)),
        )];
        let allotments = index_allotments(vec![allotment("1231", 2025, "", dec!(1.5), dec!(1.0))]);

        let plan = build_scan_plan(
            &announcements,
            &allotments,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert!(plan.updates.is_empty());
        assert!(plan.unresolved.is_empty());
        assert_eq!(plan.inserts.len(), 1);

        let inserted = plan.inserts.values().next().unwrap();
        assert_eq!(inserted.security_code, "1231");
        // 發放年度取自除權息日，所屬年度與季別取自 MOPS。
        assert_eq!(inserted.year, 2026);
        assert_eq!(inserted.year_of_dividend, 2025);
        assert_eq!(inserted.quarter, "");
        assert_eq!(inserted.earnings_cash_dividend, dec!(1.5));
        assert_eq!(inserted.earnings_stock_dividend, dec!(1.0));
        assert_eq!(inserted.sum, dec!(2.5));
        assert_eq!(inserted.ex_dividend_date_cash, "2026-08-20");
        assert_eq!(inserted.ex_dividend_date_stock, "2026-08-20");
        assert_eq!(inserted.payable_date_cash, UNANNOUNCED_DATE);
    }

    /// 上櫃公司在第一階段一定配不到，要標成可交給 Yahoo 補救的原因。
    #[test]
    fn test_build_scan_plan_marks_over_the_counter_reason() {
        let mut ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
        ann.market = StockExchangeMarket::OverTheCounter;

        let plan = build_scan_plan(&[ann], &HashMap::new(), &HashMap::new(), &HashMap::new());

        assert_eq!(plan.unresolved.len(), 1);
        assert_eq!(
            plan.unresolved[0].reason,
            UnresolvedReason::OverTheCounterUnsupported
        );
        assert!(plan.unresolved[0].reason.is_retryable_with_yahoo());
    }

    /// 上市但配不到分派情形（例如 ETF）同樣要能交給 Yahoo 補救。
    #[test]
    fn test_build_scan_plan_reports_unresolved_without_allotment() {
        let announcements = vec![announcement(
            "00929",
            (2026, 8, 26),
            true,
            false,
            Some(dec!(0.1)),
            None,
        )];

        let plan = build_scan_plan(
            &announcements,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );

        assert!(plan.updates.is_empty());
        assert!(plan.inserts.is_empty());
        assert_eq!(plan.unresolved.len(), 1);
        assert_eq!(plan.unresolved[0].announcement.stock_symbol, "00929");
        assert_eq!(
            plan.unresolved[0].reason,
            UnresolvedReason::NoMatchingAllotment
        );
    }

    /// 同一年有兩期金額相同時無法分辨，必須放棄配對而不是隨便挑一筆。
    #[test]
    fn test_match_allotment_rejects_ambiguous_amounts() {
        let announcements = announcement("1102", (2026, 7, 1), true, false, Some(dec!(2.3)), None);
        let allotments = index_allotments(vec![
            allotment("1102", 2026, "Q1", dec!(2.3), Decimal::ZERO),
            allotment("1102", 2026, "Q2", dec!(2.3), Decimal::ZERO),
        ]);

        assert!(match_allotment(&announcements, &allotments).is_none());
    }

    /// 金額比對必須以數值為準，`5.000000` 與 `5` 是同一個金額。
    #[test]
    fn test_amount_matches_ignores_scale() {
        let ann = announcement(
            "2330",
            (2026, 9, 17),
            true,
            false,
            Some(dec!(5.000000)),
            None,
        );
        let allot = allotment("2330", 2026, "Q1", dec!(5), Decimal::ZERO);

        assert!(amount_matches(&ann, &allot));
    }

    /// 未公告金額（ETF 的「待公告」）不可配對，否則會綁到錯誤的期別。
    #[test]
    fn test_amount_matches_rejects_unannounced_cash() {
        let ann = announcement("00929", (2026, 9, 17), true, false, None, None);
        let allot = allotment("00929", 2026, "Q1", dec!(0.1), Decimal::ZERO);

        assert!(!amount_matches(&ann, &allot));
    }

    /// 配股率要換算成元之後再比對：0.1 股/股 等於 1 元。
    #[test]
    fn test_amount_matches_converts_stock_ratio() {
        let ann = announcement("1231", (2026, 8, 20), false, true, None, Some(dec!(0.1)));
        let allot = allotment("1231", 2025, "", Decimal::ZERO, dec!(1.0));

        assert!(amount_matches(&ann, &allot));
    }

    /// 公司改期時要以公告為準覆寫既有的除權息日。
    #[test]
    fn test_merge_announcement_dates_overwrites_changed_ex_date() {
        let mut row = existing_row(7, "2882", 2026, "", "2026-08-01", "2026-08-30");
        let ann = announcement("2882", (2026, 8, 15), true, false, Some(dec!(3.5)), None);

        assert!(merge_announcement_dates(&mut row, &ann, "2026-08-15", None));
        assert_eq!(row.ex_dividend_date_cash, "2026-08-15");
        // MoneyDJ 沒給發放日時保留資料庫原值，不可洗成未公布。
        assert_eq!(row.payable_date_cash, "2026-08-30");
    }

    /// Yahoo 補救：以除權息日相符找出對應配息，取它的所屬年度、季別與發放年度。
    #[test]
    fn test_resolve_from_yahoo_matches_by_ex_date() {
        let ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
        let yahoo = YahooDividend {
            stock_symbol: "6488".to_string(),
            dividend: vec![(
                2026,
                vec![
                    yahoo_detail(
                        2026,
                        2025,
                        "H1",
                        dec!(5.0),
                        Decimal::ZERO,
                        "2026-03-10",
                        UNANNOUNCED_DATE,
                    ),
                    yahoo_detail(
                        2026,
                        2025,
                        "H2",
                        dec!(10.0),
                        Decimal::ZERO,
                        "2026-08-26",
                        UNANNOUNCED_DATE,
                    ),
                ],
            )],
        };

        let item = resolve_from_yahoo(&ann, &yahoo).expect("ex-date should match");

        assert_eq!(item.quarter, "H2");
        assert_eq!(item.year_of_dividend, 2025);
        assert_eq!(item.paid_year, Some(2026));
        assert_eq!(item.cash_dividend, dec!(10.0));
        // Yahoo 沒有拆分資料，盈餘／公積欄位必須留 0，避免高估盈餘分配率。
        assert_eq!(item.earnings_cash, Decimal::ZERO);
        assert_eq!(item.capital_reserve_cash, Decimal::ZERO);
    }

    /// 跨年發放的事件被 Yahoo 歸在隔年，只查除權息日那一年會整個漏掉。
    #[test]
    fn test_resolve_from_yahoo_searches_every_paid_year() {
        let ann = announcement("2882", (2025, 12, 20), true, false, Some(dec!(3.0)), None);
        let yahoo = YahooDividend {
            stock_symbol: "2882".to_string(),
            dividend: vec![(
                2026,
                vec![yahoo_detail(
                    2026,
                    2025,
                    "Q4",
                    dec!(3.0),
                    Decimal::ZERO,
                    "2025-12-20",
                    UNANNOUNCED_DATE,
                )],
            )],
        };

        let item = resolve_from_yahoo(&ann, &yahoo).expect("cross-year event should match");
        assert_eq!(item.paid_year, Some(2026));
    }

    /// 同一年沒有除權息日相符的紀錄時不可硬湊。
    #[test]
    fn test_resolve_from_yahoo_returns_none_without_match() {
        let ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
        let yahoo = YahooDividend {
            stock_symbol: "6488".to_string(),
            dividend: vec![(
                2026,
                vec![yahoo_detail(
                    2026,
                    2025,
                    "H1",
                    dec!(5.0),
                    Decimal::ZERO,
                    "2026-03-10",
                    UNANNOUNCED_DATE,
                )],
            )],
        };

        assert!(resolve_from_yahoo(&ann, &yahoo).is_none());
    }

    /// 純除權事件要用除權日比對，不能只看除息日。
    #[test]
    fn test_resolve_from_yahoo_matches_stock_ex_date() {
        let ann = announcement("3234", (2026, 8, 26), false, true, None, Some(dec!(0.1)));
        let yahoo = YahooDividend {
            stock_symbol: "3234".to_string(),
            dividend: vec![(
                2026,
                vec![yahoo_detail(
                    2026,
                    2025,
                    "",
                    Decimal::ZERO,
                    dec!(1.0),
                    UNANNOUNCED_DATE,
                    "2026-08-26",
                )],
            )],
        };

        let item = resolve_from_yahoo(&ann, &yahoo).expect("stock ex-date should match");
        assert_eq!(item.stock_dividend, dec!(1.0));
    }

    /// 解出期別後，若資料庫已有同期別的資料列，要走更新而不是新增。
    #[test]
    fn test_apply_event_updates_when_quarter_row_exists() {
        let ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
        let existing = index(vec![existing_row(
            42,
            "6488",
            2026,
            "H2",
            UNANNOUNCED_DATE,
            UNANNOUNCED_DATE,
        )]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("H2", dec!(10.0), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingYahooDividend,
        );

        assert!(plan.inserts.is_empty());
        assert_eq!(plan.updates[&42].ex_dividend_date_cash, "2026-08-26");
    }

    /// Goodinfo 已收錄同一次配息、但期別認定不同且日期未公布時，
    /// 必須靠金額比對認出是同一筆，走更新而不是新增出重複列。
    #[test]
    fn test_apply_event_matches_by_amount_when_quarter_differs() {
        let ann = announcement("1234", (2026, 8, 20), true, false, Some(dec!(2.5)), None);
        // 資料庫那筆的季別是空字串（年度），與來源判定的 H2 不同。
        let mut row = existing_row(9, "1234", 2026, "", "-", "-");
        row.cash_dividend = dec!(2.5);
        let existing = index(vec![row]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("H2", dec!(2.5), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingAllotment,
        );

        assert!(plan.inserts.is_empty());
        assert_eq!(plan.updates[&9].ex_dividend_date_cash, "2026-08-20");
        assert_eq!(plan.updates[&9].quarter, "");
    }

    /// 兩筆同額且都未公布日期時無從分辨，寧可不動也不新增。
    #[test]
    fn test_apply_event_reports_ambiguous_rows() {
        let ann = announcement("1234", (2026, 8, 20), true, false, Some(dec!(2.5)), None);
        let mut first = existing_row(9, "1234", 2026, "Q1", "-", "-");
        first.cash_dividend = dec!(2.5);
        let mut second = existing_row(10, "1234", 2026, "Q2", "-", "-");
        second.cash_dividend = dec!(2.5);
        let existing = index(vec![first, second]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("H2", dec!(2.5), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingAllotment,
        );

        assert!(plan.updates.is_empty());
        assert!(plan.inserts.is_empty());
        assert_eq!(
            plan.unresolved[0].reason,
            UnresolvedReason::AmbiguousExistingRow
        );
    }

    /// 金額相同但日期已經公布過的列，不可被當成同一次事件覆蓋。
    #[test]
    fn test_apply_event_ignores_rows_with_announced_dates() {
        let ann = announcement("1234", (2026, 8, 20), true, false, Some(dec!(2.5)), None);
        // 這一列是同年前一次配息，日期已公布，金額剛好相同。
        let mut row = existing_row(9, "1234", 2026, "H1", "2026-03-10", "2026-04-10");
        row.cash_dividend = dec!(2.5);
        let existing = index(vec![row]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("H2", dec!(2.5), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingAllotment,
        );

        assert!(plan.updates.is_empty());
        assert_eq!(plan.inserts.len(), 1);
        assert_eq!(plan.inserts.values().next().unwrap().quarter, "H2");
    }

    /// 混合配息年度的空季別列是年度合計，不能被當成事件寫入除權息日；
    /// 全年事件必須改用 `A`。
    #[test]
    fn test_apply_event_never_touches_annual_total_row() {
        let ann = announcement("2072", (2026, 4, 10), true, false, Some(dec!(7.0833)), None);
        let existing = index(vec![
            // 年度合計列（空季別）+ 已存在的半年配明細 → 這是混合配息年度。
            existing_row(1, "2072", 2026, "", "-", "-"),
            existing_row(2, "2072", 2026, "H1", "2026-08-28", "2026-09-29"),
        ]);
        let mut plan = ScanPlan::default();

        apply_event(
            &mut plan,
            &ann,
            Some(&resolved("", dec!(7.0833), Some(2026))),
            &HashMap::new(),
            &existing,
            UnresolvedReason::NoMatchingAllotment,
        );

        // 合計列（serial 1）不得被更新。
        assert!(plan.updates.is_empty());
        assert_eq!(plan.inserts.len(), 1);
        let inserted = plan.inserts.values().next().unwrap();
        assert_eq!(inserted.quarter, FULL_YEAR_EVENT_QUARTER);
        assert_eq!(inserted.ex_dividend_date_cash, "2026-04-10");
    }

    /// 同一次配息拆成除息、除權兩筆公告時，兩筆日期都要留下來。
    #[test]
    fn test_apply_event_merges_two_announcements_into_one_update() {
        let cash_ann = announcement("1438", (2026, 9, 9), true, false, Some(dec!(0.2)), None);
        let stock_ann = announcement("1438", (2026, 9, 16), false, true, None, Some(dec!(0.08)));
        let existing = index(vec![existing_row(
            5,
            "1438",
            2026,
            "",
            UNANNOUNCED_DATE,
            UNANNOUNCED_DATE,
        )]);
        let mut plan = ScanPlan::default();

        for ann in [&cash_ann, &stock_ann] {
            apply_event(
                &mut plan,
                ann,
                Some(&resolved("", dec!(0.2), Some(2026))),
                &HashMap::new(),
                &existing,
                UnresolvedReason::NoMatchingAllotment,
            );
        }

        assert_eq!(plan.updates.len(), 1);
        let update = &plan.updates[&5];
        // 第二筆公告不可以把第一筆補好的除息日還原成未公布。
        assert_eq!(update.ex_dividend_date_cash, "2026-09-09");
        assert_eq!(update.ex_dividend_date_stock, "2026-09-16");
    }

    /// 同一次配息拆成兩筆公告、且資料庫沒有這筆時，只能新增一列。
    #[test]
    fn test_apply_event_merges_two_announcements_into_one_insert() {
        let cash_ann = announcement("1438", (2026, 9, 9), true, false, Some(dec!(0.2)), None);
        let stock_ann = announcement("1438", (2026, 9, 16), false, true, None, Some(dec!(0.08)));
        let mut plan = ScanPlan::default();

        for ann in [&cash_ann, &stock_ann] {
            apply_event(
                &mut plan,
                ann,
                Some(&resolved("", dec!(0.2), Some(2026))),
                &HashMap::new(),
                &HashMap::new(),
                UnresolvedReason::NoMatchingAllotment,
            );
        }

        assert_eq!(plan.inserts.len(), 1);
        let inserted = plan.inserts.values().next().unwrap();
        assert_eq!(inserted.ex_dividend_date_cash, "2026-09-09");
        assert_eq!(inserted.ex_dividend_date_stock, "2026-09-16");
    }

    /// 跨年發放：12 月除息、隔年 1 月發放，資料列要掛在發放年度底下。
    #[test]
    fn test_resolve_paid_year_prefers_payable_date_for_periodic_dividend() {
        let ann = announcement("2882", (2025, 12, 20), true, false, Some(dec!(3.0)), None);

        // 季配且有發放日 → 用發放日的年份。
        assert_eq!(
            resolve_paid_year(
                &ann,
                Some(&resolved("Q4", dec!(3.0), None)),
                Some("2026-01-15")
            ),
            2026
        );
        // 年配則維持除權息日的年份。
        assert_eq!(
            resolve_paid_year(
                &ann,
                Some(&resolved("", dec!(3.0), None)),
                Some("2026-01-15")
            ),
            2025
        );
        // 來源已判定發放年度時一律優先。
        assert_eq!(
            resolve_paid_year(&ann, Some(&resolved("Q4", dec!(3.0), Some(2026))), None),
            2026
        );
        // 什麼都沒有時退回除權息日的年份。
        assert_eq!(resolve_paid_year(&ann, None, None), 2025);
    }

    /// 對真實來源做一次不碰資料庫的乾跑，印出配對統計。
    ///
    /// `existing` 刻意給空的，因此每一筆公告都會走「新增或無法判定」這條路徑，
    /// 可以直接看出第一階段（MOPS）能配對出多少、剩下多少要交給第二階段。
    /// 這支測試只讀取外部資料，不寫入任何東西，也不會呼叫 Yahoo。
    #[tokio::test]
    #[ignore]
    async fn test_dry_run_without_database() {
        dotenvy::dotenv().ok();

        let (listed, otc, allotments, schedules) = tokio::join!(
            twse::ex_dividend_announcement::visit(),
            tpex::ex_dividend_announcement::visit(),
            dividend_allotment::visit(),
            dividend_schedule::visit(),
        );

        let mut announcements = listed.expect("上市預告表應可取得");
        announcements.extend(otc.expect("上櫃預告表應可取得"));
        let allotments = index_allotments(allotments.expect("股利分派情形應可取得"));
        let schedules = index_schedules(schedules.expect("股利政策表應可取得"));

        let plan = build_scan_plan(&announcements, &allotments, &schedules, &HashMap::new());

        println!(
            "公告 {} 筆／第一階段可新增 {} 筆／待 Yahoo 補救 {} 筆",
            announcements.len(),
            plan.inserts.len(),
            plan.unresolved.len()
        );
        let with_payable_date = plan
            .inserts
            .values()
            .filter(|item| item.payable_date_cash != UNANNOUNCED_DATE)
            .count();
        println!("可新增的資料中已帶現金發放日 {} 筆", with_payable_date);
        for item in plan.inserts.values().take(5) {
            println!(
                "  + {} {}{} 除息 {} 發放 {} 現金 {} 股票 {}",
                item.security_code,
                item.year_of_dividend,
                if item.quarter.is_empty() {
                    "年度"
                } else {
                    &item.quarter
                },
                item.ex_dividend_date_cash,
                item.payable_date_cash,
                item.cash_dividend,
                item.stock_dividend
            );
        }
    }

    /// 對真實的上櫃／ETF 事件實跑第二階段，確認 Yahoo 能補出期別。
    ///
    /// 只查前幾檔以免測試時間過長；同樣不寫入資料庫。
    #[tokio::test]
    #[ignore]
    async fn test_yahoo_fallback_against_live_site() {
        dotenvy::dotenv().ok();

        let otc = tpex::ex_dividend_announcement::visit()
            .await
            .expect("上櫃預告表應可取得");
        let sample: Vec<ExDividendAnnouncement> = otc.into_iter().take(3).collect();

        for announcement in sample {
            let Some(yahoo) = fetch_yahoo_dividend(&announcement.stock_symbol).await else {
                println!("{} 抓取失敗", announcement.stock_symbol);
                continue;
            };

            match resolve_from_yahoo(&announcement, &yahoo) {
                Some(item) => println!(
                    "{} {} → 發放年度 {:?} {}{} 現金 {} 股票 {}",
                    announcement.stock_symbol,
                    announcement.ex_date,
                    item.paid_year,
                    item.year_of_dividend,
                    if item.quarter.is_empty() {
                        "年度".to_string()
                    } else {
                        item.quarter.clone()
                    },
                    item.cash_dividend,
                    item.stock_dividend
                ),
                None => println!(
                    "{} {} 找不到除權息日相符的 Yahoo 紀錄",
                    announcement.stock_symbol, announcement.ex_date
                ),
            }

            tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
        }
    }
}
