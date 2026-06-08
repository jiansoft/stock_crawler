use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::{Datelike, Days, Local, NaiveDate, Weekday};
use rust_decimal::Decimal;

use crate::{
    app::calculation, core::declare::Industry, core::logging,
    domain::dividend::repository::DividendRepository,
    domain::portfolio::entity::StockOwnershipDetail,
    domain::portfolio::repository::PortfolioRepository, infra::crawler::twse,
    infra::database::repository::dividend::PgDividendRepository,
    infra::database::repository::portfolio::PgPortfolioRepository,
    infra::database::table::dividend::extension::stock_dividend_info::StockDividendInfo,
    interfaces::bot, interfaces::bot::telegram::Telegram,
};

use super::{format_decimal_with_commas, format_share_quantity, member_label};

/// 判斷一筆除權息資料是否屬於 ETF。
fn is_etf(stock: &StockDividendInfo) -> bool {
    stock.stock_industry_id == Industry::ExchangeTradedFund.serial()
}

/// 依殖利率由高到低比較兩筆除權息資料。
fn compare_dividend_yield_desc(a: &StockDividendInfo, b: &StockDividendInfo) -> std::cmp::Ordering {
    b.dividend_yield
        .partial_cmp(&a.dividend_yield)
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// 將市場清單排序成「股票在前、ETF 在後」，各群組內再按殖利率降序。
fn sort_market_dividend_info(stocks_dividend_info: &mut [StockDividendInfo]) {
    stocks_dividend_info.sort_by(|a, b| match (is_etf(a), is_etf(b)) {
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        _ => compare_dividend_yield_desc(a, b),
    });
}

/// 將同一類別的除權息清單行文字寫入 Telegram 訊息。
fn write_market_dividend_rows<'a>(
    msg: &mut String,
    title: &str,
    stocks: impl Iterator<Item = &'a StockDividendInfo>,
) {
    let mut has_rows = false;
    for stock in stocks {
        if !has_rows {
            let _ = writeln!(msg, "{}︰", Telegram::escape_markdown_v2(title));
            has_rows = true;
        }

        // 市場清單列出指定日期有除權或除息的股票與殖利率。
        let _ = writeln!(
            msg,
            "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1} 現金︰{2}元\\({6}%\\) 股票 {3}元 合計︰{4}元\\({7}%\\) 昨收價:{5} 現金殖利率:{6}% 殖利率:{7}%",
            stock.stock_symbol,
            Telegram::escape_markdown_v2(&stock.name),
            Telegram::escape_markdown_v2(stock.cash_dividend.normalize().to_string()),
            Telegram::escape_markdown_v2(stock.stock_dividend.normalize().to_string()),
            Telegram::escape_markdown_v2(stock.sum.normalize().to_string()),
            Telegram::escape_markdown_v2(stock.closing_price.normalize().to_string()),
            Telegram::escape_markdown_v2(stock.cash_dividend_yield.normalize().to_string()),
            Telegram::escape_markdown_v2(stock.dividend_yield.normalize().to_string())
        );
    }
}

/// 組出指定日期的市場除權息清單訊息。
fn build_market_dividend_message(
    date: NaiveDate,
    title: &str,
    stocks_dividend_info: &[StockDividendInfo],
) -> String {
    let mut msg = String::with_capacity(2048);
    if writeln!(
        &mut msg,
        "{} {}",
        Telegram::escape_markdown_v2(date.to_string()),
        Telegram::escape_markdown_v2(title)
    )
    .is_ok()
    {
        write_market_dividend_rows(
            &mut msg,
            "股票",
            stocks_dividend_info.iter().filter(|stock| !is_etf(stock)),
        );
        write_market_dividend_rows(
            &mut msg,
            "ETF",
            stocks_dividend_info.iter().filter(|stock| is_etf(stock)),
        );
    }

    msg
}

/// 依今日除權息事件與目前持股，組出第二則持股預估股利通知。
///
/// 計算規則：
/// 1. 只統計今天以前買入、且目前尚未賣出的持股。
/// 2. 同一檔股票若多人持有，按 `member_id` 分開統計。
/// 3. 只有今天真的發生除息/除權的欄位才納入本次預估。
fn build_holding_dividend_message(
    today: NaiveDate,
    stocks_dividend_info: &[StockDividendInfo],
    holdings: &[StockOwnershipDetail],
) -> Option<String> {
    // 先把今日除權息股票轉成查表結構，後續可用持股代號快速對應股利資料。
    let stock_info_map = stocks_dividend_info
        .iter()
        .map(|stock| (stock.stock_symbol.as_str(), stock))
        .collect::<std::collections::HashMap<_, _>>();
    // key = (股票代號, member_id)，value = (股票名稱, 持股股數合計, 持股成本合計, 現金股利合計, 股票股利合計)
    let mut grouped = BTreeMap::<(String, i64), (String, i64, Decimal, Decimal, Decimal)>::new();

    for holding in holdings
        .iter()
        .filter(|holding| holding.created_time.date_naive() < today)
    {
        // 持股代號不在今日除權息清單內時，直接略過。
        let Some(stock) = stock_info_map.get(holding.security_code.as_str()) else {
            continue;
        };

        let share_quantity = Decimal::from(holding.share_quantity);
        // 只有今天是除息日才計入現金股利；若今天只有除權，現金股利應為 0。
        let estimated_cash_dividend = if stock.is_cash_ex_dividend_on_date {
            stock.cash_dividend * share_quantity
        } else {
            Decimal::ZERO
        };
        // 只有今天是除權日才計入股票股利；若今天只有除息，股票股利應為 0。
        let estimated_stock_dividend = if stock.is_stock_ex_dividend_on_date {
            stock.stock_dividend * share_quantity
        } else {
            Decimal::ZERO
        };
        // 殖利率以目前每股成本為基準，而不是歷史買入總成本。
        let holding_cost = (holding.current_cost_per_share * share_quantity).abs();

        // 同一人同一檔若分多筆買進，這裡合併成一列訊息。
        // 每股成本以加權平均成本重算，避免不同批次成本不一致時訊息失真。
        let entry = grouped
            .entry((holding.security_code.clone(), holding.member_id))
            .or_insert_with(|| {
                (
                    stock.name.clone(),
                    0,
                    Decimal::ZERO,
                    Decimal::ZERO,
                    Decimal::ZERO,
                )
            });
        let total_shares = entry.1 + holding.share_quantity;
        let total_cost = entry.2 + holding_cost;
        entry.1 = total_shares;
        entry.2 = total_cost;
        entry.3 += estimated_cash_dividend;
        entry.4 += estimated_stock_dividend;
    }

    if grouped.is_empty() {
        return None;
    }

    // 第二則訊息專門呈現「自己目前持有的股票」在今日除權息可領多少。
    let mut msg = String::with_capacity(2048);
    if writeln!(
        &mut msg,
        "{} 持股除權息預估如下︰",
        Telegram::escape_markdown_v2(today.to_string())
    )
    .is_err()
    {
        return None;
    }

    for (
        (stock_symbol, member_id),
        (name, share_quantity, holding_cost, cash_dividend, stock_dividend),
    ) in grouped
    {
        let cash_yield = if holding_cost.is_zero() {
            Decimal::ZERO
        } else {
            (cash_dividend / holding_cost) * Decimal::new(100, 0)
        };
        let total_yield = if holding_cost.is_zero() {
            Decimal::ZERO
        } else {
            ((cash_dividend + stock_dividend) / holding_cost) * Decimal::new(100, 0)
        };
        let current_cost_per_share = if share_quantity == 0 {
            Decimal::ZERO
        } else {
            holding_cost / Decimal::from(share_quantity)
        };
        // 最後輸出時才 round，內部計算過程維持原始精度。
        let _ = writeln!(
            &mut msg,
            "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1} {2} 持股:{3}股 成本:{4}元\\({5}元\\) 現金股利:{6}元 股票股利:{7}元 現金殖利率:{8}% 殖利率:{9}%",
            stock_symbol,
            Telegram::escape_markdown_v2(name),
            Telegram::escape_markdown_v2(member_label(member_id)),
            Telegram::escape_markdown_v2(format_share_quantity(share_quantity)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(holding_cost)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(current_cost_per_share)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(cash_dividend)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(stock_dividend)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(cash_yield)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(total_yield))
        );
    }

    Some(msg)
}

/// 取得並排序指定日期的市場除權息資料。
async fn fetch_sorted_market_dividend_info(date: NaiveDate) -> Result<Vec<StockDividendInfo>> {
    let dividend_repo = PgDividendRepository::new();
    let mut stocks_dividend_info = dividend_repo
        .fetch_stocks_with_dividends_on_date(date)
        .await?;

    // 先顯示股票，再顯示 ETF；兩組內部各自按殖利率降序排序。
    sort_market_dividend_info(&mut stocks_dividend_info);

    Ok(stocks_dividend_info)
}

/// 發送指定日期的市場除權息提醒。
async fn send_market_dividend_message(
    date: NaiveDate,
    title: &str,
    stocks_dividend_info: &[StockDividendInfo],
) {
    if stocks_dividend_info.is_empty() {
        return;
    }

    let msg = build_market_dividend_message(date, title, stocks_dividend_info);

    bot::telegram::send(&msg).await;
}

/// 取得指定年份的交易所休市日集合。
///
/// 此函式會呼叫台灣證券交易所（TWSE）的休市日 API，將回傳的日期放入 `HashSet` 中。
/// 若連線或解析失敗，會透過 `logging::error_file_async` 記錄錯誤，並回傳空集合，
/// 此時系統將會降級為僅依據星期六、日進行交易日判斷。
///
/// # 參數
///
/// * `year` - 要查詢的西元年份。
async fn get_holidays_set(year: i32) -> HashSet<NaiveDate> {
    // 呼叫 TWSE 的休市日 API 取得資料
    match twse::holiday_schedule::visit(year).await {
        Ok(holidays) => {
            // 將所有休市日期收集至 HashSet 以利後續快速查表
            holidays.into_iter().map(|h| h.date).collect()
        }
        Err(err) => {
            // 發生網路或解析錯誤時，發送錯誤日誌並降級（回傳空集合）
            logging::error_file_async(format!(
                "Failed to fetch TWSE holiday schedule for {}, falling back to weekend check: {:?}",
                year, err
            ));
            HashSet::new()
        }
    }
}

/// 判斷特定日期是否為交易日。
///
/// 若該日期為星期六或星期日，或是存在於 `holidays` 休市日集合中，則判定為非交易日。
///
/// # 參數
///
/// * `date` - 要判定的日期。
/// * `holidays` - 已載入的休市日 `HashSet`。
fn is_trading_day(date: NaiveDate, holidays: &HashSet<NaiveDate>) -> bool {
    // 1. 如果是星期六或星期日，則不是交易日
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    // 2. 如果該日期在交易所公告的休市日名單中，則不是交易日
    !holidays.contains(&date)
}

/// 尋找指定日期之後的下一個交易日。
///
/// 從指定日期的隔天開始遞增，直到找到符合交易日條件的日期為止。
///
/// # 參數
///
/// * `date` - 起始日期。
/// * `holidays` - 已載入的休市日 `HashSet`。
fn find_next_trading_day(mut date: NaiveDate, holidays: &HashSet<NaiveDate>) -> NaiveDate {
    loop {
        // 遞增一天，若加天數溢出則回退使用一般的加法運算
        date = date
            .checked_add_days(Days::new(1))
            .unwrap_or(date + Days::new(1));
        // 判斷遞增後的日期是否為交易日
        if is_trading_day(date, holidays) {
            return date;
        }
    }
}

/// 發送今日除權息提醒，並追加持股預估股利與下一交易日除權息預告通知。
///
/// 此任務每天早上 08:00 執行。現在會判斷今天是否為交易日：
/// 1. 若今天為非交易日，則直接跳過不處理，以避免重複或無效的通知。
/// 2. 若今天為交易日，則除原本今日提醒與持股計算外，會預報「下一個交易日」而非「曆法明天」的除權息名單。
pub async fn execute() -> Result<()> {
    let today: NaiveDate = Local::now().date_naive();

    // 載入今年度的休市日清單
    let current_year = today.year();
    let mut holidays = get_holidays_set(current_year).await;

    // 計算明天以判斷是否跨年，若跨年則需一併載入明年度的休市清單
    let tomorrow = today
        .checked_add_days(Days::new(1))
        .context("Failed to calculate tomorrow for ex-dividend reminder")?;
    if tomorrow.year() != current_year {
        let next_year_holidays = get_holidays_set(tomorrow.year()).await;
        holidays.extend(next_year_holidays);
    }

    // 若今天不是交易日（週末或節假日），直接跳過不發送通知
    if !is_trading_day(today, &holidays) {
        return Ok(());
    }

    // 尋找下一個實際交易日
    let next_trading = find_next_trading_day(today, &holidays);

    // 取得本日市場除權息資料
    let stocks_dividend_info = fetch_sorted_market_dividend_info(today).await?;

    // 先發送今日市場清單，維持既有提醒順序。
    send_market_dividend_message(
        today,
        "進行除權息的股票與 ETF 如下︰",
        &stocks_dividend_info,
    )
    .await;

    let stock_symbols: Vec<String> = stocks_dividend_info
        .iter()
        .map(|stock| stock.stock_symbol.to_string())
        .collect();

    if stock_symbols.is_empty() {
        // 本日無除權息時，只發送下一個交易日的預定公告，並提早返回
        let next_stocks_dividend_info = fetch_sorted_market_dividend_info(next_trading).await?;
        send_market_dividend_message(
            next_trading,
            "預計進行除權息的股票與 ETF 如下︰",
            &next_stocks_dividend_info,
        )
        .await;
        return Ok(());
    }

    // 再更新這批股票對應持股的股利記錄。
    calculation::dividend_record::execute(today.year(), Some(stock_symbols.clone())).await;

    // 重新讀取持股後，組第二則「分人分股」的預估股利通知。
    let portfolio_repo = PgPortfolioRepository::new();
    let holdings = portfolio_repo
        .fetch_active_holdings(Some(stock_symbols))
        .await?;
    if let Some(holding_msg) =
        build_holding_dividend_message(today, &stocks_dividend_info, &holdings)
    {
        bot::telegram::send(&holding_msg).await;
    }

    // 最後發送下一交易日的預訂除權息公告
    let next_stocks_dividend_info = fetch_sorted_market_dividend_info(next_trading).await?;
    send_market_dividend_message(
        next_trading,
        "預計進行除權息的股票與 ETF 如下︰",
        &next_stocks_dividend_info,
    )
    .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::TimeZone;
    use rust_decimal_macros::dec;
    use tokio::time;

    use crate::core::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        logging::info_file_async("開始 execute".to_string());
        //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        let _ = execute().await;

        logging::info_file_async("結束 execute".to_string());
        time::sleep(Duration::from_secs(1)).await;
    }

    #[test]
    fn test_is_trading_day() {
        let mut holidays = HashSet::new();
        // 2026-05-25 (週一)
        let monday = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        // 2026-05-24 (週日)
        let sunday = NaiveDate::from_ymd_opt(2026, 5, 24).unwrap();
        // 2026-05-23 (週六)
        let saturday = NaiveDate::from_ymd_opt(2026, 5, 23).unwrap();

        // 未設定休市日時，週一應為交易日，週末非交易日
        assert!(is_trading_day(monday, &holidays));
        assert!(!is_trading_day(sunday, &holidays));
        assert!(!is_trading_day(saturday, &holidays));

        // 將週一設為休市日
        holidays.insert(monday);
        assert!(!is_trading_day(monday, &holidays));
    }

    #[test]
    fn test_find_next_trading_day() {
        let mut holidays = HashSet::new();
        // 2026-05-22 (週五)
        let friday = NaiveDate::from_ymd_opt(2026, 5, 22).unwrap();
        // 2026-05-25 (週一)
        let monday = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        // 2026-05-26 (週二)
        let tuesday = NaiveDate::from_ymd_opt(2026, 5, 26).unwrap();

        // 正常週五的下一交易日應為週一
        assert_eq!(find_next_trading_day(friday, &holidays), monday);

        // 若週一為節假日休市，則週五的下一交易日應為週二
        holidays.insert(monday);
        assert_eq!(find_next_trading_day(friday, &holidays), tuesday);
    }

    #[test]
    fn test_build_holding_dividend_message_groups_by_stock_and_member() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let stocks = vec![
            StockDividendInfo {
                stock_symbol: "2330".to_string(),
                name: "台積電".to_string(),
                stock_industry_id: Industry::Semiconductor.serial(),
                cash_dividend: dec!(3.5),
                stock_dividend: dec!(0.2),
                sum: dec!(3.7),
                closing_price: dec!(950),
                dividend_yield: dec!(0.39),
                cash_dividend_yield: dec!(0.37),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
            StockDividendInfo {
                stock_symbol: "2317".to_string(),
                name: "鴻海".to_string(),
                stock_industry_id: Industry::ElectronicComponents.serial(),
                cash_dividend: dec!(5),
                stock_dividend: dec!(0.3),
                sum: dec!(5.3),
                closing_price: dec!(150),
                dividend_yield: dec!(3.53),
                cash_dividend_yield: dec!(3.33),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: true,
            },
        ];
        let holdings = vec![
            StockOwnershipDetail {
                serial: 1,
                security_code: "2330".to_string(),
                member_id: 1,
                share_quantity: 1000,
                share_price_average: Decimal::ZERO,
                current_cost_per_share: dec!(600),
                holding_cost: dec!(-600000),
                is_sold: false,
                cumulate_dividends_cash: Decimal::ZERO,
                cumulate_dividends_stock: Decimal::ZERO,
                cumulate_dividends_stock_money: Decimal::ZERO,
                cumulate_dividends_total: Decimal::ZERO,
                created_time: Local.with_ymd_and_hms(2026, 3, 31, 9, 0, 0).unwrap(),
            },
            StockOwnershipDetail {
                serial: 2,
                security_code: "2330".to_string(),
                member_id: 2,
                share_quantity: 500,
                share_price_average: Decimal::ZERO,
                current_cost_per_share: dec!(500),
                holding_cost: dec!(-250000),
                is_sold: false,
                cumulate_dividends_cash: Decimal::ZERO,
                cumulate_dividends_stock: Decimal::ZERO,
                cumulate_dividends_stock_money: Decimal::ZERO,
                cumulate_dividends_total: Decimal::ZERO,
                created_time: Local.with_ymd_and_hms(2026, 3, 30, 9, 0, 0).unwrap(),
            },
            StockOwnershipDetail {
                serial: 3,
                security_code: "2317".to_string(),
                member_id: 2,
                share_quantity: 2000,
                share_price_average: Decimal::ZERO,
                current_cost_per_share: dec!(120),
                holding_cost: dec!(-240000),
                is_sold: false,
                cumulate_dividends_cash: Decimal::ZERO,
                cumulate_dividends_stock: Decimal::ZERO,
                cumulate_dividends_stock_money: Decimal::ZERO,
                cumulate_dividends_total: Decimal::ZERO,
                created_time: Local.with_ymd_and_hms(2026, 3, 20, 9, 0, 0).unwrap(),
            },
            StockOwnershipDetail {
                serial: 4,
                security_code: "2317".to_string(),
                member_id: 2,
                share_quantity: 1000,
                share_price_average: Decimal::ZERO,
                current_cost_per_share: dec!(120),
                holding_cost: dec!(-120000),
                is_sold: false,
                cumulate_dividends_cash: Decimal::ZERO,
                cumulate_dividends_stock: Decimal::ZERO,
                cumulate_dividends_stock_money: Decimal::ZERO,
                cumulate_dividends_total: Decimal::ZERO,
                created_time: Local.with_ymd_and_hms(2026, 4, 1, 9, 0, 0).unwrap(),
            },
        ];

        let msg = build_holding_dividend_message(today, &stocks, &holdings).unwrap();

        assert!(msg.contains("2330"));
        assert!(msg.contains("Eddie"));
        assert!(msg.contains("持股:1,000股"));
        assert!(msg.contains("成本:600,000元\\(600元\\)"));
        assert!(msg.contains("現金股利:3,500元"));
        assert!(msg.contains("股票股利:0元"));
        assert!(msg.contains("現金殖利率:0\\.58%"));
        assert!(msg.contains("殖利率:0\\.58%"));
        assert!(msg.contains("Unice"));
        assert!(msg.contains("持股:2,000股"));
        assert!(msg.contains("成本:240,000元\\(120元\\)"));
        assert!(msg.contains("現金股利:10,000元"));
        assert!(msg.contains("股票股利:600元"));
        assert!(msg.contains("現金殖利率:4\\.17%"));
        assert!(msg.contains("殖利率:4\\.42%"));
        assert!(!msg.contains("持股:3000股"));
    }

    #[test]
    fn test_market_dividend_message_groups_stocks_before_etfs_and_sorts_each_group_by_yield() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let mut stocks = vec![
            StockDividendInfo {
                stock_symbol: "0050".to_string(),
                name: "元大台灣50".to_string(),
                stock_industry_id: Industry::ExchangeTradedFund.serial(),
                cash_dividend: dec!(2),
                stock_dividend: Decimal::ZERO,
                sum: dec!(2),
                closing_price: dec!(100),
                dividend_yield: dec!(2),
                cash_dividend_yield: dec!(2),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
            StockDividendInfo {
                stock_symbol: "2317".to_string(),
                name: "鴻海".to_string(),
                stock_industry_id: Industry::ElectronicComponents.serial(),
                cash_dividend: dec!(5),
                stock_dividend: Decimal::ZERO,
                sum: dec!(5),
                closing_price: dec!(100),
                dividend_yield: dec!(5),
                cash_dividend_yield: dec!(5),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
            StockDividendInfo {
                stock_symbol: "00878".to_string(),
                name: "國泰永續高股息".to_string(),
                stock_industry_id: Industry::ExchangeTradedFund.serial(),
                cash_dividend: dec!(3),
                stock_dividend: Decimal::ZERO,
                sum: dec!(3),
                closing_price: dec!(100),
                dividend_yield: dec!(3),
                cash_dividend_yield: dec!(3),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
            StockDividendInfo {
                stock_symbol: "2330".to_string(),
                name: "台積電".to_string(),
                stock_industry_id: Industry::Semiconductor.serial(),
                cash_dividend: dec!(1),
                stock_dividend: Decimal::ZERO,
                sum: dec!(1),
                closing_price: dec!(100),
                dividend_yield: dec!(1),
                cash_dividend_yield: dec!(1),
                is_cash_ex_dividend_on_date: true,
                is_stock_ex_dividend_on_date: false,
            },
        ];

        sort_market_dividend_info(&mut stocks);
        let msg = build_market_dividend_message(today, "進行除權息的股票與 ETF 如下︰", &stocks);

        let stock_section = msg.find("股票︰").unwrap();
        let etf_section = msg.find("ETF︰").unwrap();
        let hon_hai = msg.find("2317").unwrap();
        let tsmc = msg.find("2330").unwrap();
        let high_yield_etf = msg.find("00878").unwrap();
        let low_yield_etf = msg.find("0050").unwrap();

        assert!(msg.contains("2026\\-04\\-01 進行除權息的股票與 ETF 如下︰"));
        assert!(stock_section < hon_hai);
        assert!(hon_hai < tsmc);
        assert!(tsmc < etf_section);
        assert!(etf_section < high_yield_etf);
        assert!(high_yield_etf < low_yield_etf);
    }

    #[test]
    fn test_format_decimal_with_commas() {
        assert_eq!(format_decimal_with_commas(dec!(16638)), "16,638");
        assert_eq!(format_decimal_with_commas(dec!(83.19)), "83.19");
        assert_eq!(format_decimal_with_commas(dec!(1234567.8)), "1,234,567.8");
    }
}
