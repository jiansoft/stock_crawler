//! User-Agent 產生器的測試。

use super::*;

#[test]
fn test_gen_random_ua_variety() {
    // Generate 20 user agents to test variety
    println!("\n=== Testing User Agent Variety ===");
    for i in 1..=20 {
        let ua = gen_random_ua();
        println!("{:2}. {}", i, ua);
    }
}

#[test]
fn test_all_browser_generators() {
    println!("\n=== Testing All Browser Generators ===");

    println!("\nChrome:");
    println!("  {}", gen_chrome_ua());

    println!("\nFirefox:");
    println!("  {}", gen_firefox_ua());

    println!("\nEdge:");
    println!("  {}", gen_edge_ua());

    println!("\nOpera:");
    println!("  {}", gen_opera_ua());

    println!("\nBrave:");
    println!("  {}", gen_brave_ua());

    println!("\nVivaldi:");
    println!("  {}", gen_vivaldi_ua());

    println!("\nSafari Desktop:");
    println!("  {}", gen_safari_desktop_ua());

    println!("\nSafari Mobile:");
    println!("  {}", gen_safari_mobile_ua());

    println!("\nSamsung Internet:");
    println!("  {}", gen_samsung_internet_ua());
}

#[test]
fn test_ua_formats() {
    // Test that all UAs contain expected patterns
    for _ in 0..100 {
        let ua = gen_random_ua();
        assert!(
            ua.starts_with("Mozilla/5.0"),
            "UA should start with Mozilla/5.0: {}",
            ua
        );
        assert!(ua.len() > 50, "UA should be reasonably long: {}", ua);
    }
}
