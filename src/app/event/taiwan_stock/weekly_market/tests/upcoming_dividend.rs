//! 下週持股除權息一段：期間計算、附填息文字、沒有事件不列、文字與合計、本月入帳對照、run 整合。
//!
//! 逐日查詢、只留持股與金額換算由 `app::calculation::upcoming_dividends` 的測試驗證。

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::super::upcoming_dividend::{
    MonthIncome, UpcomingLine, UpcomingSection, lines, load_upcoming, month_income, next_week,
    next_week_payouts, upcoming_section,
};
use super::fakes::*;
use super::*;
use crate::app::calculation::dividend_fill_hints::FillHints;
use crate::app::calculation::upcoming_dividends::UpcomingDividend;
use crate::domain::dividend::fill::FillSummary;
use crate::domain::portfolio::payout::{DividendPayout, PayoutAllocation};
use crate::domain::portfolio::test_double::FakePayoutRepository;

fn dividend(symbol: &str, ex_day: u32, cash: Decimal, stock: Decimal) -> UpcomingDividend {
    UpcomingDividend {
        ex_date: day(ex_day),
        stock_symbol: symbol.to_owned(),
        name: format!("名{symbol}"),
        cash_dividend: cash,
        stock_dividend: stock,
        closing_price: dec!(100),
        shares: 1500,
    }
}

/// 週六、週日、週一執行都取「接下來的」週一到週五。
#[test]
fn next_week_starts_on_the_coming_monday() {
    assert_eq!(next_week(day(17)), (day(19), day(23)), "週六");
    assert_eq!(next_week(day(18)), (day(19), day(23)), "週日");
    assert_eq!(next_week(day(19)), (day(26), day(30)), "週一");
}

/// 每檔附填息文字（有自身紀錄的才有）；期間為下週；沒有事件回 None。
#[test]
fn upcoming_section_attaches_fill_hints() {
    let hints = FillHints::for_test(
        HashMap::from([(
            "2330".to_owned(),
            FillSummary {
                events: 5,
                filled: 4,
                average_trading_days: Some(3.0),
                median_trading_days: Some(2.0),
            },
        )]),
        Vec::new(),
    );
    let section = upcoming_section(
        vec![
            dividend("2330", 20, dec!(5), Decimal::ZERO),
            dividend("2412", 21, dec!(1), Decimal::ZERO),
        ],
        &hints,
        day(17),
        Vec::new(),
        None,
    )
    .expect("有持股除權息");
    assert_eq!((section.from, section.to), (day(19), day(23)));
    assert_eq!(
        section
            .items
            .iter()
            .map(|item| item.fill.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("近 5 年填息 4／5 次、平均 3 日"), None]
    );
    assert!(upcoming_section(Vec::new(), &hints, day(17), Vec::new(), None).is_none());
    let only_payouts =
        upcoming_section(Vec::new(), &hints, day(17), vec![payout("0056", 20)], None)
            .expect("只有入帳也要列");
    assert!(only_payouts.items.is_empty());
    let only_month = upcoming_section(Vec::new(), &hints, day(17), Vec::new(), Some(october()))
        .expect("只有本月入帳也要列");
    assert_eq!(only_month.month, Some(october()));
}

/// 每檔一行：日期、代號、股數、除息與除權、填息；兩檔以上才附現金合計。
#[test]
fn lines_show_amounts_and_total() {
    let line = |dividend: UpcomingDividend, fill: Option<&str>| UpcomingLine {
        dividend,
        fill: fill.map(str::to_owned),
    };
    let section = UpcomingSection {
        from: day(19),
        to: day(23),
        items: vec![
            line(
                dividend("2330", 20, dec!(5), Decimal::ZERO),
                Some("近 5 年填息 4／5 次"),
            ),
            line(dividend("2412", 20, dec!(1.5), dec!(0.5)), None),
        ],
        payouts: Vec::new(),
        month: None,
    };
    assert_eq!(
        lines(&section),
        vec![
            String::new(),
            "下週持股除權息（10-19～10-23）".to_owned(),
            "• 10-20 2330 名2330（1,500 股）除息 5 元、預估入帳 7,500 元；近 5 年填息 4／5 次"
                .to_owned(),
            "• 10-20 2412 名2412（1,500 股）除息 1.5 元、預估入帳 2,250 元；除權 0.5 元、預估配股 75 股"
                .to_owned(),
            "• 合計預估現金入帳 9,750 元（未扣二代健保與稅）".to_owned(),
        ]
    );
    let single = UpcomingSection {
        items: vec![line(dividend("2330", 20, dec!(5), Decimal::ZERO), None)],
        ..section
    };
    assert_eq!(lines(&single).len(), 3, "只有一檔不附合計");
}

/// run 把下週除權息接在持股填息之後。
#[tokio::test]
async fn run_includes_upcoming_dividends() {
    let mut held = dividend("2330", 20, dec!(5), Decimal::ZERO);
    held.name = "台積電".to_owned();
    held.shares = 1000;
    let section = UpcomingSection {
        from: day(19),
        to: day(23),
        items: vec![UpcomingLine {
            dividend: held,
            fill: None,
        }],
        payouts: vec![payout("0056", 21)],
        month: None,
    };
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
        None,
        &[],
        Some(&section),
    )
    .await
    .expect("有內容");
    assert!(
        message.contains(&esc(
            "• 10-20 2330 台積電（1,000 股）除息 5 元、預估入帳 5,000 元"
        )),
        "{message}"
    );
    assert!(
        message.contains(&esc("• 10-21 0056 名0056（2,000 股）現金 2,872.4 元")),
        "{message}"
    );
}

fn payout(symbol: &str, payable_day: u32) -> DividendPayout {
    DividendPayout {
        member_id: 0,
        payable_date: day(payable_day),
        stock_symbol: symbol.to_owned(),
        name: format!("名{symbol}"),
        quarter: "Q3".to_owned(),
        ex_date: Some(day(1)),
        shares: 2000,
        cash_dividend: dec!(1.4362),
        stock_dividend: Decimal::ZERO,
        cash_amount: dec!(2872.4),
        stock_shares: 0,
        estimated: false,
    }
}

/// 入帳一段：每筆一行（現金、配股、推算註記），最後附現金合計；沒有除權息時只列入帳。
#[test]
fn lines_show_payouts_with_total() {
    let mut with_stock = payout("2330", 22);
    with_stock.cash_amount = dec!(4500);
    with_stock.stock_shares = 50;
    with_stock.estimated = true;
    let mut stock_only = payout("2002", 23);
    stock_only.cash_amount = Decimal::ZERO;
    stock_only.stock_shares = 100;
    let section = UpcomingSection {
        from: day(19),
        to: day(23),
        items: Vec::new(),
        payouts: vec![payout("0056", 20), with_stock, stock_only],
        month: None,
    };
    assert_eq!(
        lines(&section),
        vec![
            String::new(),
            "下週持股股利入帳（10-19～10-23）".to_owned(),
            "• 10-20 0056 名0056（2,000 股）現金 2,872.4 元".to_owned(),
            "• 10-22 2330 名2330（2,000 股）現金 4,500 元、配股 50 股（以目前持股推算）".to_owned(),
            "• 10-23 2002 名2002（2,000 股）配股 100 股".to_owned(),
            "• 合計現金入帳 7,372.4 元（未扣補充保費與匯費）".to_owned(),
        ]
    );
}

fn allocation(member_id: i64, shares: i64) -> PayoutAllocation {
    PayoutAllocation {
        member_id,
        stock_symbol: "0056".to_owned(),
        name: "元大高股息".to_owned(),
        quarter: "Q3".to_owned(),
        cash_dividend: dec!(1),
        stock_dividend: Decimal::ZERO,
        cash_payable: Some(day(21)),
        stock_payable: None,
        cash_ex: Some(day(1)),
        stock_ex: None,
        shares,
        cash_amount: Decimal::from(shares),
        stock_shares: Decimal::ZERO,
        estimated: false,
    }
}

/// 沒有持股除權息時只列入帳；入帳查詢失敗、也沒有除權息時整段不列。
#[tokio::test]
async fn load_upcoming_lists_payouts_and_degrades_on_failure() {
    let dividends = crate::infra::database::repository::dividend::PgDividendRepository::new();
    let no_holdings = std::collections::BTreeMap::new();
    let repo = FakePayoutRepository::with_allocations(vec![allocation(1, 1000)]);
    let section = load_upcoming(&dividends, &repo, &no_holdings, day(17))
        .await
        .expect("有入帳");
    assert!(section.items.is_empty());
    assert_eq!(section.payouts.len(), 1);

    let failing = FakePayoutRepository {
        fail: true,
        ..FakePayoutRepository::default()
    };
    assert!(
        load_upcoming(&dividends, &failing, &no_holdings, day(17))
            .await
            .is_none()
    );
}

/// 查詢下週入帳：倉儲以下週一到週五查詢，結果合併會員。
#[tokio::test]
async fn next_week_payouts_queries_next_monday_to_friday() {
    let repo =
        FakePayoutRepository::with_allocations(vec![allocation(1, 1000), allocation(2, 2000)]);
    let payouts = next_week_payouts(&repo, day(17)).await.expect("查詢入帳");

    assert_eq!(repo.queries(), [(0, day(19), day(23), day(17))]);
    assert_eq!(payouts.len(), 1, "兩位會員同一筆合併");
    assert_eq!(
        (payouts[0].shares, payouts[0].cash_amount),
        (3000, dec!(3000))
    );
}

fn october() -> MonthIncome {
    MonthIncome {
        month: 10,
        received: dec!(1000),
        pending: dec!(2500.5),
        last_year: dec!(3000),
    }
}

/// 本月入帳一段：已入帳、待入帳與去年同月；接在下週入帳之後。
#[test]
fn lines_show_month_income() {
    let section = UpcomingSection {
        from: day(19),
        to: day(23),
        items: Vec::new(),
        payouts: Vec::new(),
        month: Some(october()),
    };
    assert_eq!(
        lines(&section),
        vec![
            String::new(),
            "本月持股股利入帳（10 月）".to_owned(),
            "• 已入帳 1,000 元、待入帳 2,500.5 元；去年同月 3,000 元".to_owned(),
        ]
    );
}

/// 指定發放日的入帳明細。
fn paid_on(date: NaiveDate, shares: i64) -> PayoutAllocation {
    PayoutAllocation {
        cash_payable: Some(date),
        ..allocation(1, shares)
    }
}

/// 本月入帳：發放日不晚於今天算已入帳、之後算待入帳；去年同月另查一次；其他月份不計。
#[tokio::test]
async fn month_income_splits_received_pending_and_last_year() {
    let last_year = NaiveDate::from_ymd_opt(2025, 10, 15).expect("有效日期");
    let next_month = NaiveDate::from_ymd_opt(2026, 11, 2).expect("有效日期");
    let repo = FakePayoutRepository::with_allocations(vec![
        paid_on(day(5), 1000),
        paid_on(day(17), 500),
        paid_on(day(21), 2000),
        paid_on(last_year, 3000),
        paid_on(next_month, 9000),
    ]);

    let income = month_income(&repo, day(17)).await.expect("查詢入帳");

    assert_eq!(
        income,
        Some(MonthIncome {
            month: 10,
            received: dec!(1500),
            pending: dec!(2000),
            last_year: dec!(3000),
        })
    );
    let october_2025 = NaiveDate::from_ymd_opt(2025, 10, 1).expect("有效日期");
    let end_2025 = NaiveDate::from_ymd_opt(2025, 10, 31).expect("有效日期");
    assert_eq!(
        repo.queries(),
        [
            (0, day(1), day(31), day(17)),
            (0, october_2025, end_2025, day(17))
        ]
    );
}

/// 兩個月都沒有現金入帳時不列；查詢失敗回傳錯誤。
#[tokio::test]
async fn month_income_is_none_without_cash_and_errors_on_failure() {
    let next_month = NaiveDate::from_ymd_opt(2026, 11, 2).expect("有效日期");
    let repo = FakePayoutRepository::with_allocations(vec![paid_on(next_month, 1000)]);
    assert_eq!(month_income(&repo, day(17)).await.expect("查詢入帳"), None);

    let failing = FakePayoutRepository {
        fail: true,
        ..FakePayoutRepository::default()
    };
    assert!(month_income(&failing, day(17)).await.is_err());
}
