//! Yahoo Taiwan 新聞爬蟲 (簡化版)
//! 使用方式: cargo run --example yahoo_scraper [category]
//! category: news, finance, sports, entertainment, tech

use regex::Regex;
use reqwest::Client;
use tokio;

#[derive(Debug)]
struct NewsItem {
    title: String,
    link: String,
}

async fn fetch_yahoo(
    category: &str,
) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()?;

    let url = match category {
        "finance" => "https://tw.stock.yahoo.com/",
        "sports" => "https://tw.sports.yahoo.com/",
        "entertainment" => "https://tw.entertainment.yahoo.com/",
        "tech" => "https://tw.tech.yahoo.com/",
        _ => "https://news.yahoo.com/",
    };

    println!("fetching: {}", url);
    let resp = client.get(url).send().await?;
    let html = resp.text().await?;

    // 使用正則抓取標題和連結
    let title_re = Regex::new(r#"<a[^>]+href="([^"]+)"[^>]*><[^>]+>([^<]+)</"#)?;
    let mut items = Vec::new();

    for cap in title_re.captures_iter(&html) {
        let link = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let title = cap.get(2).map(|m| m.as_str()).unwrap_or("").trim();

        if title.len() > 10 && !link.is_empty() && !title.contains("img") {
            let full_link = if link.starts_with("http") {
                link.to_string()
            } else {
                format!("https://tw.news.yahoo.com{}", link)
            };
            items.push(NewsItem {
                title: title.to_string(),
                link: full_link,
            });
        }
    }

    Ok(items)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();
    let category = args.get(1).map(|s| s.as_str()).unwrap_or("news");

    println!("🏀 Yahoo {} 新聞:\n", category);

    match fetch_yahoo(category).await {
        Ok(items) => {
            for (i, item) in items.iter().take(15).enumerate() {
                println!("{}. {}\n   🔗 {}\n", i + 1, item.title, item.link);
            }
        }
        Err(e) => eprintln!("❌ {}", e),
    }

    Ok(())
}
