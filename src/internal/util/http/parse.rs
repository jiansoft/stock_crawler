use core::result::Result::Ok;
use scraper::Selector;

/// 解析元素的值
pub fn element_value(element: &scraper::ElementRef, css_selector: &str) -> Option<String> {
    match Selector::parse(css_selector) {
        Ok(s) => element
            .select(&s)
            .next()
            .map(|v| v.text().collect::<String>()),
        Err(_) => None,
    }
}
