//! # 報價欄位解析
//!
//! 提供每日收盤報價各欄位的數值解析規則：包含「無資料」佔位符的判斷、
//! 嚴格與寬鬆兩種解析策略，以及「被拒絕資料列」比例的門檻檢查。
//! 這些規則讓「來源格式壞掉」與「真的沒有資料」可以被清楚區分。

use rust_decimal::Decimal;

use crate::infra::crawler::CrawlerError;

/// 每日收盤報價欄位解析錯誤。
///
/// 這個錯誤型別讓「來源格式壞掉」與「真的沒有資料」可以被區分開來：
///
/// - [`QuoteParseError::MissingField`]：來源少了必要欄位，通常代表對方改了
///   欄位名稱或順序——舊版程式會默默補 0，導致半套資料寫進資料庫。
/// - [`QuoteParseError::InvalidDecimal`]：欄位內容不是數字、也不是已知的
///   「無資料」佔位符（例如 `--`），代表內容被污染或格式變更。
///
/// 兩種情況都應該讓該列資料被「拒絕」而不是以零值入庫；
/// 呼叫端再依拒絕比例決定要跳過少數壞列，還是整批失敗。
#[derive(Debug, thiserror::Error)]
pub enum QuoteParseError {
    /// 來源資料缺少必要欄位（欄位名稱不存在，或該列長度不足）。
    #[error("missing field `{field}` in quote row")]
    MissingField {
        /// 缺少的欄位名稱。
        field: &'static str,
    },
    /// 欄位內容無法解析成數值，且不是已知的「無資料」佔位符。
    #[error("invalid decimal for field `{field}`: `{raw}`")]
    InvalidDecimal {
        /// 解析失敗的欄位名稱。
        field: &'static str,
        /// 原始字串內容，保留下來方便對照來源網頁除錯。
        raw: String,
        /// 底層的 decimal 解析錯誤（保留 source chain）。
        #[source]
        source: rust_decimal::Error,
    },
}

/// 判斷欄位內容是否為來源的「無資料」佔位符。
///
/// TWSE/TPEx 對「當日無成交、無委買賣」的價格欄位會回傳 `--`（或多個連字號）、
/// 空字串，部分來源用 `N/A`。這些是合法的「沒有值」，應轉成 0 而不是解析錯誤；
/// 注意負數如 `-5.00` 含有數字，不會被此規則誤判。
fn is_no_data_placeholder(raw: &str) -> bool {
    let trimmed = raw.trim();
    trimmed.is_empty() || trimmed.chars().all(|c| c == '-') || trimmed.eq_ignore_ascii_case("n/a")
}

/// 解析單一報價欄位為 `Decimal`。
///
/// 規則（依序）：
/// 1. 「無資料」佔位符（`--`、空白、`N/A`）→ 回傳 0，這是合法情況。
/// 2. 移除千分位逗號後可解析 → 回傳數值。
/// 3. 其餘 → 回傳 [`QuoteParseError::InvalidDecimal`]，讓呼叫端拒絕該列，
///    而不是像舊版 `unwrap_or_default()` 那樣默默寫入 0。
pub(crate) fn parse_quote_decimal(
    field: &'static str,
    raw: &str,
) -> Result<Decimal, QuoteParseError> {
    if is_no_data_placeholder(raw) {
        return Ok(Decimal::ZERO);
    }

    // 千分位逗號（例如 "1,234,567"）不是數字的一部分，先移除再解析。
    let cleaned = raw.replace(',', "");
    cleaned
        .trim()
        .parse::<Decimal>()
        .map_err(|source| QuoteParseError::InvalidDecimal {
            field,
            raw: raw.to_owned(),
            source,
        })
}

/// 解析「軟性」報價欄位：解析失敗時記 warning 並回傳 0，而不是拒絕整列。
///
/// 用於「漲跌價差」這類欄位——除權息日等特殊情況下，來源可能在此欄放
/// 非數字標記。漲跌為 0 的傷害有限（漲跌幅稍後會用前一交易日收盤價重算），
/// 為了這一欄拒絕整列反而會遺失開高低收等重要資料，因此採取寬鬆策略。
/// 價格與量能欄位仍使用嚴格的 [`parse_quote_decimal`]。
pub(super) fn parse_soft_quote_decimal(field: &'static str, raw: &str) -> Decimal {
    match parse_quote_decimal(field, raw) {
        Ok(value) => value,
        Err(why) if is_ex_rights_marker(raw) => {
            // 除權息當天證交所在漲跌欄放「除權」「除息」，每天都會出現，屬預期情況。
            tracing::debug!("quote field fallback to zero: {why}");
            Decimal::ZERO
        }
        Err(why) => {
            tracing::warn!("quote field fallback to zero: {why}");
            Decimal::ZERO
        }
    }
}

/// 是否為除權息當天的漲跌欄標記（`除權`、`除息`、`除權息`）。
fn is_ex_rights_marker(raw: &str) -> bool {
    matches!(raw.trim(), "除權" | "除息" | "除權息")
}

/// 檢查「被拒絕的資料列」比例是否仍在容忍範圍內。
///
/// 單一壞列可能只是來源偶發雜訊，跳過即可；但大量壞列幾乎可以肯定是
/// 來源格式變更（欄位改名、欄位順序調整）。此時寧可讓整批抓取失敗、
/// 觸發告警請人來調查，也不要把大量缺漏的行情資料寫進資料庫。
///
/// 門檻：拒絕比例超過 10% 即回傳錯誤；低於門檻時只記 warning。
pub(crate) fn ensure_rejected_rows_within_threshold(
    source: &str,
    rejected: usize,
    total: usize,
) -> Result<(), CrawlerError> {
    if rejected == 0 {
        return Ok(());
    }

    // rejected * 10 > total 等價於 rejected / total > 10%，用整數運算避免浮點誤差。
    if rejected * 10 > total {
        return Err(CrawlerError::Parse(format!(
            "{source}: rejected {rejected}/{total} quote rows, source format may have changed"
        )));
    }

    tracing::warn!("{source}: rejected {rejected}/{total} quote rows during parsing");
    Ok(())
}

#[cfg(test)]
mod tests;
