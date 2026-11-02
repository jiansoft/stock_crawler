//! 類股 API 的分頁抓取：依 `offset` 逐頁請求、解析並累計快照，處理首頁空白、分頁中斷與無法解析的頁面。

use std::{collections::HashMap, future::Future};

use anyhow::{Result, anyhow};
use reqwest::StatusCode;

use crate::{
    core::util, infra::cache::RealtimeSnapshot, infra::crawler::yahoo::YahooClassCategory,
};

/// 回應 DTO 與欄位解析。
use super::parse::{ClassQuotesResponse, parse_class_quote_item};
use super::{
    CategoryFetchDiagnostics, CategoryFetchResult, build_class_quotes_api_url,
    build_class_quotes_headers, get_yahoo_class_quotes_client,
};

/// 單一分頁抓取後的最小結果。
///
/// 這裡只保留背景輪詢下一步真正需要的資料，
/// 避免把完整 JSON 結構與整頁字串生命週期拉長到頁外。
pub(super) struct ClassQuotesPageResult {
    pub(super) snapshots: Vec<(String, RealtimeSnapshot)>,
    pub(super) raw_item_count: usize,
    pub(super) results_total: usize,
    pub(super) next_offset: Option<usize>,
}

/// 分頁迴圈本體：`fetch_page` 抓指定 offset 的單頁、`pause` 是頁與頁之間的停頓，
/// 測試時換成假資料與不等待。錯誤條件同 [`fetch_category_snapshots`]。
pub(super) async fn collect_category_pages<F, FF, P, PF>(
    category: &YahooClassCategory,
    fetch_page: F,
    pause: P,
) -> Result<CategoryFetchResult>
where
    F: Fn(usize) -> FF,
    FF: Future<Output = Result<ClassQuotesPageResult>>,
    P: Fn() -> PF,
    PF: Future<Output = ()>,
{
    // 用 symbol -> snapshot 收集完整類股結果，
    // 後面若同頁或跨頁出現重複代號，後寫入的值會覆蓋前值。
    let mut snapshots = HashMap::new();
    // `offset` 是 Yahoo 分頁的游標；首頁永遠從 0 開始。
    let mut offset = 0usize;
    // `page` 只用來判斷「首頁是否為空」這種特殊錯誤情境。
    let mut page = 0usize;
    let mut page_count = 0usize;
    let mut raw_item_count = 0usize;

    loop {
        // 每一圈只抓一頁，抓完再看 `nextOffset` 決定要不要繼續。
        let page_result = fetch_page(offset).await?;
        let list_len = page_result.raw_item_count;
        page_count += 1;
        raw_item_count += list_len;

        // 首頁就空通常代表 URL 參數、來源行為或類股設定有問題，
        // 這種情況不應該靜默吞掉，直接回錯讓呼叫端知道這個類股整輪失敗。
        if list_len == 0 && page == 0 {
            let error_message = format!(
                "Yahoo 類股 API 首頁為空: {} {}({})",
                category.exchange.label(),
                category.name,
                category.sector_id
            );
            tracing::error!("{}", error_message.clone());
            return Err(anyhow!(error_message));
        }

        // 首頁的 `resultsTotal` 最接近這個類股的最終容量，
        // 提前保留空間可以減少整輪插入時的 rehash 與額外配置。
        if page == 0 && page_result.results_total > 0 {
            snapshots.reserve(page_result.results_total);
        }

        // 單頁內每一筆 Yahoo item 都轉成專案內部的 `RealtimeSnapshot`。
        // 無法辨識股票代號的資料會被 `parse_class_quote_item` 主動略過。
        for (symbol, snapshot) in page_result.snapshots {
            snapshots.insert(symbol, snapshot);
        }

        // 只有 `nextOffset` 真正往前推進時才繼續抓下一頁，
        // 這可以避免來源異常回傳相同 offset 造成無窮迴圈。
        match page_result.next_offset {
            Some(next_offset) if next_offset > offset => {
                // 同一個類股若有大量分頁，也要在頁與頁之間停一下，
                // 避免單一 sector 連續請求過密而被 Yahoo 視為異常流量。
                pause().await;
                offset = next_offset;
                page += 1;
            }
            _ => break,
        }
    }

    // 如果整個類股抓完卻一檔都解析不出來，代表不是單頁偶發缺值，
    // 而是整個類股資料格式可能有變，這時候直接回錯比較安全。
    if snapshots.is_empty() {
        let error_message = format!(
            "Yahoo 類股 API 未解析出任何股票: {} {}({})",
            category.exchange.label(),
            category.name,
            category.sector_id
        );
        tracing::error!("{}", error_message.clone());
        return Err(anyhow!(error_message));
    }

    Ok(CategoryFetchResult {
        diagnostics: CategoryFetchDiagnostics {
            page_count,
            raw_item_count,
            snapshot_count: snapshots.len(),
        },
        snapshots,
    })
}

/// 抓取單一類股分頁的原始 JSON 回應。
///
/// # Errors
/// - HTTP 失敗。
/// - JSON 解析失敗。
/// - 非首頁卻出現「有總筆數但空列表」的異常資料。
pub(super) async fn fetch_class_quotes_page(
    category: &YahooClassCategory,
    offset: usize,
) -> Result<ClassQuotesPageResult> {
    // 先把 URL 組在一起，讓日誌與測試都能重用同一套規則。
    let url = build_class_quotes_api_url(category, offset);
    // 這裡直接拿 response bytes 後立刻解析，
    // 讓 borrowed JSON 欄位可以只活在單頁處理期間，降低暫時配置壓力。
    let response = util::http::get_response_with_client(
        get_yahoo_class_quotes_client()?,
        &url,
        Some(build_class_quotes_headers(category)?),
    )
    .await?;
    let status = response.status();
    let response_body = response.bytes().await.map_err(|e| {
        anyhow!(
            "Error reading Yahoo class quote response body from {}: {}",
            url,
            e
        )
    })?;
    parse_class_quotes_page(category, offset, &url, status, response_body.as_ref())
}

/// 檢查並解析單頁回應（HTTP 狀態、JSON、空頁規則）；不碰網路，測試可直接餵資料。
///
/// # Errors
/// - HTTP 狀態不是成功。
/// - JSON 解析失敗或 `nextOffset` 格式錯誤。
/// - 非首頁卻出現「有總筆數但空列表」的異常資料。
pub(super) fn parse_class_quotes_page(
    category: &YahooClassCategory,
    offset: usize,
    url: &str,
    status: StatusCode,
    response_body: &[u8],
) -> Result<ClassQuotesPageResult> {
    let response_body_preview = String::from_utf8_lossy(response_body);

    if !status.is_success() {
        return Err(anyhow!(
            "Yahoo 類股 API request failed with status {} for {}. Body: {}",
            status,
            url,
            util::text::log_snippet(&response_body_preview, 200)
        ));
    }

    let response: ClassQuotesResponse<'_> = serde_json::from_slice(response_body).map_err(|e| {
        anyhow!(
            "Failed to parse Yahoo 類股 API response from {}: {:?}. Body: {}",
            url,
            e,
            util::text::log_snippet(&response_body_preview, 200)
        )
    })?;
    let raw_item_count = response.list.len();
    let results_total = response.pagination.results_total;
    let next_offset = response.pagination.next_offset()?;

    // 就算首頁沒有直接回錯，也先把「整頁空資料」寫到日誌，
    // 方便後續人工從 log 追查是 Yahoo schema 變更、類股失效還是被擋流量。
    if raw_item_count == 0 {
        tracing::error!(
            "Yahoo 類股 API 回空資料: {} {}({}) offset={} resultsTotal={} url={}",
            category.exchange.label(),
            category.name,
            category.sector_id,
            offset,
            results_total,
            url
        );
    }

    // 非首頁如果宣稱有總筆數卻回空列表，通常代表中間頁資料異常，
    // 直接報錯，避免把「只抓到前半段」當成正常成功。
    if raw_item_count == 0 && results_total > 0 && offset > 0 {
        return Err(anyhow!(
            "Yahoo 類股 API offset={} 返回空列表但 resultsTotal={}",
            offset,
            results_total
        ));
    }

    let mut snapshots = Vec::with_capacity(raw_item_count);
    for item in &response.list {
        if let Some((symbol, snapshot)) = parse_class_quote_item(item)? {
            snapshots.push((symbol, snapshot));
        }
    }

    Ok(ClassQuotesPageResult {
        snapshots,
        raw_item_count,
        results_total,
        next_offset,
    })
}
