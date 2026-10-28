//! 持股大戶週報：週資料整理、排序、訊息格式與去重鍵。

use async_trait::async_trait;
use rust_decimal_macros::dec;

use super::*;

fn day(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).expect("日期應合法")
}

/// 五個資料週，由新到舊：10-30、10-23、10-16、10-09、10-02。
fn five_weeks() -> Vec<NaiveDate> {
    vec![
        day(10, 30),
        day(10, 23),
        day(10, 16),
        day(10, 9),
        day(10, 2),
    ]
}

fn record(
    symbol: &str,
    date: NaiveDate,
    major_holders: i64,
    major_percent: Decimal,
    total_holders: i64,
    level_400: Option<(i64, Decimal)>,
) -> HolderDistributionRecord {
    HolderDistributionRecord {
        date,
        stock_symbol: symbol.to_string(),
        major_holders,
        major_percent,
        total_holders,
        holders_400: level_400.map(|(holders, _)| holders),
        shares_400: level_400.map(|(holders, _)| holders * 1_000_000),
        percent_400: level_400.map(|(_, percent)| percent),
    }
}

fn symbols(list: &[&str]) -> Vec<String> {
    list.iter().map(|symbol| (*symbol).to_string()).collect()
}

fn weeks() -> RecentHolderWeeks {
    RecentHolderWeeks {
        dates: five_weeks(),
        records: vec![
            // 2330：五週都有；本週 400 張也有。
            record(
                "2330",
                day(10, 30),
                1_485,
                dec!(84.77),
                3_010_913,
                Some((2_626, dec!(87.5647))),
            ),
            record(
                "2330",
                day(10, 23),
                1_480,
                dec!(84.45),
                3_047_000,
                Some((2_613, dec!(87.1883))),
            ),
            record("2330", day(10, 16), 1_478, dec!(84.40), 3_050_000, None),
            record("2330", day(10, 9), 1_470, dec!(84.10), 3_060_000, None),
            record("2330", day(10, 2), 1_460, dec!(83.67), 3_070_000, None),
            // 2884：上週有、400 張上週為 NULL，4 週前沒有。
            record(
                "2884",
                day(10, 30),
                700,
                dec!(60.0),
                800_000,
                Some((900, dec!(65.0))),
            ),
            record("2884", day(10, 23), 701, dec!(61.0), 790_000, None),
            // 6505：新持股，只有本週，也沒有 400 張。
            record("6505", day(10, 30), 10, dec!(90.0), 300_000, None),
            // 1101：上週缺，往前一筆是 10-16。
            record("1101", day(10, 30), 300, dec!(50.5), 400_000, None),
            record("1101", day(10, 16), 300, dec!(50.0), 400_000, None),
            // 9999：本週沒有資料（停牌），不列。
            record("9999", day(10, 23), 1, dec!(1.0), 1_000, None),
        ],
    }
}

/// 本週、上週與 4 週前各自對到正確的資料週；本週沒有資料的持股不列。
#[test]
fn build_rows_picks_current_previous_and_four_weeks() {
    let rows = build_rows(
        &symbols(&["2330", "2884", "6505", "1101", "9999"]),
        &weeks(),
    );
    let listed: Vec<&str> = rows.iter().map(|row| row.stock_symbol.as_str()).collect();
    assert_eq!(listed, vec!["2330", "2884", "6505", "1101"]);

    let tsmc = &rows[0];
    assert_eq!(tsmc.major_change(), Some(dec!(0.32)));
    assert_eq!(tsmc.major_change_four_weeks(), Some(dec!(1.10)));
    assert_eq!(tsmc.change_400(), Some(dec!(0.3764)));
    assert_eq!(tsmc.holders_400_change(), Some(13));

    // 上週 400 張為 NULL：千張照算，400 張不算變化。
    let ctbc = &rows[1];
    assert_eq!(ctbc.major_change(), Some(dec!(-1.0)));
    assert_eq!(ctbc.major_change_four_weeks(), None);
    assert_eq!(ctbc.change_400(), None);
    assert_eq!(ctbc.holders_400_change(), None);

    // 新持股：沒有任何變化。
    assert_eq!(rows[2].previous, None);
    assert_eq!(rows[2].major_change(), None);

    // 上週缺資料時以本週之前最近的一筆為準。
    assert_eq!(
        rows[3].previous.as_ref().map(|record| record.date),
        Some(day(10, 16))
    );
    assert_eq!(rows[3].major_change(), Some(dec!(0.5)));
}

/// 資料週不到 5 週時沒有 4 週變化；沒有任何資料週時沒有列。
#[test]
fn build_rows_without_enough_weeks() {
    let mut short = weeks();
    short.dates.truncate(3);
    let rows = build_rows(&symbols(&["2330"]), &short);
    assert_eq!(rows[0].major_change(), Some(dec!(0.32)));
    assert_eq!(rows[0].major_change_four_weeks(), None);

    assert!(build_rows(&symbols(&["2330"]), &RecentHolderWeeks::default()).is_empty());
}

/// 大戶比例增加最多的在前、減少最多的在後，沒有上週資料的排最後並依代號排序。
#[test]
fn sort_rows_by_major_change() {
    let mut rows = build_rows(&symbols(&["6505", "2884", "2330", "1101"]), &weeks());
    rows.push(Row {
        stock_symbol: "0050".to_string(),
        current: record("0050", day(10, 30), 1, dec!(1.0), 1, None),
        previous: None,
        four_weeks_ago: None,
    });
    sort_rows(&mut rows);
    let order: Vec<&str> = rows.iter().map(|row| row.stock_symbol.as_str()).collect();
    assert_eq!(order, vec!["1101", "2330", "2884", "0050", "6505"]);
}

#[test]
fn formatting_helpers() {
    assert_eq!(signed_points(dec!(0.32)), "+0.32");
    assert_eq!(signed_points(dec!(-1.5)), "-1.5");
    assert_eq!(signed_points(Decimal::ZERO), "0");
    assert_eq!(signed_count(1_200), "+1,200");
    assert_eq!(signed_count(-3), "-3");
    assert_eq!(signed_count(0), "0");
    assert_eq!(parenthesized(""), "");
    assert_eq!(parenthesized("+1"), "（+1）");
    assert_eq!(holders_change(3_047_000, 3_010_913), "（-1.2%）");
    assert_eq!(holders_change(0, 10), "");
    assert_eq!(holders_change(10, 10), "");
    assert_eq!(reported_key(day(10, 30)), "tdcc:holder_report:2026-10-30");
}

/// 千張一行附週與 4 週變化；400 張與股東人數一行，沒有 400 張時只列股東人數。
#[test]
fn build_message_lists_every_holding() {
    let mut rows = build_rows(&symbols(&["2330", "2884", "6505"]), &weeks());
    sort_rows(&mut rows);
    let message = build_message(day(10, 30), &rows, |symbol| match symbol {
        "2330" => "台積電".to_string(),
        "2884" => "玉山金".to_string(),
        _ => "台塑化".to_string(),
    });

    assert!(
        message.starts_with("持股大戶週報︰2026\\-10\\-30（集保股權分散表）\n"),
        "{message}"
    );
    assert!(
        message.contains(
            "\n2330 台積電 千張 84\\.77%（\\+0\\.32，4 週 \\+1\\.1）／1,485 人（\\+5）\n\
             \u{3000}400 張 87\\.56%（\\+0\\.38）／2,626 人（\\+13）／股東 3,010,913 人（\\-1\\.2%）\n"
        ),
        "{message}"
    );
    assert!(
        message.contains(
            "\n2884 玉山金 千張 60%（\\-1）／700 人（\\-1）\n\
             \u{3000}400 張 65%／900 人／股東 800,000 人（\\+1\\.3%）\n"
        ),
        "{message}"
    );
    assert!(
        message.ends_with("\n6505 台塑化 千張 90%／10 人\n　股東 300,000 人\n"),
        "{message}"
    );
}

/// 固定回傳的假倉儲，記下收到的參數。
struct FakeHistory {
    weeks: RecentHolderWeeks,
}

#[async_trait]
impl HolderHistoryRepository for FakeHistory {
    async fn recent_holder_weeks(
        &self,
        symbols: &[String],
        as_of: NaiveDate,
        weeks: i64,
    ) -> Result<RecentHolderWeeks> {
        assert_eq!(as_of, day(10, 31));
        assert_eq!(weeks, WEEKS);
        let mut result = self.weeks.clone();
        result
            .records
            .retain(|record| symbols.contains(&record.stock_symbol));
        Ok(result)
    }
}

/// 有資料時回傳最新資料週與排好序的列；沒有資料週或沒有持股資料時回傳 `None`。
#[tokio::test]
async fn collect_rows_from_repository() {
    let repository = FakeHistory { weeks: weeks() };
    let (date, rows) = collect_rows(&repository, &symbols(&["2884", "2330"]), day(10, 31))
        .await
        .expect("讀取週資料")
        .expect("有本週資料");
    assert_eq!(date, day(10, 30));
    let order: Vec<&str> = rows.iter().map(|row| row.stock_symbol.as_str()).collect();
    assert_eq!(order, vec!["2330", "2884"]);

    assert_eq!(
        collect_rows(&repository, &symbols(&["9999"]), day(10, 31))
            .await
            .expect("讀取週資料"),
        None
    );
    let empty = FakeHistory {
        weeks: RecentHolderWeeks::default(),
    };
    assert_eq!(
        collect_rows(&empty, &symbols(&["2330"]), day(10, 31))
            .await
            .expect("讀取週資料"),
        None
    );
}

/// 同一資料週只會標記成功一次；測試用很久以前的日期，結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn mark_reported_only_once_per_week() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 mark_reported_only_once_per_week：無 Redis 連線");
        return;
    }
    let date = NaiveDate::from_ymd_opt(1999, 1, 8).expect("日期應合法");
    let _ = CLIENT.delete(&reported_key(date)).await;
    assert!(mark_reported(date).await);
    assert!(!mark_reported(date).await);
    CLIENT
        .delete(&reported_key(date))
        .await
        .expect("清除測試紀錄");
}
