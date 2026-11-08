use crate::core::declare::Industry;
use crate::infra::cache::SHARE;
use crate::infra::crawler::share::EtfInfo;
use crate::infra::crawler::twse::international_securities_identification_number::InternationalSecuritiesIdentificationNumber;
use crate::infra::crawler::twse::suspend_listing::SuspendListing;

/// 註冊或變更證券識別資料命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterStockCommand {
    /// 證券代號
    pub symbol: String,
    /// 證券名稱
    pub name: String,
    /// 交易所市場 ID
    pub market_id: i32,
    /// 產業分類 ID
    pub industry_id: i32,
}

/// 終止上市 (下市) 處理命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelistedCompanyCommand {
    /// 證券代號
    pub symbol: String,
}

/// ISIN 爬蟲資料防腐層轉譯器。
pub struct IsinAclMapper;

impl IsinAclMapper {
    /// 將爬蟲取得的 ISIN 原始 DTO 轉譯成系統內部的 `RegisterStockCommand`。
    ///
    /// # 安全防護與過濾
    /// - 若 `industry_id` 為 `0` (代表未分類、非法或解析錯誤的產業資料)，此函式會回傳 `None` 進行過濾阻擋。
    /// - 市場別不在 `stock_exchange_market` 對照表（例如公開發行）時回傳 `None`，不寫入主檔。
    pub fn from_isin(
        dto: &InternationalSecuritiesIdentificationNumber,
    ) -> Option<RegisterStockCommand> {
        let industry_id = resolve_industry(
            SHARE.get_industry_id(&dto.industry).unwrap_or(0),
            &dto.stock_symbol,
            |parent| {
                SHARE
                    .stocks
                    .read()
                    .ok()?
                    .get(parent)
                    .map(|stock| stock.industry_id())
            },
        );
        // 1. 過濾非法、未分類產業資料 (industry_id == 0) 或 ETF (9001) / ETN (9002)
        if industry_id == 0 || industry_id == 9001 || industry_id == 9002 {
            return None;
        }

        // 對照表只有上市、上櫃、興櫃；公開發行（1）查不到就略過，不可 unwrap（會讓 ISIN 排程 panic）。
        let market_id = SHARE
            .get_exchange_market(dto.market.serial())?
            .stock_exchange_market_id;

        // 2. 轉換為內部的 RegisterStockCommand
        Some(RegisterStockCommand {
            symbol: dto.stock_symbol.clone(),
            name: dto.name.clone(),
            market_id,
            industry_id,
        })
    }
}

/// 特別股代號（4 碼數字後接大寫英文字母，例如 1101B、8349A、2887Z1）對應的普通股代號。
fn preferred_parent(symbol: &str) -> Option<&str> {
    let (base, suffix) = symbol.split_at_checked(4)?;
    let is_preferred = base.bytes().all(|byte| byte.is_ascii_digit())
        && suffix
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase());
    is_preferred.then_some(base)
}

/// ISIN 沒有產業別而落在「未分類」的特別股，沿用普通股的產業；查不到或普通股也未分類時維持原值。
fn resolve_industry(
    industry_id: i32,
    symbol: &str,
    parent_industry: impl Fn(&str) -> Option<i32>,
) -> i32 {
    let uncategorized = Industry::Uncategorized.serial();
    if industry_id != uncategorized {
        return industry_id;
    }
    preferred_parent(symbol)
        .and_then(parent_industry)
        .filter(|id| *id != 0 && *id != uncategorized)
        .unwrap_or(industry_id)
}

/// 下市公司爬蟲資料防腐層轉譯器。
pub struct DelistedCompanyAclMapper;

impl DelistedCompanyAclMapper {
    /// 將下市公司原始 DTO 轉譯成 `DelistedCompanyCommand`。
    ///
    /// # 資料清洗與過濾
    /// - 若下市日期格式無效（長度不足、開頭不是可解析的民國年），回傳 `None`。
    /// - 過濾掉民國 110 年之前的下市資料，以縮小回補與更新範圍。
    pub fn from_suspend_listing(dto: &SuspendListing) -> Option<DelistedCompanyCommand> {
        // 擷取民國年前 3 碼並解析為整數。
        // 用 .get(..3) 取代 [..3]：外部資料可能混入多位元組字元（如中文），
        // byte 3 落在字元中間時，[..3] 會 panic，而 .get 只會回傳 None；
        // 長度不足 3 bytes 時 .get 也會回傳 None，因此不需另做長度檢查。
        let year = dto.delisting_date.get(..3)?.parse::<i32>().ok()?;
        if year < 110 {
            return None;
        }

        Some(DelistedCompanyCommand {
            symbol: dto.stock_symbol.clone(),
        })
    }
}

/// ETF 爬蟲資料防腐層轉譯器。
pub struct EtfAclMapper;

impl EtfAclMapper {
    /// 將原始 ETF DTO 轉譯成 `RegisterStockCommand`。
    pub fn from_etf(dto: &EtfInfo) -> RegisterStockCommand {
        let market_id = SHARE
            .get_exchange_market(dto.market.serial())
            .unwrap()
            .stock_exchange_market_id;
        RegisterStockCommand {
            symbol: dto.stock_symbol.clone(),
            name: dto.name.clone(),
            market_id,
            industry_id: dto.industry_id,
        }
    }
}

#[cfg(test)]
mod tests;
