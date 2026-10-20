//! 每週市場摘要各段（產業、穿越、大盤趨勢、策略）的測試。

use super::fakes::*;
use super::*;

/// 產業中位數排名、一週變化（少於 5 檔與未分類不列）、轉強轉弱。
#[tokio::test]
async fn industry_section_ranks_medians_with_weekly_change() {
    let section = industry_section(&quote(), day(17))
        .await
        .expect("成功")
        .expect("有分數");
    assert_eq!(section.data_date, day(16));
    let medians: Vec<(&str, f64, Option<f64>)> = section
        .industries
        .iter()
        .map(|item| (item.industry.as_str(), item.median, item.change))
        .collect();
    assert_eq!(
        medians,
        [
            ("半導體業", 70.0, Some(5.0)),
            ("航運業", 50.0, Some(-10.0)),
            ("光電業", 30.0, None),
        ]
    );
    assert!(
        industry_section(&FakeQuote::default(), day(17))
            .await
            .expect("成功")
            .is_none()
    );
}

/// 穿越：站上依最新分數由高到低、跌破由低到高，各列最多 3 檔；沒有最新分數的排最後；
/// 沒有股票的門檻與方向不列。
#[test]
fn crossing_lines_rank_by_latest_score() {
    let crossings = quote().crossings;
    assert_eq!(
        crossing_line(&crossings, 80, CrossDirection::Up, &name).as_deref(),
        Some("• 站上 80 分 4 檔：2317 鴻海 92、2330 台積電 85、6669 其他 81")
    );
    assert_eq!(
        crossing_line(&crossings, 50, CrossDirection::Down, &name).as_deref(),
        Some("• 跌破 50 分 2 檔：1101 其他 30、2603 其他 45")
    );
    assert!(crossing_line(&crossings, 80, CrossDirection::Down, &name).is_none());
    let unscored = [crossing("2454", 80, true, None)];
    assert_eq!(
        crossing_line(&unscored, 80, CrossDirection::Up, &name).as_deref(),
        Some("• 站上 80 分 1 檔：2454 其他")
    );

    let section = CrossingSection {
        from: day(12),
        to: day(16),
        crossings,
    };
    let message = build_message(None, None, Some(&section), &[], &[], None, name).expect("有穿越");
    assert!(
        message.starts_with(&esc("每週市場摘要︰2026-10-16\n")),
        "{message}"
    );
    assert!(
        message.contains(&esc(
            "• 站上 80 分 4 檔：2317 鴻海 92、2330 台積電 85、6669 其他 81\n• 跌破 50 分 2 檔"
        )),
        "{message}"
    );
    let empty = CrossingSection {
        crossings: Vec::new(),
        ..section
    };
    assert!(build_message(None, None, Some(&empty), &[], &[], None, name).is_none());
}

/// 大盤趨勢：跌破均線、資料不足、穿越由舊到新；沒有加權指數或查詢失敗時不列這段。
#[tokio::test]
async fn regime_section_lists_crosses() {
    // 由新到舊：最新一天跌到 100（跌破 20／60 日均線），前一天 400（站上 20／60 日均線），
    // 再之前往前每天高 1（一路下跌、都在均線下方）；只有 102 天，240 日均線資料不足。
    let mut closes = vec![100.0, 400.0];
    closes.extend((2..102u32).map(|offset| 200.0 + f64::from(offset)));
    let points: Vec<(NaiveDate, f64)> = closes
        .iter()
        .enumerate()
        .map(|(offset, close)| (day(16) - chrono::Days::new(offset as u64), *close))
        .collect();
    // 市場廣度由新到舊：10-16 站上 60 日線 40%、5 個交易日前（10-09）65%；新高新低只加總前 5 天。
    let breadth: Vec<BreadthDay> = [(16, 40), (15, 45), (14, 50), (13, 55), (12, 60), (9, 65)]
        .into_iter()
        .map(|(d, above_ma60)| BreadthDay {
            date: day(d),
            above_ma20: 30,
            below_ma20: 70,
            above_ma60,
            below_ma60: 100 - above_ma60,
            above_ma240: 0,
            below_ma240: 0,
            new_high: 3,
            new_low: if d == 9 { 100 } else { 1 },
        })
        .collect();
    let fake = FakeIndex {
        points: points.clone(),
        breadth,
        ..FakeIndex::default()
    };
    let section = regime_section(&fake, day(17))
        .await
        .expect("查詢成功")
        .expect("有加權指數");
    assert_eq!(
        section
            .crosses
            .iter()
            .map(|cross| (cross.date, cross.days, cross.above))
            .collect::<Vec<_>>(),
        [
            (day(15), 20, true),
            (day(15), 60, true),
            (day(16), 20, false),
            (day(16), 60, false)
        ]
    );
    let lines = market::lines(&section);
    assert_eq!(lines[2], "• 收盤 100.00，近 5 日 -51.22%；均線糾結");
    assert_eq!(
        lines[3],
        "• 60 日均線 231.15，收盤在下方 56.74%（均線下彎），連續 1 個交易日在下方"
    );
    assert_eq!(lines[4], "• 240 日均線資料不足");
    assert_eq!(
        lines[5],
        "• 近 5 日：10-15 站上 20 日均線、10-15 站上 60 日均線、10-16 跌破 20 日均線、10-16 跌破 60 日均線"
    );
    assert_eq!(
        lines[6],
        "• 市場廣度 10-16：站上 60 日線 40.0%（5 日前 65.0%）、20 日線 30.0%、240 日線 —；近 5 日創 52 週新高 15 家次、新低 5 家次"
    );
    assert_eq!(lines.len(), 7);

    // 廣度讀取失敗只少這一行；沒有統計也是。
    let failing_breadth = FakeIndex {
        points: points.clone(),
        breadth_fail: true,
        ..FakeIndex::default()
    };
    let section = regime_section(&failing_breadth, day(17))
        .await
        .expect("指數查詢成功")
        .expect("有加權指數");
    assert!(section.breadth.is_none());
    assert_eq!(market::lines(&section).len(), 6);

    let message = build_message(Some(&section), None, None, &[], &[], None, name).expect("有大盤");
    assert!(
        message.starts_with(&format!(
            "{}\n\n{}",
            esc("每週市場摘要︰2026-10-16"),
            esc("大盤趨勢")
        )),
        "{message}"
    );

    let empty = FakeIndex::default();
    assert!(
        regime_section(&empty, day(17))
            .await
            .expect("查詢成功")
            .is_none()
    );
    assert!(regime_section(&failing_index(), day(17)).await.is_err());
}

/// 策略一段：換股日、已過與剩下的交易日數、本期至今報酬與 0050、最好與最差的持股（沒有報酬的不列）。
#[test]
fn strategy_lines_rank_holdings_by_return() {
    use super::strategy::{StrategySection, lines};
    use crate::app::event::taiwan_stock::strategy_alert::tests::{holding, rebalance};
    use crate::app::ports::StrategyHolding;
    let with_return = |symbol: &str, ret: Option<f64>| StrategyHolding {
        return_to_date: ret,
        ..holding(symbol, None, 20.0)
    };
    let mut section = StrategySection {
        name: "動能".to_owned(),
        rebalance: crate::app::ports::StrategyRebalance {
            days_elapsed: 7,
            return_to_date: Some(3.456),
            etf_return_to_date: Some(-1.0),
            added: vec![
                with_return("A", Some(10.0)),
                with_return("B", Some(-5.0)),
                with_return("C", None),
            ],
            kept: vec![
                with_return("D", Some(2.0)),
                with_return("E", Some(1.0)),
                with_return("F", Some(-1.0)),
                StrategyHolding {
                    name: Some("台積電".to_owned()),
                    ..with_return("2330", Some(4.0))
                },
            ],
            ..rebalance()
        },
    };
    assert_eq!(
        lines(&section),
        [
            String::new(),
            "策略持股（動能）：2026-10-16 換股、已過 7 個交易日，再 13 個交易日換股".to_owned(),
            "• 持有 7 檔，本期至今 +3.46%（0050 -1.00%，未扣成本）".to_owned(),
            "• 最好：A +10.00%、2330 台積電 +4.00%、D +2.00%".to_owned(),
            "• 最差：B -5.00%、F -1.00%、E +1.00%".to_owned(),
        ]
    );

    // 有報酬的持股不多於 3 檔時只列最好；只有 1 檔多出來時最差只列 1 檔。
    section.rebalance.kept.truncate(2);
    let few = lines(&section);
    assert_eq!(few.len(), 5, "{few:?}");
    assert_eq!(few[4], "• 最差：B -5.00%");
    section.rebalance.added.truncate(1);
    let one = lines(&section);
    assert_eq!(
        one.last().map(String::as_str),
        Some("• 最好：A +10.00%、D +2.00%、E +1.00%")
    );

    // 大盤濾網空手、沒有符合條件的股票；沒有名稱時標題只寫「策略持股」。
    section.name.clear();
    section.rebalance.market_filtered = true;
    section.rebalance.etf_return_to_date = None;
    let filtered = lines(&section);
    assert_eq!(
        filtered[1],
        "策略持股：2026-10-16 換股、已過 7 個交易日，再 13 個交易日換股"
    );
    assert_eq!(filtered[2], "• 大盤濾網沒過，策略部位空手；0050 同期 —");
    section.rebalance.market_filtered = false;
    section.rebalance.added.clear();
    section.rebalance.kept.clear();
    assert!(lines(&section)[2].starts_with("• 沒有符合條件的股票"));

    // 有報酬但都查不到個股報酬時只列持有檔數。
    section.rebalance.kept = vec![with_return("G", None)];
    assert_eq!(lines(&section).len(), 3);

    // 組進訊息：放在最後一段。
    let message = build_message(None, None, None, &[], &[], Some(&section), |_| {
        String::new()
    })
    .expect("有策略段");
    assert!(
        message.contains("策略持股︰") || message.contains("策略持股："),
        "{message}"
    );
}

/// 沒有設定策略條件時不跑回測。
#[tokio::test]
async fn strategy_section_without_query_is_none() {
    use crate::app::event::taiwan_stock::strategy_alert::QUERY_ENV;
    if std::env::var(QUERY_ENV).is_ok_and(|query| !query.trim().is_empty()) {
        println!("跳過 strategy_section_without_query_is_none：環境已設定 {QUERY_ENV}");
        return;
    }
    assert!(super::strategy::strategy_section().await.is_none());
}
