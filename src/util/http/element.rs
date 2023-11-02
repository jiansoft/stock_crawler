use anyhow::{anyhow, Result};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use scraper::{Html, Selector};

use crate::{util::text};

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
/// let text = parse_value(&element, "div.example");
/// assert_eq!(text, Some("Hello, world!".to_string()));
/// ```
pub fn parse_value(element: &scraper::ElementRef, css_selector: &str) -> Option<String> {
    match Selector::parse(css_selector) {
        Ok(s) => element
            .select(&s)
            .next()
            .map(|v| v.text().collect::<String>()),
        Err(_) => None,
    }
}

/// Extracts the value of the specified CSS selector from an HTML element and converts it to a `Decimal`.
///
/// This function is particularly useful for extracting numerical values from web pages.
///
/// # Arguments
///
/// * `element`: A reference to an `scraper::ElementRef` containing the HTML element to extract the value from.
/// * `css_selector`: A string representing the CSS selector to use for extracting the value from the HTML element.
///
/// # Returns
///
/// * `Decimal`: The extracted value as a `Decimal`. If the value cannot be parsed as a `Decimal`, or if the CSS selector is not found, it returns 0.
///
/// # Example
///
/// ```
/// use scraper::{Html, Selector};
/// use rust_decimal::Decimal;
///
/// let html = r#"
/// <div class="price">100.50元</div>
/// "#;
///
/// let fragment = Html::parse_fragment(html);
/// let price_selector = Selector::parse(".price").unwrap();
/// let element = fragment.select(&price_selector).next().unwrap();
///
/// let price = parse_to_decimal(&element, ".price");
/// assert_eq!(price, Decimal::from_str("100.50").unwrap());
/// ```
pub fn parse_to_decimal(element: &scraper::ElementRef, css_selector: &str) -> Decimal {
    parse_value(element, css_selector)
        .and_then(|v| text::parse_decimal(v.trim(), None).ok())
        .unwrap_or(dec!(0))
}

pub fn parse_to_i32(element: &scraper::ElementRef, css_selector: &str) -> i32 {
    parse_value(element, css_selector)
        .and_then(|v| text::parse_i32(v.trim(), None).ok())
        .unwrap_or(0)
}

pub fn parse_to_string(element: &scraper::ElementRef, css_selector: &str) -> String {
    parse_value(element, css_selector)
        .and_then(|v| Option::from(v.trim().to_string()))
        .unwrap_or(String::from(""))
}

/// A structure that represents the information needed to extract text from a particular HTML element.
#[derive(Debug, Clone)]
pub struct GetOneElementText<'a> {
    /// Stock symbol, often used for identifying stocks uniquely.
    pub stock_symbol: &'a str,
    /// The URL from where the HTML content will be fetched.
    pub url: &'a str,
    /// The CSS selector string that is used to identify and locate the desired HTML element.
    pub selector: &'a str,
    /// The desired HTML element's name (like "div", "span", etc.) that we want to extract text from.
    pub element: &'a str,
    pub document: Html,
}

/// Asynchronously fetches and extracts text from a specific HTML element in the provided URL.
///
/// This function first makes an HTTP request to fetch the content of the specified URL.
/// It then parses the fetched content as HTML. Using the provided CSS selector, it locates
/// the desired HTML element and then extracts its text. If the element is not found,
/// an error is returned.
///
/// # Arguments
///
/// * `target` - A structure containing details about the desired HTML element and the URL to fetch from.
///
/// # Returns
///
/// * `Result<String>` - On success, returns the extracted text from the specified HTML element.
///   On failure, returns an error indicating the reason for failure (like element not found, selector parse failure, etc.).
pub  fn get_one_element(target: GetOneElementText<'_>) -> Result<String> {
    //let text = util::http::get(target.url, None).await?;
   // let document = Html::parse_document(&text);
    let selector = Selector::parse(target.selector)
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;
    target.document
        .select(&selector)
        .next()
        .and_then(|element| parse_value(&element, target.element))
        .ok_or_else(|| anyhow!("The element not found from {}", target.url))
}

pub  fn get_one_element_as_decimal(target: GetOneElementText<'_>) -> Result<Decimal> {
    text::parse_decimal(&get_one_element(target)?, None)
}
