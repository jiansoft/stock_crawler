//! Yahoo 類股 API 的錯誤分類：WAF 阻擋（需要整輪冷卻與警報）與 5xx 暫時性錯誤（只記 warn）。

use std::time::Duration;

use crate::infra::{
    cache::{TTL, TtlCacheInner},
    crawler::yahoo::YahooClassCategory,
};

/// 是否為 Yahoo WAF 阻擋（`Request denied` 或 999 狀態碼）。
///
/// 阻擋綁在來源端（IP／client）而非個別類股，遇到就要中止整輪並冷卻。
pub(super) fn is_denied(message: &str) -> bool {
    message.contains("Request denied") || message.contains("status 999")
}

/// 遭遇 WAF 阻擋時發出警報；以記憶體 TTL 旗標確保 1 小時內只發一次。
pub(super) fn alert_denied_once(category: &YahooClassCategory, err_msg: &str) {
    let alert_cache_key = "alert:yahoo:denied";
    // 使用專案內建的記憶體 TTL 快取，確認 1 小時內是否已發送過警報，防範洗板
    let already_alerted = TTL.daily_quote_contains_key(alert_cache_key);

    if !already_alerted {
        // 寫入為期 1 小時的警報快取旗標至記憶體 TTL 快取 (3600秒)
        TTL.daily_quote_set(
            alert_cache_key.to_string(),
            "true".to_string(),
            Duration::from_secs(3600),
        );

        // 準備 TG 通知所需的變數，轉移所有權至 async 區塊
        let exchange_label = category.exchange.label().to_string();
        let category_name = category.name.to_string();
        let sector_id = category.sector_id;
        let err_msg_clone = err_msg.to_string();

        // 透過 tokio::spawn 非同步發送警報，避免阻塞爬蟲主流程。
        // 走 core::alert 抽象介面（實際管道由 main 註冊的 adapter 決定），
        // infra 層不再直接依賴 interfaces::bot（反向耦合已移除）。
        tokio::spawn(async move {
            crate::core::alert::send_alert(
                "Yahoo 類股採集遭遇阻擋",
                &format!(
                    "類股: {} {}({})\n原因: {}\n該次更新將強制冷卻 10 分鐘，1小時內不重複提醒。",
                    exchange_label, category_name, sector_id, err_msg_clone
                ),
            )
            .await;
        });
    }
}

/// 是否為 Yahoo 類股 API 回應的 5xx 暫時性錯誤（見 `class_quote` 的錯誤格式）。
///
/// 2026-09-24 盤中共 12 次 500／502，皆在 09:02～09:37，下一輪即恢復。
pub(super) fn is_transient_server_error(message: &str) -> bool {
    message.contains("request failed with status 5")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_transient_server_error_matches_only_5xx() {
        assert!(is_transient_server_error(
            "Yahoo 類股 API request failed with status 500 Internal Server Error for https://x. Body: "
        ));
        assert!(is_transient_server_error(
            "Yahoo 類股 API request failed with status 502 Bad Gateway for https://x. Body: "
        ));
        assert!(!is_transient_server_error(
            "Yahoo 類股 API request failed with status 404 Not Found for https://x. Body: "
        ));
        assert!(!is_transient_server_error("Request denied"));
    }

    /// 遭遇阻擋後寫入 1 小時的警報旗標；旗標還在時再次呼叫不會重設也不會出錯。
    #[tokio::test]
    async fn alert_denied_once_sets_the_alert_flag() {
        let category = YahooClassCategory::enabled(
            crate::infra::crawler::yahoo::YahooClassExchange::Listed,
            40,
            "半導體",
        );

        alert_denied_once(&category, "Request denied");
        assert!(TTL.daily_quote_contains_key("alert:yahoo:denied"));
        alert_denied_once(&category, "Request denied");
        assert!(TTL.daily_quote_contains_key("alert:yahoo:denied"));
    }

    #[test]
    fn is_denied_matches_waf_responses_only() {
        assert!(is_denied("Yahoo 類股 API Request denied by WAF"));
        assert!(is_denied(
            "Yahoo 類股 API request failed with status 999 Unknown for https://x. Body: "
        ));
        assert!(!is_denied(
            "Yahoo 類股 API request failed with status 500 Internal Server Error for https://x. Body: "
        ));
    }
}
