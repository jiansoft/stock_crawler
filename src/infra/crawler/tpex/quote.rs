use crate::{
    core::declare::StockExchange,
    core::util,
    infra::cache::{TTL, TtlCacheInner},
    infra::crawler::{
        share,
        share::{DailyQuoteDto, change_range_percent},
        tpex,
    },
};
use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use hashbrown::HashMap;
use serde::{Deserialize, Serialize};

// QuoteResponse 上櫃公司每日收盤資訊
/*#[derive(Debug, Deserialize)]
struct QuoteResponse {
    //pub report_date: String,
    #[serde(rename = "aaData")]
    pub aa_data: Vec<Vec<String>>,
    // i_total_records: i32,
}*/

// QuoteResponse 上櫃公司每日收盤資訊
#[derive(Serialize, Deserialize, Debug)]
pub struct QuoteResponse {
    #[serde(rename = "tables")]
    pub tables: Vec<Table>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Table {
    #[serde(rename = "data")]
    pub data: Option<Vec<Vec<String>>>,
}

/// PeRatioAnalysis 上櫃股票個股本益比、殖利率、股價淨值比
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PeRatioAnalysisResponse {
    /// 證券代號
    #[serde(rename = "SecuritiesCompanyCode")]
    pub security_code: String,
    /// 本益比
    #[serde(rename = "PriceEarningRatio")]
    pub price_earning_ratio: String,
    /// 資料日期（民國年 `yyyMMdd`，例如 `1151012`）。這支 openapi 只給最新公布日的資料，
    /// 收盤時（15:00）通常還是前一個交易日，重抓歷史日期時更會拿到當天的值。
    #[serde(rename = "Date", default)]
    pub date: String,
}

/// 抓取上櫃公司每日收盤資訊
pub async fn visit(date: NaiveDate) -> Result<Vec<DailyQuoteDto>> {
    let pe_ratio_url = "https://www.tpex.org.tw/openapi/v1/tpex_mainboard_peratio_analysis";
    // 本益比
    let pe_ratio_response =
        util::http::get_json::<Vec<PeRatioAnalysisResponse>>(pe_ratio_url).await?;

    let republic_date = util::datetime::gregorian_year_to_roc_year(date.year());
    //https://www.tpex.org.tw/web/stock/aftertrading/daily_close_quotes/stk_quote_result.php?l=zh-tw&_=1681801169006
    let quote_url = format!(
        "https://{}/web/stock/aftertrading/otc_quotes_no1430/stk_wn1430_result.php?l=zh-tw&d={}{}&se=EW&_={}",
        tpex::HOST,
        republic_date,
        date.format("/%m/%d"),
        date
    );

    let quote_response = util::http::get_json::<QuoteResponse>(&quote_url).await?;
    parse_quote_response(quote_response, pe_ratio_response, date).await
}

/// 解析 TPEX 每日收盤資訊與本益比資料，並將其轉換為 `DailyQuoteDto` 列表。
///
/// # 參數
/// * `quote_response` - TPEX 每日收盤資訊的回應資料。
/// * `pe_ratio_response` - TPEX 本益比分析的原始資料列表。
/// * `date` - 資料所屬的日期。
///
/// # 傳回值
/// 回傳解析後的 `DailyQuoteDto` 向量。
pub async fn parse_quote_response(
    quote_response: QuoteResponse,
    pe_ratio_response: Vec<PeRatioAnalysisResponse>,
    date: NaiveDate,
) -> Result<Vec<DailyQuoteDto>> {
    // 只採用資料日期就是 `date` 的本益比；不是當天的值留 0，交給官方補值排程
    // （app::backfill::official_ratio，17:30、20:30 以櫃買當日資料覆寫）補上。
    let expected_date = format!(
        "{}{}",
        util::datetime::gregorian_year_to_roc_year(date.year()),
        date.format("%m%d")
    );
    let mut pe_ratio_analysis: HashMap<String, PeRatioAnalysisResponse> =
        HashMap::with_capacity(pe_ratio_response.len());
    let mut stale_date = None;
    for item in pe_ratio_response {
        if item.date == expected_date {
            pe_ratio_analysis.insert(item.security_code.to_string(), item);
        } else {
            stale_date.get_or_insert(item.date);
        }
    }
    if let Some(stale_date) = stale_date {
        tracing::info!(
            "上櫃本益比 openapi 的資料日期 {stale_date} 不是 {expected_date}，本益比留待官方補值排程"
        );
    }

    let mut dqs: Vec<DailyQuoteDto> = Vec::with_capacity(2048);
    if !quote_response.tables.is_empty()
        && let Some(tpex_dqs) = &quote_response.tables[0].data
    {
        // 統計解析失敗而被拒絕的資料列數；結尾會依比例決定只警告還是整批失敗。
        let mut rejected_rows = 0usize;

        for item in tpex_dqs {
            // 解析失敗（欄位缺失或數值格式錯誤）時拒絕該列，不再默默補 0——
            // 半套零值資料入庫會污染均線、估價等後續計算。
            let mut dto = match DailyQuoteDto::from_with_exchange(StockExchange::TPEx, item, date) {
                Ok(dto) => dto,
                Err(why) => {
                    rejected_rows += 1;
                    // 只記錄前幾筆的完整內容，避免來源整批壞掉時 log 洗版。
                    if rejected_rows <= 5 {
                        tracing::warn!("TPEx quote row rejected: {why}, row={item:?}");
                    }
                    continue;
                }
            };

            if dto.closing_price.is_zero()
                && dto.highest_price.is_zero()
                && dto.lowest_price.is_zero()
                && dto.opening_price.is_zero()
            {
                continue;
            }

            let daily_quote_memory_key = format!("{}-{}", date.format("%Y%m%d"), dto.symbol);

            if TTL.daily_quote_contains_key(&daily_quote_memory_key) {
                continue;
            }

            // 漲跌幅以交易所的參考價（收盤 − 漲跌）為基準，與官方一致。
            dto.change_range = change_range_percent(dto.closing_price, dto.change);

            // 本益比來自另一支 API，屬「補充」欄位：解析失敗只記 warning 並保持 0，
            // 不值得為它拒絕整列行情（開高低收與量能仍是完整有效的）。
            // "N/A"、"-" 等無資料佔位符會由 parse_quote_decimal 規則化為 0。
            if let Some(pe_ratio_analysis_response) = pe_ratio_analysis.get(&dto.symbol) {
                match share::parse_quote_decimal(
                    "本益比",
                    &pe_ratio_analysis_response.price_earning_ratio,
                ) {
                    Ok(value) => dto.price_earning_ratio = value,
                    Err(why) => {
                        tracing::warn!("TPEx PE ratio fallback to zero for {}: {why}", dto.symbol);
                    }
                }
            }

            dqs.push(dto);
        }

        // 拒絕比例超過門檻（10%）代表來源格式極可能已變更：
        // 整批失敗讓呼叫端告警調查，避免大量缺漏資料悄悄入庫。
        share::ensure_rejected_rows_within_threshold("TPEx", rejected_rows, tpex_dqs.len())?;
    }

    Ok(dqs)
}

#[cfg(test)]
mod tests;
