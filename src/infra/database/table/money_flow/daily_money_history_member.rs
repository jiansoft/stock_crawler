use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction, postgres::PgQueryResult};

use crate::infra::database;

/// 每日市值垂直化總覽資料。
///
/// 一筆資料代表特定 `date`、`household_id` 與 `member_id` 的市值總額。
/// `member_id = 0` 保留為該家庭總和，其餘值對應實際會員。
#[derive(sqlx::FromRow, Debug)]
pub struct DailyMoneyHistoryMember {
    /// 交易日期。
    pub date: NaiveDate,
    /// 所屬家庭，用於區別各家庭的合計列。
    pub household_id: i64,
    /// 會員編號；0 代表該家庭總和。
    pub member_id: i64,
    /// 當日收盤市值總額。
    pub market_value: Decimal,
    /// 建立時間。
    pub created_at: DateTime<Local>,
    /// 最後更新時間。
    pub updated_at: DateTime<Local>,
}

/// 當日與前一交易日的會員市值對照資料。
#[derive(sqlx::FromRow, Debug)]
pub struct DailyMoneyHistoryMemberWithPreviousTradingDay {
    /// 當日資料日期。
    pub date: NaiveDate,
    /// 前一個交易日日期。
    pub previous_date: Option<NaiveDate>,
    /// 所屬家庭，用於區別各家庭的合計列。
    pub household_id: i64,
    /// 會員編號；0 代表該家庭總和。
    pub member_id: i64,
    /// 當日收盤市值總額。
    pub market_value: Decimal,
    /// 前一交易日收盤市值總額。
    pub previous_market_value: Decimal,
}

/// 逐家庭找最近兩個交易日，合計列必須以家庭與會員的複合鍵配對。
const PREVIOUS_TRADING_DAY_SQL: &str = r#"
WITH ranked_dates AS (
    SELECT household_id, date, ROW_NUMBER() OVER (PARTITION BY household_id ORDER BY date DESC) AS rn
    FROM (SELECT DISTINCT household_id, date FROM daily_money_history_member WHERE date <= $1) dates
),
current_rows AS (
    SELECT dmh.* FROM daily_money_history_member dmh
    JOIN ranked_dates rd ON rd.household_id = dmh.household_id AND rd.date = dmh.date AND rd.rn = 1
),
previous_rows AS (
    SELECT dmh.* FROM daily_money_history_member dmh
    JOIN ranked_dates rd ON rd.household_id = dmh.household_id AND rd.date = dmh.date AND rd.rn = 2
),
member_scope AS (
    SELECT household_id, member_id FROM current_rows
    UNION
    SELECT household_id, member_id FROM previous_rows
)
SELECT rd.date, pd.date AS previous_date, ms.household_id, ms.member_id,
       COALESCE(cr.market_value, 0) AS market_value,
       COALESCE(pr.market_value, 0) AS previous_market_value
FROM member_scope ms
JOIN ranked_dates rd ON rd.household_id = ms.household_id AND rd.rn = 1
LEFT JOIN ranked_dates pd ON pd.household_id = ms.household_id AND pd.rn = 2
LEFT JOIN current_rows cr ON ms.household_id = cr.household_id AND ms.member_id = cr.member_id
LEFT JOIN previous_rows pr ON ms.household_id = pr.household_id AND ms.member_id = pr.member_id
ORDER BY ms.household_id, ms.member_id;
"#;

impl DailyMoneyHistoryMember {
    /// 依指定日期重算並寫入每日市值垂直總覽。
    ///
    /// 設計目標：
    /// 1. 保持舊 `daily_money_history` 扁平表不變。
    /// 2. 新增可擴充的新表，避免新增會員時必須再加欄位。
    /// 3. 同一日同時寫入家庭合計 (`member_id = 0`) 與所有未刪除的會員（含已停用，
    ///    自建立當天起），以及當日前出現過持股的未刪除會員；當日無持股也保留 0 值資料列。
    ///
    /// # Errors
    /// 會員、持股或報價查詢與寫入失敗時回傳錯誤，由呼叫者決定交易回滾。
    pub async fn upsert(
        date: NaiveDate,
        tx: &mut Option<Transaction<'_, Postgres>>,
    ) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO daily_money_history_member (date, household_id, settlement_household_id, member_id, market_value)
WITH member_scope AS (
    -- 停用會員的資產仍屬家庭；建立前的既有持股也保留，刪除者不參與結算。
    SELECT m."Id" AS member_id, m.household_id
    FROM "Member" m
    WHERE m.deleted_at IS NULL AND m.activated_at IS NOT NULL
      AND ((m."CreateTime" AT TIME ZONE 'Asia/Taipei')::date <= $1
           OR EXISTS (SELECT 1 FROM stock_ownership_details od WHERE od.member_id = m."Id" AND od.date <= $1))
),
base_calc AS (
    SELECT ms.household_id, od.member_id,
           od.share_quantity * dq."ClosingPrice" AS market_value
    FROM stock_ownership_details od
    JOIN member_scope ms ON ms.member_id = od.member_id
    JOIN "DailyQuotes" dq ON od.security_code = dq.stock_symbol AND dq."Date" = $1
    WHERE od.is_sold = FALSE AND od.date <= $1
),
member_agg AS (
    SELECT ms.household_id, ms.member_id, COALESCE(SUM(bc.market_value), 0) AS market_value
    FROM member_scope ms
    LEFT JOIN base_calc bc ON bc.member_id = ms.member_id AND bc.household_id = ms.household_id
    GROUP BY ms.household_id, ms.member_id
),
final_rows AS (
    -- 以家庭列為起點，沒有持股或會員的家庭也保留零合計。
    SELECT h.id AS household_id, 0::bigint AS member_id, COALESCE(SUM(ma.market_value), 0) AS market_value
    FROM member_household h
    LEFT JOIN member_agg ma ON ma.household_id = h.id
    WHERE NOT h.registration_pending
    GROUP BY h.id
    UNION ALL
    SELECT household_id, member_id, market_value FROM member_agg
)
SELECT $1 AS date, fr.household_id, fr.household_id, fr.member_id, fr.market_value
FROM final_rows fr
ON CONFLICT (date, household_id, member_id) DO UPDATE SET
    settlement_household_id = EXCLUDED.settlement_household_id,
    market_value = EXCLUDED.market_value,
    updated_time = NOW();
"#;

        let query = sqlx::query(sql).bind(date);
        let result = match tx {
            None => query.execute(database::get_connection()).await,
            Some(t) => query.execute(&mut **t).await,
        };

        result.map_err(|why| {
            anyhow!(
                "Failed to daily_money_history_member::upsert({}) from database because {:?}",
                date,
                why
            )
        })
    }

    /// 取得指定日期與前一交易日的會員市值對照資料。
    ///
    /// 會保留 `member_id = 0` 的合計列，並同時回傳所有會員，
    /// 家庭編號會一路帶入比較與通知，不讓不同家庭的合計互相配對。
    pub async fn fetch_with_previous_trading_day(
        date: NaiveDate,
    ) -> Result<Vec<DailyMoneyHistoryMemberWithPreviousTradingDay>> {
        sqlx::query_as::<_, DailyMoneyHistoryMemberWithPreviousTradingDay>(PREVIOUS_TRADING_DAY_SQL)
            .bind(date)
            .fetch_all(database::get_connection())
            .await
            .context(format!(
                "Failed to fetch_with_previous_trading_day({}) from database",
                date
            ))
    }
}

#[cfg(test)]
mod tests;
