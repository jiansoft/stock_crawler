//! # 領域事件派發器模組
//!
//! 負責接收領域事件 ([`DomainEvent`]) 並在背景非同步處理對應的副作用，
//! 例如 Telegram 通知與 gRPC 同步推送。
//! 此模組的目的是將核心業務邏輯 (Use Case) 與外部副作用解耦，
//! 使 Use Case 僅負責商業編排，不直接耦合基礎設施。
//!
//! 事件類型對應的副作用處理拆分至子模組：
//! - [`debounce`]：Telegram 訊息防震批次發送器。
//! - [`money_flow`]：`MoneyFlowRecalculated` 市值變化通知。
//! - [`ex_dividend`]：`ExDividendReminderTriggered` 除權息提醒與持股股利通知。

use std::sync::Arc;
use std::sync::OnceLock;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::domain::events::DomainEvent;

use debounce::TelegramDebouncer;

/// Telegram 訊息防震批次發送器子模組。
mod debounce;
/// 除權息提醒事件處理子模組。
mod ex_dividend;
/// 資金流（市值變化）事件處理子模組。
mod money_flow;

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
            tracing::error!(
                "Failed to push_stock_info_to_go_service for {} because {:?}",
                symbol,
                why
            );
        }
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
            old_nav: rust_decimal_macros::dec!(90.0),
            new_nav: rust_decimal_macros::dec!(95.12),
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
            assert_eq!(old_nav, rust_decimal_macros::dec!(90.0));
            assert_eq!(new_nav, rust_decimal_macros::dec!(95.12));
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
            index: rust_decimal_macros::dec!(16500.25),
            change: rust_decimal_macros::dec!(120.50),
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
            assert_eq!(index, rust_decimal_macros::dec!(16500.25));
            assert_eq!(change, rust_decimal_macros::dec!(120.50));
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
}
