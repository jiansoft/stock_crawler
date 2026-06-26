//! # 領域事件派發器模組
//!
//! 負責接收領域事件 ([`DomainEvent`]) 並在背景非同步處理對應的副作用，
//! 例如 Telegram 通知與 gRPC 同步推送。
//! 此模組的目的是將核心業務邏輯 (Use Case) 與外部副作用解耦，
//! 使 Use Case 僅負責商業編排，不直接耦合基礎設施。

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::app::event::taiwan_stock::{
    format_decimal_with_commas as format_decimal_flexible_commas,
    format_decimal_with_fixed_two_commas as format_decimal_with_commas, format_share_quantity,
    member_label,
};
use crate::core::declare::Industry;
use crate::domain::dividend::entity::StockDividendInfo;
use crate::domain::dividend::repository::DividendRepository;
use crate::domain::events::DomainEvent;
use crate::domain::money_flow::repository::MoneyFlowRepository;
use crate::domain::portfolio::entity::StockOwnershipDetail;
use crate::domain::portfolio::repository::PortfolioRepository;
use crate::infra::database::repository::dividend::PgDividendRepository;
use crate::infra::database::repository::money_flow::PgMoneyFlowRepository;
use crate::infra::database::repository::portfolio::PgPortfolioRepository;
use crate::interfaces::bot::telegram::Telegram;
use chrono::Datelike;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::fmt::Write;

/// 全域事件派發器實例，使用 `OnceLock` 確保只初始化一次。
static EVENT_DISPATCHER: OnceLock<EventDispatcher> = OnceLock::new();

/// 事件處理函式的型別別名。
///
/// 接收一個 `DomainEvent`，回傳一個非同步的 `Result<()>`。
/// 使用 trait object 以支援測試時替換為 fake handler。
type EventHandlerFn = Box<
    dyn Fn(DomainEvent) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// <summary>
/// Telegram 訊息的防震 (Debouncer) 發送器。
/// 用於將短時間內密集產生的 Telegram 訊息合併後批次發送，以節省 API 呼叫次數，
/// 並維持與重構前批次發送通知相同的視覺呈現。
/// </summary>
struct TelegramDebouncer {
    buffer: Arc<Mutex<Vec<String>>>,
    epoch: Arc<AtomicU64>,
}

impl TelegramDebouncer {
    /// 建立新的 Debouncer 實例。
    fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 將單一訊息加入發送緩衝區，並重置定時器。
    /// 在 500 毫秒內若無新訊息加入，則會觸發批次發送。
    async fn add_message(&self, msg: String) {
        {
            let mut buf = self.buffer.lock().await;
            buf.push(msg);
        }
        let current_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;

        let buffer = self.buffer.clone();
        let epoch = self.epoch.clone();

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if epoch.load(Ordering::SeqCst) == current_epoch {
                let mut buf = buffer.lock().await;
                if !buf.is_empty() {
                    let merged = buf.join("\r\n");
                    buf.clear();
                    let _ = crate::interfaces::bot::telegram::send(&merged).await;
                }
            }
        });
    }
}

/// <summary>
/// 領域事件派發器 (Event Dispatcher)。
/// 透過 `tokio::sync::mpsc` 非阻塞通道接收領域事件，
/// 並在背景 tokio task 中逐一處理。
/// </summary>
pub struct EventDispatcher {
    /// 事件發送端，Use Case 透過此通道將事件送入背景處理迴圈。
    event_sender: mpsc::Sender<DomainEvent>,
}

impl EventDispatcher {
    /// <summary>
    /// 建立新的事件派發器，並啟動背景事件處理迴圈。
    /// </summary>
    ///
    /// # 參數
    /// - `handler`: 自訂的事件處理函式。若為 `None`，則使用預設的生產處理器。
    /// - `buffer_size`: mpsc 通道的緩衝區大小。
    pub fn new_with_handler(handler: Option<EventHandlerFn>, buffer_size: usize) -> Self {
        let (tx, mut rx) = mpsc::channel::<DomainEvent>(buffer_size);

        let debouncer = Arc::new(TelegramDebouncer::new());

        // 決定使用自訂 handler 或預設生產環境 handler
        let handler: EventHandlerFn = handler.unwrap_or_else(|| {
            let debouncer = debouncer.clone();
            Box::new(move |event| {
                let debouncer = debouncer.clone();
                Box::pin(async move { Self::default_handle_event(event, debouncer).await })
            })
        });

        // 啟動背景事件處理迴圈
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(why) = handler(event).await {
                    tracing::error!("領域事件處理失敗: {:?}", why);
                }
            }
        });

        EventDispatcher { event_sender: tx }
    }

    /// <summary>
    /// 建立使用預設生產環境處理器的事件派發器。
    /// 通道緩衝區大小預設為 100。
    /// </summary>
    pub fn new() -> Self {
        Self::new_with_handler(None, 100)
    }
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl EventDispatcher {
    /// <summary>
    /// 非同步派發一批領域事件至背景處理迴圈。
    /// 此方法不會阻塞呼叫端，事件會透過 mpsc 通道送出。
    /// </summary>
    pub async fn dispatch_async(&self, events: Vec<DomainEvent>) {
        for event in events {
            if let Err(why) = self.event_sender.send(event).await {
                tracing::error!("無法送出領域事件至背景通道: {:?}", why);
            }
        }
    }

    /// <summary>
    /// 預設的生產環境事件處理函式。
    /// 根據事件類型分派至對應的副作用處理邏輯。
    /// </summary>
    async fn default_handle_event(
        event: DomainEvent,
        debouncer: Arc<TelegramDebouncer>,
    ) -> Result<()> {
        use crate::core::declare::StockExchangeMarket;
        use crate::infra::cache::SHARE;
        use crate::interfaces::bot::telegram::Telegram;

        match event {
            DomainEvent::StockRegistered {
                ref symbol,
                ref name,
                market_id,
                industry_id,
                ..
            } => {
                // 副作用 1：透過 gRPC 同步通知 Go 微服務
                Self::push_to_go_service(symbol, name, market_id, industry_id).await;

                // 副作用 2：產生日誌與寫入 Telegram 緩衝區進行防震批次發送
                let market = StockExchangeMarket::from(market_id);
                let market_name = match market {
                    None => " - ".to_string(),
                    Some(sem) => sem.name(),
                };
                let industry_name = SHARE
                    .get_industry_name(industry_id)
                    .unwrap_or_else(|| " - ".to_string());

                let log_msg = format!(
                    "新增股票︰ {stock_symbol} {stock_name} {market_name} {industry_name}",
                    stock_symbol = symbol,
                    stock_name = Telegram::escape_markdown_v2(name),
                    market_name = market_name,
                    industry_name = industry_name
                );

                tracing::info!("{}", log_msg.clone());
                debouncer.add_message(log_msg).await;
            }
            DomainEvent::StockIdentityChanged {
                ref symbol,
                ref new_name,
                new_market_id,
                new_industry_id,
                ..
            } => {
                // 副作用 1：透過 gRPC 同步通知 Go 微服務
                Self::push_to_go_service(symbol, new_name, new_market_id, new_industry_id).await;

                // 副作用 2：產生日誌與寫入 Telegram 緩衝區進行防震批次發送
                let market = StockExchangeMarket::from(new_market_id);
                let market_name = match market {
                    None => " - ".to_string(),
                    Some(sem) => sem.name(),
                };
                let industry_name = SHARE
                    .get_industry_name(new_industry_id)
                    .unwrap_or_else(|| " - ".to_string());

                let log_msg = format!(
                    "新增股票︰ {stock_symbol} {stock_name} {market_name} {industry_name}",
                    stock_symbol = symbol,
                    stock_name = Telegram::escape_markdown_v2(new_name),
                    market_name = market_name,
                    industry_name = industry_name
                );

                tracing::info!("{}", log_msg.clone());
                debouncer.add_message(log_msg).await;
            }
            DomainEvent::NetAssetValueUpdated { .. } => {
                // 目前每股淨值更新不需要額外副作用
            }
            DomainEvent::StockIndexUpdated {
                date,
                index,
                change,
                ..
            } => {
                let msg = format!(
                    "{} 大盤指數︰{} 漲跌︰{}",
                    Telegram::escape_markdown_v2(date.to_string()),
                    Telegram::escape_markdown_v2(index.to_string()),
                    Telegram::escape_markdown_v2(change.to_string())
                );
                debouncer.add_message(msg).await;
            }
            DomainEvent::MoneyFlowRecalculated { date, .. } => {
                Self::handle_money_flow_recalculated(date).await?;
            }
            DomainEvent::ExDividendReminderTriggered {
                date,
                next_trading_date,
                ..
            } => {
                Self::handle_ex_dividend_reminder_triggered(date, next_trading_date).await?;
            }
        }

        Ok(())
    }

    /// 透過 gRPC 將股票資訊同步推送至 Go 微服務。
    /// 推送失敗時僅記錄錯誤日誌，不中斷流程。
    async fn push_to_go_service(symbol: &str, name: &str, market_id: i32, industry_id: i32) {
        use crate::interfaces::rpc::client::stock_service;
        use crate::interfaces::rpc::stock::StockInfoRequest;

        let request = StockInfoRequest {
            stock_symbol: symbol.to_string(),
            name: name.to_string(),
            stock_exchange_market_id: market_id,
            stock_industry_id: industry_id,
            net_asset_value_per_share: 0.0,
            suspend_listing: false,
        };

        if let Err(why) = stock_service::push_stock_info_to_go_service(request).await {
            tracing::error!("Failed to push_stock_info_to_go_service for {} because {:?}",
                symbol, why);
        }
    }

    /// 處理 `MoneyFlowRecalculated` 事件：重新計算並發送 Telegram 市值變化通知。
    async fn handle_money_flow_recalculated(date: chrono::NaiveDate) -> Result<()> {
        let money_flow_repo = PgMoneyFlowRepository::new();
        // 透過倉儲獲取會員收盤與前日市值之對照資料
        let rows = money_flow_repo
            .fetch_member_money_history_with_previous_day(date)
            .await?;
        // 建立通知內容並發送 Telegram 訊息
        if let Some(msg) = Self::build_money_change_message(&rows) {
            crate::interfaces::bot::telegram::send(&msg).await;
        }

        Ok(())
    }

    /// 格式化市值變化行文字。
    fn format_money_change_line(
        label: &str,
        market_value: Decimal,
        previous_market_value: Decimal,
    ) -> String {
        let diff = market_value - previous_market_value;
        let percentage = if previous_market_value.is_zero() {
            "N/A".to_string()
        } else {
            format_decimal_with_commas(
                (diff / previous_market_value) * rust_decimal_macros::dec!(100),
            )
        };

        format!(
            "{}:{} {} \\({}%\\)",
            Telegram::escape_markdown_v2(label),
            Telegram::escape_markdown_v2(format_decimal_with_commas(market_value)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(diff)),
            Telegram::escape_markdown_v2(percentage),
        )
    }

    /// 組裝市值變化 Telegram 訊息。
    fn build_money_change_message(
        rows: &[crate::domain::money_flow::entity::MoneyFlowMemberWithPreviousDay],
    ) -> Option<String> {
        let date = rows.first()?.date;
        let mut msg = String::with_capacity(256);
        let _ = writeln!(
            &mut msg,
            "{} 市值變化",
            Telegram::escape_markdown_v2(date.to_string())
        );

        // 合計列
        if let Some(total_row) = rows.iter().find(|row| row.member_id == 0) {
            let _ = writeln!(
                &mut msg,
                "{}",
                Self::format_money_change_line(
                    "合計",
                    total_row.market_value,
                    total_row.previous_market_value
                )
            );
        }

        // 個別會員列
        for row in rows.iter().filter(|row| row.member_id > 0) {
            let _ = writeln!(
                &mut msg,
                "{}",
                Self::format_money_change_line(
                    &member_label(row.member_id),
                    row.market_value,
                    row.previous_market_value,
                )
            );
        }

        Some(msg.trim_end().to_string())
    }

    /// 處理 `ExDividendReminderTriggered` 事件：發送除權息提醒、重新計算持股股利並發送通知。
    async fn handle_ex_dividend_reminder_triggered(
        date: chrono::NaiveDate,
        next_trading_date: chrono::NaiveDate,
    ) -> Result<()> {
        let dividend_repo = PgDividendRepository::new();
        // 取得本日市場除權息資料
        let mut stocks_dividend_info = dividend_repo
            .fetch_stocks_with_dividends_on_date(date)
            .await?;
        Self::sort_market_dividend_info(&mut stocks_dividend_info);

        // 發送今日市場清單
        Self::send_market_dividend_message(
            date,
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
            let mut next_stocks = dividend_repo
                .fetch_stocks_with_dividends_on_date(next_trading_date)
                .await?;
            Self::sort_market_dividend_info(&mut next_stocks);
            Self::send_market_dividend_message(
                next_trading_date,
                "預計進行除權息的股票與 ETF 如下︰",
                &next_stocks,
            )
            .await;
            return Ok(());
        }

        // 更新這批股票對應持股的股利記錄
        crate::app::calculation::dividend_record::execute(date.year(), Some(stock_symbols.clone()))
            .await;

        // 重新讀取持股後，組「分人分股」的預估股利通知
        let portfolio_repo = PgPortfolioRepository::new();
        let holdings = portfolio_repo
            .fetch_active_holdings(Some(stock_symbols))
            .await?;

        if let Some(holding_msg) =
            Self::build_holding_dividend_message(date, &stocks_dividend_info, &holdings)
        {
            crate::interfaces::bot::telegram::send(&holding_msg).await;
        }

        // 最後發送下一交易日的預訂除權息公告
        let mut next_stocks = dividend_repo
            .fetch_stocks_with_dividends_on_date(next_trading_date)
            .await?;
        Self::sort_market_dividend_info(&mut next_stocks);
        Self::send_market_dividend_message(
            next_trading_date,
            "預計進行除權息的股票與 ETF 如下︰",
            &next_stocks,
        )
        .await;

        Ok(())
    }

    /// 判斷一筆除權息資料是否屬於 ETF。
    fn is_etf(stock: &StockDividendInfo) -> bool {
        stock.stock_industry_id == Industry::ExchangeTradedFund.serial()
    }

    /// 依殖利率由高到低比較兩筆除權息資料。
    fn compare_dividend_yield_desc(
        a: &StockDividendInfo,
        b: &StockDividendInfo,
    ) -> std::cmp::Ordering {
        b.dividend_yield
            .partial_cmp(&a.dividend_yield)
            .unwrap_or(std::cmp::Ordering::Equal)
    }

    /// 將市場清單排序成「股票在前、ETF 在後」，各群組內再按殖利率降序。
    fn sort_market_dividend_info(stocks_dividend_info: &mut [StockDividendInfo]) {
        stocks_dividend_info.sort_by(|a, b| match (Self::is_etf(a), Self::is_etf(b)) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => Self::compare_dividend_yield_desc(a, b),
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
        date: chrono::NaiveDate,
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
            Self::write_market_dividend_rows(
                &mut msg,
                "股票",
                stocks_dividend_info
                    .iter()
                    .filter(|stock| !Self::is_etf(stock)),
            );
            Self::write_market_dividend_rows(
                &mut msg,
                "ETF",
                stocks_dividend_info
                    .iter()
                    .filter(|stock| Self::is_etf(stock)),
            );
        }

        msg
    }

    /// 發送指定日期的市場除權息提醒。
    async fn send_market_dividend_message(
        date: chrono::NaiveDate,
        title: &str,
        stocks_dividend_info: &[StockDividendInfo],
    ) {
        if stocks_dividend_info.is_empty() {
            return;
        }

        let msg = Self::build_market_dividend_message(date, title, stocks_dividend_info);
        crate::interfaces::bot::telegram::send(&msg).await;
    }

    /// 依今日除權息事件與目前持股，組出第二則持股預估股利通知。
    fn build_holding_dividend_message(
        today: chrono::NaiveDate,
        stocks_dividend_info: &[StockDividendInfo],
        holdings: &[StockOwnershipDetail],
    ) -> Option<String> {
        let stock_info_map = stocks_dividend_info
            .iter()
            .map(|stock| (stock.stock_symbol.as_str(), stock))
            .collect::<std::collections::HashMap<_, _>>();
        let mut grouped =
            BTreeMap::<(String, i64), (String, i64, Decimal, Decimal, Decimal)>::new();

        for holding in holdings
            .iter()
            .filter(|holding| holding.created_time.date_naive() < today)
        {
            let Some(stock) = stock_info_map.get(holding.security_code.as_str()) else {
                continue;
            };

            let share_quantity = Decimal::from(holding.share_quantity);
            let estimated_cash_dividend = if stock.is_cash_ex_dividend_on_date {
                stock.cash_dividend * share_quantity
            } else {
                Decimal::ZERO
            };
            let estimated_stock_dividend = if stock.is_stock_ex_dividend_on_date {
                stock.stock_dividend * share_quantity
            } else {
                Decimal::ZERO
            };
            let holding_cost = (holding.current_cost_per_share * share_quantity).abs();

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

            let _ = writeln!(
                &mut msg,
                "    [{0}](https://tw\\.stock\\.yahoo\\.com/quote/{0}) {1} {2} 持股:{3}股 成本:{4}元\\({5}元\\) 現金股利:{6}元 股票股利:{7}元 現金殖利率:{8}% 殖利率:{9}%",
                stock_symbol,
                Telegram::escape_markdown_v2(name),
                Telegram::escape_markdown_v2(member_label(member_id)),
                Telegram::escape_markdown_v2(format_share_quantity(share_quantity)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(holding_cost)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(
                    current_cost_per_share
                )),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(cash_dividend)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(stock_dividend)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(cash_yield)),
                Telegram::escape_markdown_v2(format_decimal_flexible_commas(total_yield))
            );
        }

        Some(msg)
    }
}

/// <summary>
/// 初始化全域事件派發器。
/// 應在應用程式啟動時呼叫一次。
/// </summary>
pub fn init_global_dispatcher() {
    EVENT_DISPATCHER.get_or_init(EventDispatcher::new);
}

/// <summary>
/// 取得全域事件派發器的參照。
/// 若尚未初始化，會自動進行初始化。
/// </summary>
pub fn get_global_dispatcher() -> &'static EventDispatcher {
    EVENT_DISPATCHER.get_or_init(EventDispatcher::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// 建立一個假的事件處理器，將接收到的事件記錄到共用的 Vec 中。
    fn fake_handler(log: Arc<Mutex<Vec<DomainEvent>>>) -> EventHandlerFn {
        Box::new(move |event| {
            let log = log.clone();
            Box::pin(async move {
                log.lock().await.push(event);
                Ok(())
            })
        })
    }

    #[tokio::test]
    async fn test_dispatch_stock_registered_event() {
        let event_log = Arc::new(Mutex::new(Vec::new()));
        let dispatcher =
            EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

        let events = vec![DomainEvent::StockRegistered {
            symbol: "2330".to_string(),
            name: "台積電".to_string(),
            market_id: 2,
            industry_id: 24,
            occurred_at: chrono::Local::now(),
        }];

        dispatcher.dispatch_async(events).await;

        // 等待背景處理迴圈消化事件
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let log = event_log.lock().await;
        assert_eq!(log.len(), 1);
        if let DomainEvent::StockRegistered {
            ref symbol,
            ref name,
            market_id,
            industry_id,
            ..
        } = log[0]
        {
            assert_eq!(symbol, "2330");
            assert_eq!(name, "台積電");
            assert_eq!(market_id, 2);
            assert_eq!(industry_id, 24);
        } else {
            panic!("Expected StockRegistered event");
        }
    }

    #[tokio::test]
    async fn test_dispatch_identity_changed_event() {
        let event_log = Arc::new(Mutex::new(Vec::new()));
        let dispatcher =
            EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

        let events = vec![DomainEvent::StockIdentityChanged {
            symbol: "2330".to_string(),
            old_name: "台積電".to_string(),
            new_name: "台積電新".to_string(),
            old_market_id: 2,
            new_market_id: 3,
            old_industry_id: 24,
            new_industry_id: 25,
            occurred_at: chrono::Local::now(),
        }];

        dispatcher.dispatch_async(events).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let log = event_log.lock().await;
        assert_eq!(log.len(), 1);
        if let DomainEvent::StockIdentityChanged {
            ref symbol,
            ref old_name,
            ref new_name,
            ..
        } = log[0]
        {
            assert_eq!(symbol, "2330");
            assert_eq!(old_name, "台積電");
            assert_eq!(new_name, "台積電新");
        } else {
            panic!("Expected StockIdentityChanged event");
        }
    }

    #[tokio::test]
    async fn test_dispatch_nav_updated_event() {
        let event_log = Arc::new(Mutex::new(Vec::new()));
        let dispatcher =
            EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

        let events = vec![DomainEvent::NetAssetValueUpdated {
            symbol: "2330".to_string(),
            old_nav: dec!(90.0),
            new_nav: dec!(95.12),
            occurred_at: chrono::Local::now(),
        }];

        dispatcher.dispatch_async(events).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let log = event_log.lock().await;
        assert_eq!(log.len(), 1);
        if let DomainEvent::NetAssetValueUpdated {
            ref symbol,
            old_nav,
            new_nav,
            ..
        } = log[0]
        {
            assert_eq!(symbol, "2330");
            assert_eq!(old_nav, dec!(90.0));
            assert_eq!(new_nav, dec!(95.12));
        } else {
            panic!("Expected NetAssetValueUpdated event");
        }
    }

    #[tokio::test]
    async fn test_dispatch_stock_index_updated_event() {
        let event_log = Arc::new(Mutex::new(Vec::new()));
        let dispatcher =
            EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

        let events = vec![DomainEvent::StockIndexUpdated {
            date: chrono::NaiveDate::from_ymd_opt(2026, 6, 7).unwrap(),
            index: dec!(16500.25),
            change: dec!(120.50),
            occurred_at: chrono::Local::now(),
        }];

        dispatcher.dispatch_async(events).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let log = event_log.lock().await;
        assert_eq!(log.len(), 1);
        if let DomainEvent::StockIndexUpdated {
            date,
            index,
            change,
            ..
        } = log[0]
        {
            assert_eq!(date, chrono::NaiveDate::from_ymd_opt(2026, 6, 7).unwrap());
            assert_eq!(index, dec!(16500.25));
            assert_eq!(change, dec!(120.50));
        } else {
            panic!("Expected StockIndexUpdated event");
        }
    }

    #[tokio::test]
    async fn test_dispatch_multiple_events() {
        let event_log = Arc::new(Mutex::new(Vec::new()));
        let dispatcher =
            EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

        let events = vec![
            DomainEvent::StockRegistered {
                symbol: "2330".to_string(),
                name: "台積電".to_string(),
                market_id: 2,
                industry_id: 24,
                occurred_at: chrono::Local::now(),
            },
            DomainEvent::StockIdentityChanged {
                symbol: "2330".to_string(),
                old_name: "台積電".to_string(),
                new_name: "台積電新".to_string(),
                old_market_id: 2,
                new_market_id: 3,
                old_industry_id: 24,
                new_industry_id: 25,
                occurred_at: chrono::Local::now(),
            },
        ];

        dispatcher.dispatch_async(events).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let log = event_log.lock().await;
        assert_eq!(log.len(), 2, "應收到 2 個事件");
    }

    #[tokio::test]
    async fn test_dispatch_empty_events_is_noop() {
        let event_log = Arc::new(Mutex::new(Vec::new()));
        let dispatcher =
            EventDispatcher::new_with_handler(Some(fake_handler(event_log.clone())), 10);

        // 派發空事件列表
        dispatcher.dispatch_async(vec![]).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let log = event_log.lock().await;
        assert!(log.is_empty(), "空事件列表不應產生 any 處理");
    }

    #[tokio::test]
    async fn test_handler_error_does_not_panic() {
        // 建立一個會回傳錯誤的 handler
        let error_handler: EventHandlerFn =
            Box::new(|_event| Box::pin(async { Err(anyhow::anyhow!("模擬處理失敗")) }));

        let dispatcher = EventDispatcher::new_with_handler(Some(error_handler), 10);

        let events = vec![DomainEvent::StockRegistered {
            symbol: "9999".to_string(),
            name: "測試股".to_string(),
            market_id: 1,
            industry_id: 1,
            occurred_at: chrono::Local::now(),
        }];

        // 即使 handler 回傳錯誤，dispatch 本身不應 panic
        dispatcher.dispatch_async(events).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // 若執行到此處，代表錯誤被妥善處理而非 panic
    }

    #[tokio::test]
    async fn test_telegram_debouncer() {
        let debouncer = TelegramDebouncer::new();
        debouncer.add_message("msg1".to_string()).await;
        debouncer.add_message("msg2".to_string()).await;

        // 此時 buffer 應該有 2 個 msg
        {
            let buf = debouncer.buffer.lock().await;
            assert_eq!(buf.len(), 2);
        }

        // 等待超過 500 毫秒，批次發送應該會執行且排空 buffer
        tokio::time::sleep(tokio::time::Duration::from_millis(700)).await;
        {
            let buf = debouncer.buffer.lock().await;
            assert!(buf.is_empty());
        }
    }

    #[test]
    fn test_build_money_change_message_includes_hugo() {
        use crate::domain::money_flow::entity::MoneyFlowMemberWithPreviousDay;
        use chrono::NaiveDate;
        use rust_decimal_macros::dec;

        let date = NaiveDate::parse_from_str("2026-04-02", "%Y-%m-%d").unwrap();
        let previous_date = NaiveDate::parse_from_str("2026-04-01", "%Y-%m-%d").unwrap();
        let rows = vec![
            MoneyFlowMemberWithPreviousDay {
                date,
                previous_date: Some(previous_date),
                member_id: 0,
                market_value: dec!(4273187.20),
                previous_market_value: dec!(4053774.55),
            },
            MoneyFlowMemberWithPreviousDay {
                date,
                previous_date: Some(previous_date),
                member_id: 1,
                market_value: dec!(2195395.10),
                previous_market_value: dec!(2207807.70),
            },
            MoneyFlowMemberWithPreviousDay {
                date,
                previous_date: Some(previous_date),
                member_id: 2,
                market_value: dec!(1500000.00),
                previous_market_value: dec!(1400000.00),
            },
            MoneyFlowMemberWithPreviousDay {
                date,
                previous_date: Some(previous_date),
                member_id: 3,
                market_value: dec!(577792.10),
                previous_market_value: dec!(445966.85),
            },
        ];

        let msg =
            EventDispatcher::build_money_change_message(&rows).expect("message should be built");

        assert!(msg.contains("合計"));
        assert!(msg.contains("Eddie"));
        assert!(msg.contains("Unice"));
        assert!(msg.contains("Hugo"));
        assert!(msg.contains("4,273,187\\.20"));
        assert!(msg.contains("577,792\\.10"));
        assert!(msg.contains("\\-12,412\\.60"));
    }

    #[test]
    fn test_build_holding_dividend_message_groups_by_stock_and_member() {
        use crate::domain::portfolio::entity::StockOwnershipDetail;
        use chrono::{Local, NaiveDate, TimeZone};
        use rust_decimal_macros::dec;

        let today = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let stocks = vec![
            StockDividendInfo {
                stock_symbol: "2330".to_string(),
                name: "台積電".to_string(),
                stock_industry_id: crate::core::declare::Industry::Semiconductor.serial(),
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
                stock_industry_id: crate::core::declare::Industry::ElectronicComponents.serial(),
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

        let msg =
            EventDispatcher::build_holding_dividend_message(today, &stocks, &holdings).unwrap();

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
        use chrono::NaiveDate;
        use rust_decimal_macros::dec;

        let today = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let mut stocks = vec![
            StockDividendInfo {
                stock_symbol: "0050".to_string(),
                name: "元大台灣50".to_string(),
                stock_industry_id: crate::core::declare::Industry::ExchangeTradedFund.serial(),
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
                stock_industry_id: crate::core::declare::Industry::ElectronicComponents.serial(),
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
                stock_industry_id: crate::core::declare::Industry::ExchangeTradedFund.serial(),
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
                stock_industry_id: crate::core::declare::Industry::Semiconductor.serial(),
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

        EventDispatcher::sort_market_dividend_info(&mut stocks);
        let msg = EventDispatcher::build_market_dividend_message(
            today,
            "進行除權息的股票與 ETF 如下︰",
            &stocks,
        );

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
}
