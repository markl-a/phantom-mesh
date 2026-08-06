//! fetch_yahoo.rs - 抓取 tw.yahoo.com 主要標題新聞
use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct YahooNewsResponse {
    data: Option<Vec<YahooNewsItem>>,
}

#[derive(Debug, Deserialize)]
struct YahooNewsItem {
    title: Option<String>,
    url: Option<String>,
    publisher: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
        .build()?;

    // 嘗試抓取 Yahoo 新聞首頁
    let url = "https://tw.yahoo.com/";
    
    println!("正在抓取: {}", url);
    
    let response = client.get(url).send().await?;
    let html = response.text().await?;
    
    // 簡單解析：找出標題區塊 (在 <a> 標籤中的文字)
    let mut titles: Vec<String> = Vec::new();
    
    // 用正則抓取標題
    let re = regex::Regex::new(r#"<a[^>]+class="[^"]*trending[^"]*"[^>]*>([^<]+)"#).unwrap();
    for cap in re.captures_iter(&html) {
        if let Some(title) = cap.get(1) {
            let t = title.as_str().trim().to_string();
            if !t.is_empty() && t.len() > 5 {
                titles.push(t);
            }
        }
    }
    
    // 或嘗試抓取新聞列表 API
    let api_url = "https://news.yahoo.com/v3/mcp/ten_article/rc_adas?子串=1";
    
    if let Ok(resp) = client.get(api_url).send().await {
        if let Ok(json) = resp.json::<YahooNewsResponse>() {
            if let Some(items) = json.data {
                for item in items.iter().take(10) {
                    if let Some(title) = &item.title {
                        println!("📰 {}", title);
                    }
                }
                return Ok(());
            }
        }
    }
    
    // 如果 API 失敗，展示 HTML 抓到的標題
    println!("\n=== Yahoo 熱門標題 ===\n");
    for (i, title) in titles.iter().take(10).enumerate() {
        println!("{}. {}", i + 1, title);
    }
    
    Ok(())
}