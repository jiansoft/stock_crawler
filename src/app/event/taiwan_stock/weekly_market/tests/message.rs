//! 每週市場摘要整則訊息、各段降級與數字格式的測試。

use super::fakes::*;
use super::*;

/// 完整訊息：三段內容、億元換算、持股上升下降、沒有變化或只有一筆的持股不列。
#[tokio::test]
async fn run_builds_full_message() {
    let message = run(
        &quote(),
        &chip(),
        &index(),
        &FakeValuation::default(),
        &portfolio(),
        day(17),
        &holdings(),
        None,
        &[],
    )
    .await
    .expect("有內容");
    // name 由 holdings::stock_name 提供；這裡只驗證與名稱無關的段落。
    for line in [
        "每週市場摘要︰2026-10-16",
        "產業相對強度（成分股 RS 中位數，括號為與 5 個交易日前相比）",
        "• 最強：半導體業 70（+5.0）、航運業 50（-10.0）、光電業 30",
        "• 最弱：光電業 30、航運業 50（-10.0）、半導體業 70（+5.0）",
        "• 轉強：半導體業 +5.0",
        "• 轉弱：航運業 -10.0",
        "相對強度穿越（2026-10-12～2026-10-16，每檔只取最後一次；後面數字為最新分數）",
        "外資近 5 日產業淨買賣（2026-10-12～2026-10-16）",
        "• 流入：半導體業 +20.0 億、金融保險業 +12.0 億",
        "• 流出：航運業 -5.0 億",
        "持股相對強度（與 5 個交易日前相比）",
        "持股基本面警示（1 檔）",
        "大盤趨勢（加權指數 2026-10-16）",
        "• 收盤 300.00，近 5 日 +1.69%；多頭排列",
        "• 60 日均線 270.50，收盤在上方 10.91%（均線上揚），連續 241 個交易日在上方",
        "• 240 日均線 180.50，收盤在上方 66.20%（均線上揚）",
        "• 近 5 日沒有穿越 20／60／240 日均線",
    ] {
        assert!(message.contains(&esc(line)), "缺少 {line}：{message}");
    }
    assert!(!message.contains("投信"), "投信沒有資料不列：{message}");
    assert!(!message.contains("觀光業"), "少於 5 檔的產業不列");
}

/// 持股 RS 變化：依變化排序，上升與下降各列最多 3 檔，沒有變化的不列。
#[test]
fn build_message_lists_holding_moves() {
    let moves = holding_moves(&quote().rs);
    assert_eq!(
        moves,
        [
            HoldingMove {
                stock_symbol: "1101".to_owned(),
                from: 40,
                to: 40
            },
            HoldingMove {
                stock_symbol: "2317".to_owned(),
                from: 50,
                to: 59
            },
            HoldingMove {
                stock_symbol: "2330".to_owned(),
                from: 84,
                to: 79
            },
        ]
    );
    let message = build_message(
        None,
        None,
        None,
        &[],
        HoldingSection {
            moves: &moves,
            ..Default::default()
        },
        None,
        name,
    )
    .expect("有持股變化");
    assert!(message.starts_with(&esc("每週市場摘要\n")), "{message}");
    assert!(
        message.contains(&esc("• 上升：2317 鴻海 50 → 59（+9）")),
        "{message}"
    );
    assert!(
        message.contains(&esc("• 下降：2330 台積電 84 → 79（-5）")),
        "{message}"
    );
    assert!(!message.contains("1101"), "沒有變化的不列");
    assert!(build_message(None, None, None, &[], HoldingSection::default(), None, name).is_none());
}

/// 各段查詢失敗只略過該段；全部失敗或沒有資料時不發送。
#[tokio::test]
async fn run_degrades_per_section() {
    let failing_chip = FakeChip {
        fail: true,
        ..chip()
    };
    let message = run(
        &quote(),
        &failing_chip,
        &index(),
        &FakeValuation::default(),
        &portfolio(),
        day(17),
        &holdings(),
        None,
        &[],
    )
    .await
    .expect("產業與持股仍有內容");
    assert!(!message.contains("外資"), "{message}");
    assert!(message.contains(&esc("• 轉強：半導體業 +5.0")));

    let failing_quote = FakeQuote {
        fail: true,
        ..quote()
    };
    let message = run(
        &failing_quote,
        &chip(),
        &failing_index(),
        &FakeValuation::default(),
        &portfolio(),
        day(17),
        &holdings(),
        None,
        &[],
    )
    .await
    .expect("法人仍有內容");
    assert!(
        message.contains(&esc("每週市場摘要︰2026-10-16")),
        "{message}"
    );
    assert!(!message.contains("相對強度"), "{message}");

    assert!(
        run(
            &failing_quote,
            &failing_chip,
            &failing_index(),
            &FakeValuation::failing(),
            &FakePortfolio::failing(),
            day(17),
            &holdings(),
            None,
            &[],
        )
        .await
        .is_none()
    );
}

/// 估值水位接在大盤趨勢之後：最新一天在近 5 年的百分位與 5 個交易日前的本益比；讀取失敗只少這一行。
#[tokio::test]
async fn run_appends_valuation_level_to_regime() {
    // 7 個交易日：本益比 10、11、…、16，淨值比都是 1.5；最新 16 是最高（第 100 百分位），5 日前為 11。
    let days: Vec<MarketValuationDay> = (0..7)
        .map(|i| MarketValuationDay {
            date: day(8 + i),
            median_pe: Some(10.0 + f64::from(i)),
            median_pb: Some(1.5),
            pe_samples: 900,
            pb_samples: 950,
        })
        .collect();
    let valuation = FakeValuation { days, fail: false };
    let message = run(
        &quote(),
        &chip(),
        &index(),
        &valuation,
        &portfolio(),
        day(17),
        &holdings(),
        None,
        &[],
    )
    .await
    .expect("有內容");
    let line = "• 估值水位 10-14：本益比中位數 16.00 倍（近 5 年第 100 百分位，5 日前 11.00 倍）、股價淨值比中位數 1.50 倍（第 100 百分位）";
    assert!(message.contains(&esc(line)), "缺少 {line}：{message}");

    let message = run(
        &quote(),
        &chip(),
        &index(),
        &FakeValuation::failing(),
        &portfolio(),
        day(17),
        &holdings(),
        None,
        &[],
    )
    .await
    .expect("其他段仍有內容");
    assert!(!message.contains("估值水位"), "{message}");
}

/// 數字格式：分數變化與億元都帶正負號、一位小數。
#[test]
fn number_formats() {
    assert_eq!(signed(5.0), "+5.0");
    assert_eq!(signed(-2.24), "-2.2");
    assert_eq!(hundred_million(1.234e9), "+12.3 億");
    assert_eq!(hundred_million(-5e7), "-0.5 億");
}
