//! 籌碼資料的 PostgreSQL 倉儲實作。
//!
//! 對應 `chip_daily`、`holder_distribution`、`insider_holding`、`broker_flow` 四張表。
//! 每次寫入都把整批資料展開成陣列參數，以單一 `INSERT ... SELECT FROM UNNEST` 完成；
//! 衝突時只在內容有變才更新（`IS DISTINCT FROM`），重跑不會產生多餘寫入。

use std::collections::HashMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::{
    domain::chip::{
        BrokerFlowRecord, ChipRepository, HolderDistributionRecord, InsiderHoldingRecord,
        InstitutionalNet, MarginRecord,
    },
    infra::database,
};

/// 基於 PostgreSQL 的籌碼資料倉儲。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgChipRepository;

impl PgChipRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }
}

/// 重算 `$1`～`$2` 每列的連續買賣超天數（外資、投信、自營商、三者合計），只更新有變的列。
///
/// - `base`／`grouped`：方向（淨額正負號，缺值當 0）依「依日期的列號 − 依方向分組的列號」切出
///   連續段；同一檔期間內第一段的分組值必為 0。
/// - `counted`：每列在所屬連續段內是第幾天。
/// - `seed`：每檔 `$1` 之前最後一列的天數（走 `(stock_symbol, date DESC)` 索引）；第一段方向
///   相同時接續累加，因此逐日或分段重算的結果與一次重算全期相同。
const REFRESH_STREAKS_SQL: &str = r#"
WITH base AS (
    SELECT stock_symbol, "date",
           SIGN(COALESCE(foreign_net, 0))::int AS fs,
           SIGN(COALESCE(trust_net, 0))::int AS ts,
           SIGN(COALESCE(dealer_net, 0))::int AS ds,
           SIGN(COALESCE(foreign_net, 0) + COALESCE(trust_net, 0) + COALESCE(dealer_net, 0))::int AS os,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol ORDER BY "date") AS rn
    FROM chip_daily
    WHERE "date" BETWEEN $1 AND $2
),
grouped AS (
    SELECT stock_symbol, "date", fs, ts, ds, os,
           rn - ROW_NUMBER() OVER (PARTITION BY stock_symbol, fs ORDER BY "date") AS fg,
           rn - ROW_NUMBER() OVER (PARTITION BY stock_symbol, ts ORDER BY "date") AS tg,
           rn - ROW_NUMBER() OVER (PARTITION BY stock_symbol, ds ORDER BY "date") AS dg,
           rn - ROW_NUMBER() OVER (PARTITION BY stock_symbol, os ORDER BY "date") AS og
    FROM base
),
counted AS (
    SELECT stock_symbol, "date", fs, ts, ds, os, fg, tg, dg, og,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol, fs, fg ORDER BY "date") AS fl,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol, ts, tg ORDER BY "date") AS tl,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol, ds, dg ORDER BY "date") AS dl,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol, os, og ORDER BY "date") AS ol
    FROM grouped
),
seed AS (
    SELECT sym.stock_symbol,
           COALESCE(p.foreign_streak, 0) AS fp,
           COALESCE(p.trust_streak, 0) AS tp,
           COALESCE(p.dealer_streak, 0) AS dp,
           COALESCE(p.total_streak, 0) AS op
    FROM (SELECT DISTINCT stock_symbol FROM base) AS sym
    CROSS JOIN LATERAL (
        SELECT c.foreign_streak, c.trust_streak, c.dealer_streak, c.total_streak
        FROM chip_daily c
        WHERE c.stock_symbol = sym.stock_symbol AND c."date" < $1
        ORDER BY c."date" DESC
        LIMIT 1
    ) AS p
),
computed AS (
    SELECT c.stock_symbol, c."date",
           (c.fs * (c.fl + CASE WHEN c.fg = 0 AND SIGN(COALESCE(s.fp, 0)) = c.fs
                                THEN ABS(COALESCE(s.fp, 0)) ELSE 0 END))::smallint AS foreign_streak,
           (c.ts * (c.tl + CASE WHEN c.tg = 0 AND SIGN(COALESCE(s.tp, 0)) = c.ts
                                THEN ABS(COALESCE(s.tp, 0)) ELSE 0 END))::smallint AS trust_streak,
           (c.ds * (c.dl + CASE WHEN c.dg = 0 AND SIGN(COALESCE(s.dp, 0)) = c.ds
                                THEN ABS(COALESCE(s.dp, 0)) ELSE 0 END))::smallint AS dealer_streak,
           (c.os * (c.ol + CASE WHEN c.og = 0 AND SIGN(COALESCE(s.op, 0)) = c.os
                                THEN ABS(COALESCE(s.op, 0)) ELSE 0 END))::smallint AS total_streak
    FROM counted c
    LEFT JOIN seed s ON s.stock_symbol = c.stock_symbol
)
UPDATE chip_daily d
SET foreign_streak = x.foreign_streak,
    trust_streak = x.trust_streak,
    dealer_streak = x.dealer_streak,
    total_streak = x.total_streak
FROM computed x
WHERE d."date" = x."date" AND d.stock_symbol = x.stock_symbol
  AND (d.foreign_streak, d.trust_streak, d.dealer_streak, d.total_streak)
      IS DISTINCT FROM (x.foreign_streak, x.trust_streak, x.dealer_streak, x.total_streak)
"#;

/// 同一批內主鍵重複時 `ON CONFLICT DO UPDATE` 會直接報錯，先以最後一筆為準去重（保留原順序）。
fn dedup_by_key<T, K: std::hash::Hash + Eq>(items: &[T], key: impl Fn(&T) -> K) -> Vec<&T> {
    let mut last: HashMap<K, usize> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        last.insert(key(item), index);
    }
    items
        .iter()
        .enumerate()
        .filter(|(index, item)| last.get(&key(item)) == Some(index))
        .map(|(_, item)| item)
        .collect()
}

#[async_trait]
impl ChipRepository for PgChipRepository {
    async fn save_institutional(&self, date: NaiveDate, flows: &[InstitutionalNet]) -> Result<u64> {
        let flows = dedup_by_key(flows, |flow| flow.stock_symbol.clone());
        if flows.is_empty() {
            return Ok(0);
        }
        let symbols: Vec<&str> = flows.iter().map(|f| f.stock_symbol.as_str()).collect();
        let foreign: Vec<i64> = flows.iter().map(|f| f.foreign).collect();
        let trust: Vec<i64> = flows.iter().map(|f| f.trust).collect();
        let dealer: Vec<i64> = flows.iter().map(|f| f.dealer).collect();

        let sql = r#"
INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net)
SELECT $1, t.symbol, t.foreign_net, t.trust_net, t.dealer_net
FROM UNNEST($2::varchar[], $3::bigint[], $4::bigint[], $5::bigint[])
     AS t(symbol, foreign_net, trust_net, dealer_net)
ON CONFLICT ("date", stock_symbol) DO UPDATE SET
    foreign_net = EXCLUDED.foreign_net,
    trust_net = EXCLUDED.trust_net,
    dealer_net = EXCLUDED.dealer_net,
    updated_time = now()
WHERE (chip_daily.foreign_net, chip_daily.trust_net, chip_daily.dealer_net)
      IS DISTINCT FROM (EXCLUDED.foreign_net, EXCLUDED.trust_net, EXCLUDED.dealer_net)
"#;
        let result = sqlx::query(sql)
            .bind(date)
            .bind(&symbols)
            .bind(&foreign)
            .bind(&trust)
            .bind(&dealer)
            .execute(database::get_connection())
            .await
            .with_context(|| format!("Failed to save institutional flows of {date}"))?;
        Ok(result.rows_affected())
    }

    async fn save_margin(&self, date: NaiveDate, margins: &[MarginRecord]) -> Result<u64> {
        let margins = dedup_by_key(margins, |margin| margin.stock_symbol.clone());
        if margins.is_empty() {
            return Ok(0);
        }
        let symbols: Vec<&str> = margins.iter().map(|m| m.stock_symbol.as_str()).collect();
        let margin_previous: Vec<i64> = margins.iter().map(|m| m.margin_previous).collect();
        let margin_balance: Vec<i64> = margins.iter().map(|m| m.margin_balance).collect();
        let short_previous: Vec<i64> = margins.iter().map(|m| m.short_previous).collect();
        let short_balance: Vec<i64> = margins.iter().map(|m| m.short_balance).collect();

        let sql = r#"
INSERT INTO chip_daily ("date", stock_symbol, margin_previous, margin_balance, short_previous, short_balance)
SELECT $1, t.symbol, t.margin_previous, t.margin_balance, t.short_previous, t.short_balance
FROM UNNEST($2::varchar[], $3::bigint[], $4::bigint[], $5::bigint[], $6::bigint[])
     AS t(symbol, margin_previous, margin_balance, short_previous, short_balance)
ON CONFLICT ("date", stock_symbol) DO UPDATE SET
    margin_previous = EXCLUDED.margin_previous,
    margin_balance = EXCLUDED.margin_balance,
    short_previous = EXCLUDED.short_previous,
    short_balance = EXCLUDED.short_balance,
    updated_time = now()
WHERE (chip_daily.margin_previous, chip_daily.margin_balance, chip_daily.short_previous, chip_daily.short_balance)
      IS DISTINCT FROM (EXCLUDED.margin_previous, EXCLUDED.margin_balance, EXCLUDED.short_previous, EXCLUDED.short_balance)
"#;
        let result = sqlx::query(sql)
            .bind(date)
            .bind(&symbols)
            .bind(&margin_previous)
            .bind(&margin_balance)
            .bind(&short_previous)
            .bind(&short_balance)
            .execute(database::get_connection())
            .await
            .with_context(|| format!("Failed to save margin balances of {date}"))?;
        Ok(result.rows_affected())
    }

    async fn refresh_streaks(&self, from: NaiveDate, to: NaiveDate) -> Result<u64> {
        let result = sqlx::query(REFRESH_STREAKS_SQL)
            .bind(from)
            .bind(to)
            .execute(database::get_connection())
            .await
            .with_context(|| format!("Failed to refresh chip streaks of {from}~{to}"))?;
        Ok(result.rows_affected())
    }

    async fn save_holder_distributions(&self, records: &[HolderDistributionRecord]) -> Result<u64> {
        let records = dedup_by_key(records, |r| (r.date, r.stock_symbol.clone()));
        if records.is_empty() {
            return Ok(0);
        }
        let dates: Vec<NaiveDate> = records.iter().map(|r| r.date).collect();
        let symbols: Vec<&str> = records.iter().map(|r| r.stock_symbol.as_str()).collect();
        let major_holders: Vec<i64> = records.iter().map(|r| r.major_holders).collect();
        let major_percent: Vec<Decimal> = records.iter().map(|r| r.major_percent).collect();
        let total_holders: Vec<i64> = records.iter().map(|r| r.total_holders).collect();

        let sql = r#"
INSERT INTO holder_distribution ("date", stock_symbol, major_holders, major_percent, total_holders)
SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::bigint[], $4::numeric[], $5::bigint[])
ON CONFLICT ("date", stock_symbol) DO UPDATE SET
    major_holders = EXCLUDED.major_holders,
    major_percent = EXCLUDED.major_percent,
    total_holders = EXCLUDED.total_holders,
    updated_time = now()
WHERE (holder_distribution.major_holders, holder_distribution.major_percent, holder_distribution.total_holders)
      IS DISTINCT FROM (EXCLUDED.major_holders, EXCLUDED.major_percent, EXCLUDED.total_holders)
"#;
        let result = sqlx::query(sql)
            .bind(&dates)
            .bind(&symbols)
            .bind(&major_holders)
            .bind(&major_percent)
            .bind(&total_holders)
            .execute(database::get_connection())
            .await
            .context("Failed to save holder distributions")?;
        Ok(result.rows_affected())
    }

    async fn replace_insider_holdings(&self, records: &[InsiderHoldingRecord]) -> Result<u64> {
        let records = dedup_by_key(records, |r| {
            (
                r.month,
                r.stock_symbol.clone(),
                r.title.clone(),
                r.name.clone(),
            )
        });
        if records.is_empty() {
            return Ok(0);
        }
        let months: Vec<NaiveDate> = records.iter().map(|r| r.month).collect();
        let symbols: Vec<&str> = records.iter().map(|r| r.stock_symbol.as_str()).collect();
        let titles: Vec<&str> = records.iter().map(|r| r.title.as_str()).collect();
        let names: Vec<&str> = records.iter().map(|r| r.name.as_str()).collect();
        let shares: Vec<i64> = records.iter().map(|r| r.shares).collect();
        let pledged: Vec<i64> = records.iter().map(|r| r.pledged).collect();
        let related: Vec<i64> = records.iter().map(|r| r.related_pledged).collect();

        // 同月份、同股票的舊名單先刪掉不在這批裡的人（辭任、改派），再寫入這批。
        let delete = r#"
DELETE FROM insider_holding AS h
USING (SELECT DISTINCT * FROM UNNEST($1::date[], $2::varchar[]) AS t("month", stock_symbol)) AS batch
WHERE h."month" = batch."month" AND h.stock_symbol = batch.stock_symbol
  AND (h."month", h.stock_symbol, h.title, h.name) NOT IN (
      SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::varchar[], $4::varchar[])
  )
"#;
        let upsert = r#"
INSERT INTO insider_holding ("month", stock_symbol, title, name, shares, pledged, related_pledged)
SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::varchar[], $4::varchar[], $5::bigint[], $6::bigint[], $7::bigint[])
ON CONFLICT ("month", stock_symbol, title, name) DO UPDATE SET
    shares = EXCLUDED.shares,
    pledged = EXCLUDED.pledged,
    related_pledged = EXCLUDED.related_pledged,
    updated_time = now()
WHERE (insider_holding.shares, insider_holding.pledged, insider_holding.related_pledged)
      IS DISTINCT FROM (EXCLUDED.shares, EXCLUDED.pledged, EXCLUDED.related_pledged)
"#;
        let mut tx = database::get_tx().await?;
        let removed = sqlx::query(delete)
            .bind(&months)
            .bind(&symbols)
            .bind(&titles)
            .bind(&names)
            .execute(&mut *tx)
            .await
            .context("Failed to remove stale insider holdings")?
            .rows_affected();
        let written = sqlx::query(upsert)
            .bind(&months)
            .bind(&symbols)
            .bind(&titles)
            .bind(&names)
            .bind(&shares)
            .bind(&pledged)
            .bind(&related)
            .execute(&mut *tx)
            .await
            .context("Failed to save insider holdings")?
            .rows_affected();
        tx.commit()
            .await
            .context("Failed to commit insider holdings")?;
        Ok(removed + written)
    }

    async fn save_broker_flows(&self, records: &[BrokerFlowRecord]) -> Result<u64> {
        let records = dedup_by_key(records, |r| (r.date, r.stock_symbol.clone()));
        if records.is_empty() {
            return Ok(0);
        }
        let dates: Vec<NaiveDate> = records.iter().map(|r| r.date).collect();
        let symbols: Vec<&str> = records.iter().map(|r| r.stock_symbol.as_str()).collect();
        let buy_totals: Vec<i64> = records.iter().map(|r| r.buy_total).collect();
        let sell_totals: Vec<i64> = records.iter().map(|r| r.sell_total).collect();
        let main_shares: Vec<Decimal> = records.iter().map(|r| r.main_share).collect();
        let buyers = records
            .iter()
            .map(|r| serde_json::to_string(&r.buyers))
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to serialize broker buyers")?;
        let sellers = records
            .iter()
            .map(|r| serde_json::to_string(&r.sellers))
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to serialize broker sellers")?;

        let sql = r#"
INSERT INTO broker_flow ("date", stock_symbol, buy_total, sell_total, main_share, buyers, sellers)
SELECT t.d, t.symbol, t.buy_total, t.sell_total, t.main_share, t.buyers::jsonb, t.sellers::jsonb
FROM UNNEST($1::date[], $2::varchar[], $3::bigint[], $4::bigint[], $5::numeric[], $6::text[], $7::text[])
     AS t(d, symbol, buy_total, sell_total, main_share, buyers, sellers)
ON CONFLICT ("date", stock_symbol) DO UPDATE SET
    buy_total = EXCLUDED.buy_total,
    sell_total = EXCLUDED.sell_total,
    main_share = EXCLUDED.main_share,
    buyers = EXCLUDED.buyers,
    sellers = EXCLUDED.sellers,
    updated_time = now()
WHERE (broker_flow.buy_total, broker_flow.sell_total, broker_flow.main_share, broker_flow.buyers, broker_flow.sellers)
      IS DISTINCT FROM (EXCLUDED.buy_total, EXCLUDED.sell_total, EXCLUDED.main_share, EXCLUDED.buyers, EXCLUDED.sellers)
"#;
        let result = sqlx::query(sql)
            .bind(&dates)
            .bind(&symbols)
            .bind(&buy_totals)
            .bind(&sell_totals)
            .bind(&main_shares)
            .bind(&buyers)
            .bind(&sellers)
            .execute(database::get_connection())
            .await
            .context("Failed to save broker flows")?;
        Ok(result.rows_affected())
    }
}

/// 測試種入籌碼後重算全部日期的連續天數（測試庫資料量小，整段重算即可）。
#[cfg(test)]
pub(crate) async fn refresh_streaks_for_tests() {
    let from = NaiveDate::from_ymd_opt(1980, 1, 1).expect("固定日期合法");
    let to = NaiveDate::from_ymd_opt(2100, 12, 31).expect("固定日期合法");
    PgChipRepository::new()
        .refresh_streaks(from, to)
        .await
        .expect("重算籌碼連續天數");
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::domain::chip::BrokerNetRecord;

    /// 測試用的假代號；寫入後一律清除。
    const FAKE: &str = "79979CH";

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 4, 30).expect("日期應合法")
    }

    async fn cleanup() {
        for sql in [
            "DELETE FROM chip_daily WHERE stock_symbol = $1",
            "DELETE FROM holder_distribution WHERE stock_symbol = $1",
            "DELETE FROM insider_holding WHERE stock_symbol = $1",
            "DELETE FROM broker_flow WHERE stock_symbol = $1",
        ] {
            sqlx::query(sql)
                .bind(FAKE)
                .execute(database::get_connection())
                .await
                .expect("清除測試資料");
        }
    }

    /// 同一批重複的鍵只留最後一筆，順序不變。
    #[test]
    fn dedup_by_key_keeps_the_last_occurrence() {
        let items = [("a", 1), ("b", 2), ("a", 3)];
        let kept: Vec<_> = dedup_by_key(&items, |item| item.0)
            .into_iter()
            .copied()
            .collect();
        assert_eq!(kept, [("b", 2), ("a", 3)]);
    }

    /// 法人與融資融券寫進同一列、互不覆蓋；內容相同重送不算寫入；
    /// 董監名單整批取代；分點的 JSON 能寫入並比對。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn saves_every_kind_of_chip_data() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 saves_every_kind_of_chip_data：無資料庫連接");
            return;
        }
        cleanup().await;
        let repo = PgChipRepository::new();

        let flow = InstitutionalNet {
            stock_symbol: FAKE.to_string(),
            foreign: 1_000,
            trust: -200,
            dealer: 30,
        };
        let margin = MarginRecord {
            stock_symbol: FAKE.to_string(),
            margin_previous: 10,
            margin_balance: 12,
            short_previous: 3,
            short_balance: 1,
        };
        assert_eq!(
            repo.save_institutional(day(), std::slice::from_ref(&flow))
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            repo.save_margin(day(), std::slice::from_ref(&margin))
                .await
                .unwrap(),
            1
        );
        assert_eq!(repo.save_institutional(day(), &[flow]).await.unwrap(), 0);
        let row: (Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
            r#"SELECT foreign_net, trust_net, margin_balance FROM chip_daily WHERE "date" = $1 AND stock_symbol = $2"#,
        )
        .bind(day())
        .bind(FAKE)
        .fetch_one(database::get_connection())
        .await
        .unwrap();
        assert_eq!(row, (Some(1_000), Some(-200), Some(12)));

        let holder = HolderDistributionRecord {
            date: day(),
            stock_symbol: FAKE.to_string(),
            major_holders: 50,
            major_percent: dec!(75.12),
            total_holders: 30_000,
        };
        assert_eq!(
            repo.save_holder_distributions(std::slice::from_ref(&holder))
                .await
                .unwrap(),
            1
        );
        assert_eq!(repo.save_holder_distributions(&[holder]).await.unwrap(), 0);

        let month = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let insider = |name: &str, pledged| InsiderHoldingRecord {
            month,
            stock_symbol: FAKE.to_string(),
            title: "董事".to_string(),
            name: name.to_string(),
            shares: 1_000,
            pledged,
            related_pledged: 0,
        };
        assert_eq!(
            repo.replace_insider_holdings(&[insider("甲", 0), insider("乙", 0)])
                .await
                .unwrap(),
            2
        );
        // 乙辭任、甲改為設質：刪 1、改 1。
        assert_eq!(
            repo.replace_insider_holdings(&[insider("甲", 500)])
                .await
                .unwrap(),
            2
        );
        let names: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM insider_holding WHERE stock_symbol = $1 ORDER BY name",
        )
        .bind(FAKE)
        .fetch_all(database::get_connection())
        .await
        .unwrap();
        assert_eq!(names, ["甲"]);

        let broker = BrokerFlowRecord {
            date: day(),
            stock_symbol: FAKE.to_string(),
            buy_total: 120,
            sell_total: 80,
            main_share: dec!(3.5),
            buyers: vec![BrokerNetRecord {
                name: "凱基-台北".to_string(),
                buy: 150,
                sell: 30,
                net: 120,
                share: dec!(6.98),
            }],
            sellers: Vec::new(),
        };
        assert_eq!(
            repo.save_broker_flows(std::slice::from_ref(&broker))
                .await
                .unwrap(),
            1
        );
        assert_eq!(repo.save_broker_flows(&[broker]).await.unwrap(), 0);

        cleanup().await;
    }

    /// 連續天數：同方向累加、反向重來、淨額 0 或沒有法人資料（只有融資融券）為 0；
    /// 分段重算接續前一段最後一列，結果與整段重算相同；內容沒變時不更新任何列。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn refresh_streaks_counts_runs_and_resumes_from_previous_row() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!(
                "跳過 refresh_streaks_counts_runs_and_resumes_from_previous_row：無資料庫連接"
            );
            return;
        }
        cleanup().await;
        let repo = PgChipRepository::new();
        let d = |n: u32| NaiveDate::from_ymd_opt(2001, 1, n).expect("日期應合法");
        let flow = |foreign, trust, dealer| InstitutionalNet {
            stock_symbol: FAKE.to_string(),
            foreign,
            trust,
            dealer,
        };
        for (n, f, t, dl) in [
            (2, 10, -5, 0),
            (3, 10, -5, 0),
            (4, -1, 0, 0),
            (8, 5, 5, 5),
            (9, 5, 5, 0),
        ] {
            repo.save_institutional(d(n), &[flow(f, t, dl)])
                .await
                .unwrap();
        }
        // 01-05 只有融資融券，法人欄位為 NULL。
        let margin = MarginRecord {
            stock_symbol: FAKE.to_string(),
            margin_previous: 1,
            margin_balance: 1,
            short_previous: 0,
            short_balance: 0,
        };
        repo.save_margin(d(5), &[margin]).await.unwrap();

        let streaks = || async {
            sqlx::query_as::<_, (Option<i16>, Option<i16>, Option<i16>, Option<i16>)>(
                r#"SELECT foreign_streak, trust_streak, dealer_streak, total_streak
                   FROM chip_daily WHERE stock_symbol = $1 ORDER BY "date""#,
            )
            .bind(FAKE)
            .fetch_all(database::get_connection())
            .await
            .unwrap()
        };
        let expected = vec![
            (Some(1), Some(-1), Some(0), Some(1)),
            (Some(2), Some(-2), Some(0), Some(2)),
            (Some(-1), Some(0), Some(0), Some(-1)),
            (Some(0), Some(0), Some(0), Some(0)),
            (Some(1), Some(1), Some(1), Some(1)),
            (Some(2), Some(2), Some(0), Some(2)),
        ];

        assert_eq!(repo.refresh_streaks(d(1), d(31)).await.unwrap(), 6);
        assert_eq!(streaks().await, expected);
        assert_eq!(repo.refresh_streaks(d(1), d(31)).await.unwrap(), 0);

        sqlx::query(
            r#"UPDATE chip_daily SET foreign_streak = NULL, trust_streak = NULL, dealer_streak = NULL, total_streak = NULL
               WHERE stock_symbol = $1 AND "date" >= $2"#,
        )
        .bind(FAKE)
        .bind(d(3))
        .execute(database::get_connection())
        .await
        .unwrap();
        for (from, to) in [(d(3), d(3)), (d(4), d(5)), (d(6), d(9))] {
            repo.refresh_streaks(from, to).await.unwrap();
        }
        assert_eq!(streaks().await, expected);

        cleanup().await;
    }
}
