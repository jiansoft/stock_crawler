//! 持股主力分點通知的單元測試。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::*;

fn broker(name: &str, net: i64, share: Decimal) -> BrokerNet {
    BrokerNet {
        name: name.to_string(),
        buy: net,
        sell: 0,
        net,
        share,
    }
}

/// 以買賣超側各一家分點組出指定主力買賣超與比重的資料。
fn flow(stock_symbol: &str, buy: (i64, Decimal), sell: (i64, Decimal)) -> BrokerFlow {
    BrokerFlow {
        stock_symbol: stock_symbol.to_string(),
        date: NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
        buyers: vec![broker("買方", buy.0, buy.1)],
        sellers: vec![broker("賣方", sell.0, sell.1)],
        buy_total: buy.0,
        sell_total: sell.0,
    }
}

/// 比重與張數都要達門檻；剛好等於門檻也算。
#[test]
fn is_notable_requires_both_share_and_lots() {
    assert!(is_notable(&flow("2330", (8_109, dec!(31.2)), (0, dec!(0)))));
    assert!(is_notable(&flow("1101", (0, dec!(0)), (100, dec!(20)))));
    assert!(
        !is_notable(&flow("2887", (6_940, dec!(19.99)), (0, dec!(0)))),
        "比重未達"
    );
    assert!(
        !is_notable(&flow("1232", (99, dec!(60)), (0, dec!(0)))),
        "張數未達"
    );
    assert!(
        !is_notable(&flow("2880", (500, dec!(25)), (500, dec!(25)))),
        "買賣相抵"
    );
}

/// 只留達門檻的股票，買超比重大的在前、賣超比重大的在最後。
#[test]
fn notable_flows_filters_and_sorts_by_share() {
    let flows = vec![
        flow("1101", (0, dec!(0)), (11_926, dec!(33.37))),
        flow("2353", (1_174, dec!(9.75)), (0, dec!(0))),
        flow("2330", (8_109, dec!(30.72)), (0, dec!(0))),
        flow("1303", (19_579, dec!(21.55)), (0, dec!(0))),
    ];
    let symbols: Vec<String> = notable_flows(flows)
        .into_iter()
        .map(|flow| flow.stock_symbol)
        .collect();
    assert_eq!(symbols, vec!["2330", "1303", "1101"]);
}

/// 最多列三家；買賣相抵為 0 的分點不列，整側沒有就寫「無」。
#[test]
fn top_brokers_lists_up_to_three_with_volume() {
    let brokers = vec![
        broker("摩根大通", 2_663, dec!(10.25)),
        broker("台灣摩根", 2_324, dec!(8.94)),
        broker("元大", 1_740, dec!(6.7)),
        broker("美商高盛", 1_417, dec!(5.45)),
    ];
    assert_eq!(
        top_brokers(&brokers),
        "摩根大通 2,663、台灣摩根 2,324、元大 1,740"
    );
    assert_eq!(top_brokers(&[broker("元大-三峽", 0, dec!(0))]), "無");
    assert_eq!(top_brokers(&[]), "無");
}

/// 訊息含日期、門檻說明與每檔的主力方向、張數、比重及前幾名分點，特殊字元依 MarkdownV2 跳脫。
#[test]
fn build_message_lists_each_stock() {
    let date = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let flows = vec![
        flow("2330", (8_109, dec!(30.72)), (0, dec!(0))),
        flow("1101", (0, dec!(0)), (11_926, dec!(33.37))),
    ];
    let history = BrokerHistory {
        days: 5,
        nets: HashMap::from([("2330".to_string(), period_net("2330", 12_500, 4, 1))]),
    };
    let message = build_message(date, &flows, 44, Some(&history), |symbol| match symbol {
        "2330" => "台積電".to_string(),
        _ => "台泥".to_string(),
    });

    assert!(message.starts_with("持股主力進出︰2026\\-10\\-05\n"));
    assert!(message.contains("主力買賣超佔成交量達 20% 的持股（今日資料 44 檔）"));
    assert!(message.contains(
        "\n2330 台積電 主力買超 8,109 張（30\\.72%）\n近 5 日主力買超 12,500 張（買超 4 日、賣超 1 日）\n買︰買方 8,109\n賣︰無\n"
    ));
    assert!(
        message.contains("\n1101 台泥 主力賣超 11,926 張（33\\.37%）\n買︰無\n賣︰賣方 11,926\n"),
        "沒有累計資料的股票不附累計行"
    );
    let without = build_message(date, &flows, 44, None, |_| String::new());
    assert!(!without.contains("近 5 日"));
}

fn period_net(symbol: &str, main_net: i64, buy_days: i64, sell_days: i64) -> BrokerPeriodNet {
    BrokerPeriodNet {
        stock_symbol: symbol.to_string(),
        main_net,
        buy_days,
        sell_days,
    }
}

/// 累計一行：買超、賣超與持平的寫法。
#[test]
fn history_line_shows_direction_and_days() {
    assert_eq!(
        history_line(5, &period_net("1301", -15_293, 1, 4)),
        "近 5 日主力賣超 15,293 張（買超 1 日、賣超 4 日）"
    );
    assert_eq!(
        history_line(3, &period_net("1301", 0, 1, 1)),
        "近 3 日主力持平（買超 1 日、賣超 1 日）"
    );
}

/// 回傳固定主力交易日與期間合計的假倉儲；`fail` 時查交易日失敗。
struct FakeBrokers {
    dates: Vec<NaiveDate>,
    fail: bool,
}

#[async_trait::async_trait]
impl HoldingBrokerQuery for FakeBrokers {
    async fn fetch_broker_dates(&self, until: NaiveDate, limit: usize) -> Result<Vec<NaiveDate>> {
        assert_eq!(limit, HISTORY_DAYS);
        if self.fail {
            anyhow::bail!("db down");
        }
        Ok(self.dates.iter().copied().filter(|d| *d <= until).collect())
    }

    async fn fetch_broker_period(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        symbols: &[String],
    ) -> Result<Vec<BrokerPeriodNet>> {
        assert_eq!(symbols, ["2330"]);
        Ok(vec![period_net("2330", (to - from).num_days(), 1, 0)])
    }
}

/// 期間從最舊的主力交易日到今天；只有今天一天或查詢失敗時不附累計。
#[tokio::test]
async fn load_history_spans_recent_broker_days() {
    let day = |d| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
    let symbols = ["2330".to_string()];
    let repository = FakeBrokers {
        dates: vec![day(20), day(17), day(16)],
        fail: false,
    };
    let history = load_history(&repository, day(20), &symbols)
        .await
        .expect("有三個交易日");
    assert_eq!(history.days, 3);
    assert_eq!(history.nets["2330"].main_net, 4, "期間 10-16～10-20");

    let only_today = FakeBrokers {
        dates: vec![day(20)],
        fail: false,
    };
    assert!(load_history(&only_today, day(20), &symbols).await.is_none());
    let failing = FakeBrokers {
        dates: Vec::new(),
        fail: true,
    };
    assert!(load_history(&failing, day(20), &symbols).await.is_none());
}
