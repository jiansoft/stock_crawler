//! 換股通知訊息：結算、換股異動、現金與核心部位、對照實際持股與基本面警示。

use super::*;

/// 有上一期結算時附兩行：上一期組合／策略部位／0050，追蹤以來累積與超額；純策略只列策略與 0050，
/// 查不到的寫「查不到」、沒有超額。
#[test]
fn build_message_includes_settlement() {
    let message = build_message(&rebalance(), "", None, Some(&settlement(50)), &[]);
    let expected = [
        r"• 上一期 2026\-09\-17～2026\-10\-16 實盤（未扣成本）：組合 \-1\.09%（策略部位 \+2\.03%，0050 \-4\.21%）",
        r"• 追蹤 2026\-08\-19 起 2 期：累積 \+5\.50%，0050 \+1\.25%，超額 \+4\.25 個百分點",
        "",
    ]
    .join("\n");
    assert!(message.contains(&expected), "{message}");
    let pure = StrategySettlement {
        strategy_return: None,
        cumulative_return: None,
        ..settlement(0)
    };
    let message = build_message(&rebalance(), "", None, Some(&pure), &[]);
    assert!(
        message.contains("實盤（未扣成本）：策略 查不到，0050 \\-4\\.21%\n"),
        "{message}"
    );
    assert!(
        message.contains("累積 查不到，0050 \\+1\\.25%\n"),
        "{message}"
    );
    assert!(!message.contains("超額"), "{message}");
    assert!(!build_message(&rebalance(), "", None, None, &[]).contains("上一期"));
}

/// 新買進、續抱、賣出各一行，附換手與下次換股。
#[test]
fn build_message_lists_changes() {
    let message = build_message(&rebalance(), "動能", None, None, &[]);
    assert_eq!(
        message,
        [
            "策略換股（動能）︰2026\\-10\\-16",
            "• 持有 2 檔",
            "• 新買進（1 檔）：2330 台積電 50\\.0%",
            "• 續抱（1 檔）：2317 鴻海 50\\.0%",
            "• 賣出（1 檔）：79979 50\\.0%",
            "• 換手 25\\.0%",
            "• 下次換股：20 個交易日後",
            "",
            "＊依回測條件以今天收盤選出，實際下單在下一個交易日；過去表現不代表未來結果",
            "",
        ]
        .join("\n")
    );
}

/// 大盤濾網空手、沒有符合條件、有現金與 0050 核心部位的寫法。
#[test]
fn build_message_describes_cash_and_core() {
    let filtered = StrategyRebalance {
        market_filtered: true,
        added: Vec::new(),
        kept: Vec::new(),
        etf_weight: 50,
        ..rebalance()
    };
    let message = build_message(&filtered, "", None, None, &[]);
    assert!(
        message.starts_with("策略換股︰2026\\-10\\-16\n"),
        "{message}"
    );
    assert!(
        message.contains("大盤濾網沒過，策略部位全部轉為現金"),
        "{message}"
    );
    assert!(message.contains("0050 核心部位 50%"), "{message}");
    assert!(!message.contains("新買進"), "{message}");

    let empty = StrategyRebalance {
        added: Vec::new(),
        kept: Vec::new(),
        ..rebalance()
    };
    assert!(build_message(&empty, "", None, None, &[]).contains("沒有符合條件的股票"));

    let capped = StrategyRebalance {
        cash: 40.0,
        ..rebalance()
    };
    assert!(
        build_message(&capped, "", None, None, &[]).contains("策略部位現金 40\\.0%"),
        "{}",
        build_message(&capped, "", None, None, &[])
    );
}

/// 對照實際持股：已持有與尚未持有、策略賣出但仍持有、0050 核心部位；讀不到持股時不列。
#[test]
fn build_message_compares_with_actual_holdings() {
    let held: BTreeSet<String> = ["2317", "79979", "0050"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let message = build_message(&rebalance(), "", Some(&held), None, &[]);
    assert!(
        message.contains(
            "• 對照實際持股：策略 2 檔中已持有 1 檔；尚未持有：2330 台積電
"
        ),
        "{message}"
    );
    assert!(
        message.contains(
            "• 策略賣出但仍持有：79979
"
        ),
        "{message}"
    );
    assert!(!message.contains("0050 核心部位："), "{message}");

    let core = StrategyRebalance {
        etf_weight: 50,
        exited: Vec::new(),
        ..rebalance()
    };
    let message = build_message(&core, "", Some(&held), None, &[]);
    assert!(message.contains("• 0050 核心部位：已持有"), "{message}");
    assert!(!message.contains("仍持有"), "{message}");
    let all: BTreeSet<String> = ["2330", "2317"].into_iter().map(str::to_owned).collect();
    let message = build_message(&core, "", Some(&all), None, &[]);
    assert!(
        message.contains(
            "已持有 2 檔
"
        ),
        "{message}"
    );
    assert!(message.contains("• 0050 核心部位：尚未持有"), "{message}");

    let filtered = StrategyRebalance {
        market_filtered: true,
        added: Vec::new(),
        kept: Vec::new(),
        ..rebalance()
    };
    let message = build_message(&filtered, "", Some(&held), None, &[]);
    assert!(!message.contains("對照實際持股"), "{message}");
    assert!(message.contains("策略賣出但仍持有：79979"), "{message}");
    assert!(!build_message(&rebalance(), "", None, None, &[]).contains("對照實際持股"));
}

/// 策略持股有基本面警示時列一行（新買進在前），附名稱與警示；賣出的股票與沒有警示的不列。
#[test]
fn build_message_lists_fundamental_warnings() {
    let warned = |symbol: &str, signals: FundamentalSignals| HoldingFundamentals {
        stock_symbol: symbol.to_owned(),
        signals,
        ..Default::default()
    };
    let fundamentals = [
        warned(
            "2317",
            FundamentalSignals {
                revenue_decline_months: Some(4),
                operating_margin: Some(-1.0),
                ..Default::default()
            },
        ),
        warned(
            "2330",
            FundamentalSignals {
                margin_changes: [Some(-1.0), Some(-1.0), Some(-1.0)],
                ..Default::default()
            },
        ),
        warned(
            "79979",
            FundamentalSignals {
                operating_margin: Some(-5.0),
                ..Default::default()
            },
        ),
    ];
    let message = build_message(&rebalance(), "", None, None, &fundamentals);
    assert!(
        message.contains(&crate::core::util::text::escape_markdown_v2(
            "• 基本面警示：2330 台積電（三率三降）、2317 鴻海（營收連續年減、營業虧損）"
        )),
        "{message}"
    );
    assert!(!build_message(&rebalance(), "", None, None, &[]).contains("基本面警示"));
}
