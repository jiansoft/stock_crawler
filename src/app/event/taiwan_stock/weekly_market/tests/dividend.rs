//! 持股填息一段：讀取時單檔失敗略過、本週填息與未填息缺口排序、run 整合。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::super::dividend::{HoldingFill, holding_fills, lines};
use super::fakes::*;
use super::*;
use crate::domain::dividend::fill::{DailyClose, DividendFillQuery, ExRightEvent, FillOutcome};

fn outcome(ex_day: u32, fill_day: Option<u32>, pre: Decimal, latest: Decimal) -> FillOutcome {
    FillOutcome {
        event: ExRightEvent {
            ex_date: day(ex_day),
            cash_dividend: dec!(1),
            stock_dividend: Decimal::ZERO,
        },
        pre_ex_date: day(ex_day - 1),
        pre_ex_close: pre,
        fill_date: fill_day.map(day),
        trading_days_to_fill: fill_day.map(|d| d - ex_day + 1),
        trading_days_since_ex: 6,
        latest_close: latest,
        latest_date: day(16),
    }
}

fn holding(symbol: &str, outcome: FillOutcome) -> HoldingFill {
    HoldingFill {
        stock_symbol: symbol.to_owned(),
        outcome,
    }
}

/// 本週（10-10 之後）填息的依日期列出；未填息依缺口由大到小取前三；統計涵蓋全部。
#[test]
fn lines_list_recent_fills_and_largest_gaps() {
    let fills = vec![
        holding("2330", outcome(8, Some(14), dec!(100), dec!(101))),
        holding("1101", outcome(2, Some(5), dec!(30), dec!(31))),
        holding("0056", outcome(9, None, dec!(40), dec!(36))),
        holding("2317", outcome(9, None, dec!(200), dec!(190))),
        holding("2454", outcome(9, None, dec!(100), dec!(99))),
        holding("2603", outcome(9, None, dec!(50), dec!(49.9))),
    ];
    let text = lines(&fills, day(17), &name);
    assert_eq!(
        text,
        [
            String::new(),
            "持股填息（近一年除權息 6 次，已填息 2 次）".to_owned(),
            "• 本週填息：2330 台積電（10-08 除權息，第 7 個交易日）".to_owned(),
            "• 尚未填息 4 次，差最多：0056 其他 差 10.0%（10-09 除權息，已 6 個交易日）、2317 鴻海 差 5.0%（10-09 除權息，已 6 個交易日）、2454 其他 差 1.0%（10-09 除權息，已 6 個交易日）".to_owned(),
        ]
    );
    assert!(lines(&[], day(17), &name).is_empty());
    let only_old = [holding("1101", outcome(2, Some(5), dec!(30), dec!(31)))];
    assert_eq!(
        lines(&only_old, day(17), &name).len(),
        2,
        "沒有本週填息、也沒有未填息時只有標題"
    );
}

/// 2330 有一次除權息（10-13 填息），9999 查詢失敗只略過。
struct FakeFills;

#[async_trait]
impl DividendFillQuery for FakeFills {
    async fn fetch_latest_quote_date(&self, _symbol: &str) -> Result<Option<NaiveDate>> {
        unreachable!("週摘要不需要最新收盤日")
    }

    async fn fetch_ex_right_events(&self, symbol: &str) -> Result<Vec<ExRightEvent>> {
        match symbol {
            "2330" => Ok(vec![ExRightEvent {
                ex_date: day(12),
                cash_dividend: dec!(5),
                stock_dividend: Decimal::ZERO,
            }]),
            "9999" => Err(anyhow!("db down")),
            _ => Ok(Vec::new()),
        }
    }

    async fn fetch_daily_closes(
        &self,
        _symbol: &str,
        _since: NaiveDate,
    ) -> Result<Vec<DailyClose>> {
        Ok([(9, dec!(100)), (12, dec!(95)), (13, dec!(100))]
            .map(|(d, close)| DailyClose {
                date: day(d),
                close,
            })
            .to_vec())
    }
}

/// 讀取各持股的填息結果；失敗的持股略過，其餘照常。
#[tokio::test]
async fn holding_fills_skip_failed_symbols() {
    let symbols: Vec<String> = ["1101", "2330", "9999"].map(str::to_owned).to_vec();
    let fills = holding_fills(&FakeFills, &symbols, day(17)).await;
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].stock_symbol, "2330");
    assert_eq!(fills[0].outcome.fill_date, Some(day(13)));
}

/// run 把填息一段接在持股法人之後、基本面警示之前。
#[tokio::test]
async fn run_includes_dividend_fills() {
    let fills = [holding("2330", outcome(8, Some(14), dec!(100), dec!(101)))];
    let message = run(
        &quote(),
        &chip(),
        &index(),
        &FakeValuation::default(),
        &portfolio(),
        day(17),
        &holdings(),
        None,
        &fills,
        None,
        &[],
    )
    .await
    .expect("有內容");
    let header = esc("持股填息（近一年除權息 1 次，已填息 1 次）");
    assert!(message.contains(&header), "{message}");
    assert!(
        message.find(&header) < message.find(&esc("持股基本面警示")),
        "{message}"
    );
}
