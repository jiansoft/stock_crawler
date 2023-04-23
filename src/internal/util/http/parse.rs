use core::result::Result::Ok;
use scraper::Selector;

/// Extracts the text value of an element selected by a given CSS selector.
///
/// This function takes a reference to a `scraper::ElementRef` and a CSS selector as input,
/// and attempts to find the corresponding element. If the element is found, its text content
/// is returned. If the CSS selector is invalid or the element cannot be found, the function
/// returns `None`.
///
/// # Arguments
///
/// * `element` - A reference to a `scraper::ElementRef` from which the text value is to be extracted.
/// * `css_selector` - A string slice representing the CSS selector used to find the element.
///
/// # Examples
///
/// ```
/// use scraper::{Html, Selector, ElementRef};
/// use your_crate::element_value;
///
/// let html = r#"<div class="example">Hello, world!</div>"#;
/// let document = Html::parse_document(html);
/// let element: ElementRef = document.select(Selector::parse("div.example").unwrap()).next().unwrap();
///
/// let text = element_value(&element, "div.example");
/// assert_eq!(text, Some("Hello, world!".to_string()));
/// ```
pub fn element_value(element: &scraper::ElementRef, css_selector: &str) -> Option<String> {
    match Selector::parse(css_selector) {
        Ok(s) => element
            .select(&s)
            .next()
            .map(|v| v.text().collect::<String>()),
        Err(_) => None,
    }
}
