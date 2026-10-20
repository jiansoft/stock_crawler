//! 持股法人一段：只篩持股、張數四捨五入、佔量與連續天數、依金額排序與各段降級。

use super::super::holding_flow::{
    HoldingFlowSection, broker_flow_section, holding_flow_section, lines,
};
use super::fakes::*;
use super::*;
use crate::domain::portfolio::chip::BrokerPeriodNet;

/// 持股的一筆法人淨買賣：淨股數、估計金額、期間成交量與連續天數。
fn held(symbol: &str, net_shares: i64, amount: f64, volume: Option<f64>, streak: i32) -> StockFlow {
    StockFlow {
        name: format!("{symbol}名"),
        net_shares,
        volume,
        streak,
        ..flow(symbol, "半導體業", amount)
    }
}

fn period() -> FlowPeriod {
    FlowPeriod {
        data_date: day(16),
        from: day(12),
        days: 5,
    }
}

/// 只留持股；不到 0.5 張的略過；張數四捨五入；依金額由多到少。
#[test]
fn section_keeps_holdings_sorted_by_amount() {
    let rows = vec![
        held("2330", 12_345_600, 1.8e10, Some(100_000_000.0), 6),
        held("2317", -2_499_500, -5e8, Some(0.0), -3),
        held("1101", 400, 1e4, None, 1),
        held("2454", 600_400, 9e8, None, 0),
        held("9999", 99_000_000, 9e10, None, 9),
    ];
    let section = holding_flow_section("外資", period(), &rows, &holdings());
    let summary: Vec<(&str, i64)> = section
        .flows
        .iter()
        .map(|flow| (flow.stock_symbol.as_str(), flow.net_lots))
        .collect();
    assert_eq!(summary, [("2330", 12_346), ("2454", 600), ("2317", -2_500)]);
    let tsmc = &section.flows[0];
    assert!((tsmc.share.expect("有成交量") - 12.3456).abs() < 1e-9);
    assert_eq!(section.flows[2].share, None, "成交量 0 不算佔量");
}

/// 買超、賣超各列最多三檔；括號內是佔量與連續天數，都沒有時不加括號。
#[test]
fn lines_list_buys_and_sells() {
    let rows = vec![
        held("2330", 12_345_600, 1.8e10, Some(100_000_000.0), 6),
        held("2454", 600_400, 9e8, None, 0),
        held("2317", -2_499_500, -5e8, Some(50_000_000.0), -3),
    ];
    let section = holding_flow_section("投信", period(), &rows, &holdings());
    assert_eq!(
        lines(&section),
        [
            String::new(),
            "持股投信近 5 日買賣超（2026-10-12～2026-10-16）".to_owned(),
            "• 買超：2330 2330名 +12346 張（佔量 +12.3%，連買 6 日）、2454 2454名 +600 張"
                .to_owned(),
            "• 賣超：2317 2317名 -2500 張（佔量 -5.0%，連賣 3 日）".to_owned(),
        ]
    );

    let only_sells = holding_flow_section("外資", period(), &rows[2..], &holdings());
    let text = lines(&only_sells);
    assert_eq!(text.len(), 3, "沒有買超就不列買超行：{text:?}");

    let empty = HoldingFlowSection {
        label: "外資",
        period: period(),
        flows: Vec::new(),
    };
    assert!(lines(&empty).is_empty());
}

/// run 會把持股法人一段接在持股相對強度之後；籌碼讀取失敗時這段跟著略過。
#[tokio::test]
async fn run_includes_holding_flows() {
    let mut chip = chip();
    chip.flows
        .get_mut("foreign")
        .expect("有外資")
        .push(held("2330", 3_000_000, 4e9, None, 2));
    let message = run(
        &quote(),
        &chip,
        &index(),
        &FakeValuation::default(),
        &portfolio(),
        day(17),
        &holdings(),
        None,
    )
    .await
    .expect("有內容");
    let header = esc("持股外資近 5 日買賣超（2026-10-12～2026-10-16）");
    let buy = esc("• 買超：2330 2330名 +3000 張（連買 2 日）");
    assert!(message.contains(&header), "{message}");
    assert!(message.contains(&buy), "{message}");
    assert!(
        message.find(&esc("持股相對強度")) < message.find(&header),
        "持股法人接在持股相對強度之後：{message}"
    );
    assert!(
        !message.contains("持股投信"),
        "投信沒有持股資料就不列：{message}"
    );

    let failing = FakeChip { fail: true, ..chip };
    let message = run(
        &quote(),
        &failing,
        &index(),
        &FakeValuation::default(),
        &portfolio(),
        day(17),
        &holdings(),
        None,
    )
    .await
    .expect("其他段仍有內容");
    assert!(!message.contains("持股外資"), "{message}");
}

fn broker(symbol: &str, main_net: i64, buy_days: i64, sell_days: i64) -> BrokerPeriodNet {
    BrokerPeriodNet {
        stock_symbol: symbol.to_owned(),
        main_net,
        buy_days,
        sell_days,
    }
}

/// 主力一段：名稱、佔量與排序金額借用法人資料；找不到法人資料時以代號為名、不算佔量；合計 0 的略過。
#[test]
fn broker_section_borrows_volume_and_price() {
    let rows = vec![
        StockFlow {
            closing_price: Some(1000.0),
            ..held("2330", 0, 0.0, Some(50_000_000.0), 0)
        },
        StockFlow {
            closing_price: Some(100.0),
            ..held("2317", 0, 0.0, None, 0)
        },
    ];
    let nets = [
        broker("2317", 9_000, 4, 1),
        broker("2330", 2_000, 3, 2),
        broker("2454", -300, 0, 2),
        broker("1101", 0, 1, 1),
    ];
    let section = broker_flow_section(period(), &nets, &rows);
    assert_eq!(section.label, "主力");
    let order: Vec<&str> = section
        .flows
        .iter()
        .map(|flow| flow.stock_symbol.as_str())
        .collect();
    assert_eq!(
        order,
        ["2330", "2317", "2454"],
        "依金額：2330 20 億、2317 9 億"
    );
    assert_eq!(
        lines(&section),
        [
            String::new(),
            "持股主力近 5 日買賣超（2026-10-12～2026-10-16）".to_owned(),
            "• 買超：2330 2330名 +2000 張（佔量 +4.0%，買超 3 日、賣超 2 日）、2317 2317名 +9000 張（買超 4 日、賣超 1 日）".to_owned(),
            "• 賣超：2454 2454 -300 張（買超 0 日、賣超 2 日）".to_owned(),
        ]
    );
}

/// run 以法人一段的期間查主力合計，接在法人之後；查詢失敗只少這一段。
#[tokio::test]
async fn run_includes_holding_broker_flows() {
    let mut chip = chip();
    chip.flows.get_mut("foreign").expect("有外資").push(held(
        "2330",
        3_000_000,
        4e9,
        Some(30_000_000.0),
        2,
    ));
    let portfolio = FakePortfolio {
        brokers: vec![broker("2330", -1_500, 1, 3)],
        ..portfolio()
    };
    let message = run(
        &quote(),
        &chip,
        &index(),
        &FakeValuation::default(),
        &portfolio,
        day(17),
        &holdings(),
        None,
    )
    .await
    .expect("有內容");
    let header = esc("持股主力近 5 日買賣超（2026-10-12～2026-10-16）");
    let sell = esc("• 賣超：2330 2330名 -1500 張（佔量 -5.0%，買超 1 日、賣超 3 日）");
    assert!(message.contains(&header), "{message}");
    assert!(message.contains(&sell), "{message}");
    assert!(
        message.find(&esc("持股外資")) < message.find(&header),
        "主力接在法人之後：{message}"
    );

    let failing = FakePortfolio {
        brokers: vec![broker("2330", -1_500, 1, 3)],
        ..FakePortfolio::failing()
    };
    let message = run(
        &quote(),
        &chip,
        &index(),
        &FakeValuation::default(),
        &failing,
        day(17),
        &holdings(),
        None,
    )
    .await
    .expect("其他段仍有內容");
    assert!(!message.contains("持股主力"), "{message}");
    assert!(message.contains(&esc("持股外資")), "{message}");
}
