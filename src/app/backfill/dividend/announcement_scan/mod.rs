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
//!
//! ## 模組結構
//!
//! 本模組依「職責」拆成以下子模組，上面的總覽是它們共同遵循的規則：
//!
//! - [`source`]：四個批次來源的擷取與正規化，以及資料庫既有資料的載入。
//! - [`quarter`]：期別判定的資料模型與第一階段（MOPS 金額吻合配對），
//!   也是前兩個資料模型陷阱（混合配息年度、發放年度推定）的實作所在。
//! - [`yahoo_fallback`]：第二階段，對第一階段判不出期別的事件逐檔查詢 Yahoo。
//! - [`plan`]：把公告事件彙整成異動計畫，含第三個陷阱（同一次配息拆成兩筆公告）的處理。
//! - [`row`]：既有資料列的比對、公告日期的套用與新資料列的組裝。
//! - [`apply`]：把異動計畫逐筆套用到資料庫，並彙總無法判定期別的事件。

use std::collections::HashMap;

use anyhow::{Result, anyhow};

use crate::infra::{
    crawler::{moneydj::dividend_schedule, mops::dividend_allotment, tpex, twse},
    database::repository::dividend::PgDividendRepository,
};

/// 把異動計畫逐筆套用到資料庫。
mod apply;
/// 異動計畫的彙整。
mod plan;
/// 期別判定的資料模型與第一階段配對。
mod quarter;
/// 既有資料列的比對與資料列組裝。
mod row;
/// 各資料來源的擷取與正規化。
mod source;
/// 第二階段：逐檔查詢 Yahoo 補救期別。
mod yahoo_fallback;

/// 測試共用的夾具。
#[cfg(test)]
mod fixtures;

use apply::apply_plan;
use plan::build_scan_plan;
use source::{collect_or_log, fetch_existing, index_allotments, index_schedules};
use yahoo_fallback::resolve_with_yahoo;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
