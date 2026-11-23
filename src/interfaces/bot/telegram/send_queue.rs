//! 行程內分聊天室排隊；等待某一聊天室的間隔時，不阻擋其他聊天室取得全域名額。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// 每聊天室一秒一則，全域每 40 毫秒一則（每秒最多 25 則）。
#[derive(Default)]
pub(super) struct SendQueue {
    chats: Mutex<HashMap<i64, Arc<Mutex<Option<Instant>>>>>,
    global: Mutex<Option<Instant>>,
}

impl SendQueue {
    /// 依聊天室 FIFO 等待，再取得全域名額；所有重送與切段也經過這裡。
    pub(super) async fn acquire(&self, chat_id: i64) {
        let chat = {
            let mut chats = self.chats.lock().await;
            // 只回收沒有等待者且已閒置五分鐘的鎖，避免大量歷史聊天室永久累積。
            if chats.len() > 1024 {
                chats.retain(|_, queue| {
                    Arc::strong_count(queue) > 1
                        || queue
                            .try_lock()
                            .map(|last| {
                                last.is_some_and(|at| at.elapsed() < Duration::from_secs(300))
                            })
                            .unwrap_or(true)
                });
            }
            Arc::clone(chats.entry(chat_id).or_default())
        };
        let mut last_chat = chat.lock().await;
        wait_interval(*last_chat, Duration::from_secs(1)).await;
        let mut last_global = self.global.lock().await;
        wait_interval(*last_global, Duration::from_millis(40)).await;
        let now = Instant::now();
        *last_global = Some(now);
        *last_chat = Some(now);
    }
}

/// 已過間隔不需等待；使用 Tokio 時鐘，測試可直接推進時間。
async fn wait_interval(last: Option<Instant>, interval: Duration) {
    if let Some(last) = last {
        tokio::time::sleep_until(last + interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 單一聊天室一秒一則；不同聊天室只受 40ms 全域間隔限制。
    #[tokio::test(start_paused = true)]
    async fn per_chat_and_global_limits_are_independent() {
        let queue = SendQueue::default();
        let start = Instant::now();
        queue.acquire(1).await;
        queue.acquire(2).await;
        assert_eq!(start.elapsed(), Duration::from_millis(40));
        queue.acquire(1).await;
        assert_eq!(start.elapsed(), Duration::from_secs(1));
    }

    /// 第二則在聊天室 1 排隊時，聊天室 2 仍可先取得名額。
    #[tokio::test(start_paused = true)]
    async fn busy_chat_does_not_block_other_chats() {
        let queue = Arc::new(SendQueue::default());
        queue.acquire(1).await;
        let start = Instant::now();
        let busy = Arc::clone(&queue);
        let pending = tokio::spawn(async move {
            busy.acquire(1).await;
        });
        tokio::task::yield_now().await;
        queue.acquire(2).await;
        assert_eq!(start.elapsed(), Duration::from_millis(40));
        pending.await.expect("聊天室排隊");
        assert_eq!(start.elapsed(), Duration::from_secs(1));
    }

    /// 不同聊天室的第 26 則至少在一秒後開始送出。
    #[tokio::test(start_paused = true)]
    async fn twenty_six_messages_need_at_least_one_second() {
        let queue = SendQueue::default();
        let start = Instant::now();
        for id in 0..26 {
            queue.acquire(id).await;
        }
        assert_eq!(start.elapsed(), Duration::from_secs(1));
    }
}
