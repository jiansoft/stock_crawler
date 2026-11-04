//! Yahoo 個股頁回 404 時的略過時間（H37）。
//!
//! 404 原本一律視為「證券已下市」，直接把略過快取拉長到
//! [`PAGE_NOT_FOUND_CACHE_TTL_SECONDS`]（30 天）。但 Yahoo 偶爾會對正常上市的股票暫時回 404
//! （2026-11-04 21:00 一次 39 檔，含 1216、2477，事後重打都是 200），這些股票因此 30 天不再回補。
//!
//! 現在改成：第一次 404 只記一個「曾經 404」標記（保留 [`NOT_FOUND_MARK_TTL_SECONDS`]），
//! 略過時間沿用呼叫端的一般值；標記期間內**再次** 404 才拉長到 30 天。抓取成功時清掉標記。
//! 標記以呼叫端的略過快取 key 加上 `:not_found` 為名，股利與財報各自計算。

use crate::infra::{
    crawler::yahoo::dividend::PAGE_NOT_FOUND_CACHE_TTL_SECONDS, nosql::redis::CLIENT,
};

/// 「曾經 404」標記的保留秒數（60 天）：這段期間內再次 404 才視為下市。
const NOT_FOUND_MARK_TTL_SECONDS: usize = 60 * 60 * 24 * 60;

/// 這次 404 之後的略過方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NotFoundSkip {
    /// 略過快取應設的秒數。
    pub(crate) ttl_seconds: usize,
    /// 標記期間內已經 404 過（視為已下市）；`false` 代表第一次，可能是暫時性。
    pub(crate) repeated: bool,
}

/// 日誌用的說明：連續 404 才說可能已下市。
pub(crate) fn not_found_note(repeated: bool) -> &'static str {
    if repeated {
        "連續 404，證券可能已下市"
    } else {
        "第一次 404，可能是 Yahoo 暫時性錯誤"
    }
}

/// 「曾經 404」標記的 key。
fn marker_key(cache_key: &str) -> String {
    format!("{cache_key}:not_found")
}

/// 依是否為標記期間內的第一次 404，決定略過時間。
fn decide(first_not_found: bool, normal_ttl_seconds: usize) -> NotFoundSkip {
    if first_not_found {
        NotFoundSkip {
            ttl_seconds: normal_ttl_seconds,
            repeated: false,
        }
    } else {
        NotFoundSkip {
            ttl_seconds: PAGE_NOT_FOUND_CACHE_TTL_SECONDS,
            repeated: true,
        }
    }
}

/// 記錄這次 404 並回傳略過方式；Redis 失敗時當成第一次（用一般略過時間，寧可早點重試）。
pub(crate) async fn on_not_found(cache_key: &str, normal_ttl_seconds: usize) -> NotFoundSkip {
    let marker = marker_key(cache_key);
    let first = match CLIENT
        .set_if_absent(&marker, true, NOT_FOUND_MARK_TTL_SECONDS)
        .await
    {
        Ok(first) => first,
        Err(why) => {
            tracing::warn!("寫入 Yahoo 404 標記失敗，當成第一次 404: key={marker}, error={why:#}");
            true
        }
    };
    decide(first, normal_ttl_seconds)
}

/// 處理一次 Yahoo 404：記錄標記；標記期間內再次 404 時把 `skip_keys` 全部拉長到 30 天。
///
/// 標記以 `skip_keys` 的第一個 key 為準（呼叫端的主要略過旗標）；呼叫前已用一般略過時間寫過旗標，
/// 所以第一次 404 不必再寫。回傳日誌用的說明（見 [`not_found_note`]）。
pub(crate) async fn handle_not_found(
    skip_keys: &[&str],
    normal_ttl_seconds: usize,
) -> &'static str {
    let Some(primary) = skip_keys.first() else {
        return not_found_note(false);
    };
    let skip = on_not_found(primary, normal_ttl_seconds).await;
    if skip.repeated {
        for key in skip_keys {
            if let Err(why) = CLIENT.set(*key, true, skip.ttl_seconds).await {
                tracing::error!("拉長 Yahoo 404 略過時間失敗: key={key}, error={why:#}");
            }
        }
    }
    not_found_note(skip.repeated)
}

/// 抓取成功時清掉「曾經 404」標記；失敗只記錄。
pub(crate) async fn on_success(cache_key: &str) {
    let marker = marker_key(cache_key);
    if let Err(why) = CLIENT.delete(&marker).await {
        tracing::warn!("清除 Yahoo 404 標記失敗: key={marker}, error={why:#}");
    }
}

#[cfg(test)]
mod tests;
