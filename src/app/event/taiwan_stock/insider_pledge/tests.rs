use super::message::{describe, lots};
use super::*;

fn month(month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, 1).unwrap()
}

fn row(symbol: &str, title: &str, name: &str, shares: i64, pledged: i64) -> InsiderHolding {
    InsiderHolding {
        stock_symbol: symbol.to_string(),
        month: month(8),
        title: title.to_string(),
        name: name.to_string(),
        shares,
        pledged,
        related_pledged: 0,
    }
}

fn insider(name: &str, shares: i64, pledged: i64, related_pledged: i64) -> Insider {
    Insider {
        name: name.to_string(),
        titles: vec!["董事本人".to_string()],
        shares,
        pledged,
        related_pledged,
    }
}

fn snapshot(month_of_year: u32, insiders: Vec<Insider>) -> Snapshot {
    Snapshot {
        month: month(month_of_year),
        insiders,
    }
}

/// 只留持股；同名多職稱合併、職稱不重複，股數取最大。
#[test]
fn group_snapshots_merges_titles_and_keeps_holdings_only() {
    let symbols: BTreeSet<String> = ["1303".to_string(), "2812".to_string()].into();
    let rows = vec![
        row("1303", "董事本人", "王貴雲", 10_723_271, 8_300_000),
        row("1303", "副總經理本人", "王貴雲", 10_723_271, 8_300_000),
        row("1303", "副總經理本人", "王貴雲", 10_723_271, 8_300_000),
        row("2812", "副總經理本人", "陳淑貞", 1_082_604, 0),
        row("2812", "經理本人", "陳淑貞", 427_702, 0),
        row("2330", "董事長本人", "魏哲家", 7_452_349, 1_600_000),
    ];
    let snapshots = group_snapshots(rows, &symbols);

    assert_eq!(snapshots.len(), 2);
    let nan_ya = &snapshots["1303"];
    assert_eq!(nan_ya.month, month(8));
    assert_eq!(nan_ya.insiders.len(), 1);
    assert_eq!(nan_ya.insiders[0].titles, vec!["董事本人", "副總經理本人"]);
    assert_eq!(nan_ya.insiders[0].pledged, 8_300_000);
    assert_eq!(snapshots["2812"].insiders[0].shares, 1_082_604);
}

/// 設質、關係人設質有變就列；持股要達門檻；新上任者只列設質，卸任者不列。
#[test]
fn diff_reports_pledge_and_significant_holding_changes() {
    let previous = vec![
        insider("加碼設質", 10_000_000, 1_000_000, 0),
        insider("解除設質", 5_000_000, 2_000_000, 300_000),
        insider("小幅減持", 10_000_000, 0, 0),
        insider("大幅減持", 10_000_000, 0, 0),
        insider("卸任", 1_000_000, 500_000, 0),
    ];
    let current = vec![
        insider("加碼設質", 10_000_000, 1_500_000, 0),
        insider("解除設質", 5_000_000, 0, 0),
        insider("小幅減持", 9_600_000, 0, 0),
        insider("大幅減持", 9_000_000, 0, 0),
        insider("新任有設質", 2_000_000, 800_000, 0),
        insider("新任無設質", 2_000_000, 0, 0),
    ];
    let changes = diff(&previous, &current);

    assert_eq!(
        changes,
        vec![
            Change::Pledged {
                insider: current[0].clone(),
                before: 1_000_000
            },
            Change::Pledged {
                insider: current[1].clone(),
                before: 2_000_000
            },
            Change::RelatedPledged {
                insider: current[1].clone(),
                before: 300_000
            },
            Change::Shares {
                insider: current[3].clone(),
                before: 10_000_000
            },
            Change::Pledged {
                insider: current[4].clone(),
                before: 0
            },
        ]
    );
}

/// 門檻是 100 張且 5%；前一期為 0 時只看張數。
#[test]
fn is_significant_holding_change_needs_both_thresholds() {
    assert!(is_significant_holding_change(2_000_000, 1_900_000));
    assert!(
        !is_significant_holding_change(2_000_001, 1_900_001),
        "未達 5%"
    );
    assert!(
        !is_significant_holding_change(1_000_000, 1_099_999),
        "未達 100 張"
    );
    assert!(is_significant_holding_change(0, 100_000));
    assert!(!is_significant_holding_change(0, 99_999));
}

/// 沒有前一期：列出目前有設質（含關係人）的人作為基準。
#[test]
fn compare_without_previous_lists_current_pledges() {
    let current = snapshot(
        8,
        vec![
            insider("有設質", 1_000_000, 500_000, 0),
            insider("無設質", 1_000_000, 0, 0),
            insider("關係人設質", 0, 0, 11_800_000),
        ],
    );
    let report = compare("2442", None, &current);

    assert!(report.baseline);
    assert_eq!(report.month, month(8));
    assert_eq!(
        report.changes,
        vec![
            Change::Current(current.insiders[0].clone()),
            Change::Current(current.insiders[2].clone()),
        ]
    );
}

/// 有前一期且沒有變動：沒有任何通知項目。
#[test]
fn compare_with_unchanged_previous_reports_nothing() {
    let previous = snapshot(7, vec![insider("甲", 1_000_000, 500_000, 0)]);
    let current = snapshot(8, vec![insider("甲", 1_000_000, 500_000, 0)]);
    let report = compare("2442", Some(&previous), &current);
    assert!(!report.baseline);
    assert!(report.changes.is_empty());
}

#[test]
fn lots_rounds_to_board_lots_and_keeps_odd_lots() {
    assert_eq!(lots(0), "0 張");
    assert_eq!(lots(152), "152 股");
    assert_eq!(lots(8_300_000), "8,300 張");
    assert_eq!(lots(10_723_271), "10,723 張");
    assert_eq!(lots(1_500), "2 張");
}

/// 各類變動的文字；設質比例用本期持股計算。
#[test]
fn describe_formats_each_change() {
    let mut wang = insider("王貴雲", 10_723_271, 8_300_000, 1_224_400);
    wang.titles.push("副總經理本人".to_string());
    assert_eq!(
        describe(&Change::Pledged {
            insider: wang.clone(),
            before: 8_000_000
        }),
        "王貴雲（董事本人、副總經理本人） 設質 8,000 張 → 8,300 張（佔持股 77.4%）"
    );
    assert_eq!(
        describe(&Change::RelatedPledged {
            insider: wang.clone(),
            before: 0
        }),
        "王貴雲（董事本人、副總經理本人） 關係人設質 0 張 → 1,224 張"
    );
    assert_eq!(
        describe(&Change::Shares {
            insider: wang.clone(),
            before: 12_000_000
        }),
        "王貴雲（董事本人、副總經理本人） 持股 12,000 張 → 10,723 張"
    );
    assert_eq!(
        describe(&Change::Current(wang)),
        "王貴雲（董事本人、副總經理本人） 設質 8,300 張（佔持股 77.4%）、關係人設質 1,224 張"
    );
    assert_eq!(
        describe(&Change::Pledged {
            insider: insider("解除", 0, 0, 0),
            before: 500_000
        }),
        "解除（董事本人） 設質 500 張 → 0 張"
    );
}

/// 訊息標題用最新月份；基準股票在標頭註明，特殊字元依 MarkdownV2 跳脫。
#[test]
fn build_message_groups_changes_by_stock() {
    let reports = vec![
        Report {
            stock_symbol: "1303".to_string(),
            month: month(8),
            baseline: false,
            changes: vec![Change::Pledged {
                insider: insider("王貴雲", 10_723_271, 8_300_000, 0),
                before: 8_000_000,
            }],
        },
        Report {
            stock_symbol: "6248".to_string(),
            month: month(8),
            baseline: true,
            changes: vec![Change::Current(insider(
                "台灣鋼鐵股份有限公司",
                3_204_054,
                3_204_000,
                0,
            ))],
        },
    ];
    let message = build_message(&reports, |symbol| match symbol {
        "1303" => "南亞".to_string(),
        _ => "台渡".to_string(),
    });

    assert!(message.starts_with("持股董監質押變動︰2026 年 8 月\n"));
    assert!(message.contains(
        "\n【1303 南亞】\n王貴雲（董事本人） 設質 8,000 張 → 8,300 張（佔持股 77\\.4%）\n"
    ));
    assert!(message.contains(
        "\n【6248 台渡】首次追蹤，目前設質\n台灣鋼鐵股份有限公司（董事本人） 設質 3,204 張（佔持股 100%）\n"
    ));
}

#[test]
fn snapshot_key_uses_the_symbol() {
    assert_eq!(snapshot_key("2330"), "insider_pledge:snapshot:2330");
}

/// 前一期寫入 Redis 後讀得回來；不存在時是 `None`。測試用假代號，結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn snapshot_round_trips_through_redis() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 snapshot_round_trips_through_redis：無 Redis 連線");
        return;
    }
    let symbol = "79987";
    let _ = CLIENT.delete(&snapshot_key(symbol)).await;

    assert_eq!(load_snapshot(symbol).await.expect("讀取"), None);
    let saved = snapshot(8, vec![insider("甲", 1_000_000, 500_000, 0)]);
    save_snapshot(symbol, &saved).await;
    assert_eq!(load_snapshot(symbol).await.expect("讀取"), Some(saved));

    CLIENT
        .delete(&snapshot_key(symbol))
        .await
        .expect("清除測試紀錄");
}
