//! Keyless Spanish-language news feed adapter (Google News RSS).

use async_trait::async_trait;
use hs_tools::{NewsItem, NewsSearch};

/// News adapter backed by the public Google News RSS endpoint.
#[derive(Debug, Clone)]
pub struct NewsAdapter {
    http: reqwest::Client,
}

impl NewsAdapter {
    /// Build an adapter with a short timeout and no proxy.
    #[must_use]
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(12))
            .user_agent("HomeServerAssistant/0.1 (news)")
            .build()
            .unwrap_or_default();
        Self { http }
    }
}

impl Default for NewsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NewsSearch for NewsAdapter {
    async fn latest(&self, query: Option<&str>, limit: usize) -> Result<Vec<NewsItem>, String> {
        let limit = limit.clamp(1, 8);
        let request = match query.map(str::trim).filter(|value| !value.is_empty()) {
            Some(query) => self.http.get("https://news.google.com/rss/search").query(&[
                ("q", query),
                ("hl", "es-419"),
                ("gl", "PE"),
                ("ceid", "PE:es-419"),
            ]),
            None => self.http.get("https://news.google.com/rss").query(&[
                ("hl", "es-419"),
                ("gl", "PE"),
                ("ceid", "PE:es-419"),
            ]),
        };
        let xml = request
            .send()
            .await
            .map_err(|error| format!("no pude consultar las noticias: {error}"))?
            .error_for_status()
            .map_err(|error| format!("el feed de noticias respondió con error: {error}"))?
            .text()
            .await
            .map_err(|error| format!("no pude leer el feed de noticias: {error}"))?;
        let items = parse_rss(&xml, limit);
        if items.is_empty() {
            return Err("no encontré noticias para esa búsqueda".to_owned());
        }
        Ok(items)
    }
}

fn parse_rss(xml: &str, limit: usize) -> Vec<NewsItem> {
    let mut items = Vec::new();
    let mut rest = xml;
    while items.len() < limit {
        let Some(start) = rest.find("<item>") else {
            break;
        };
        let after_start = &rest[start + "<item>".len()..];
        let Some(end) = after_start.find("</item>") else {
            break;
        };
        let block = &after_start[..end];
        rest = &after_start[end + "</item>".len()..];
        let Some(raw_title) = tag(block, "title") else {
            continue;
        };
        let link = tag(block, "link").unwrap_or_default();
        let published_at = tag(block, "pubDate").unwrap_or_default();
        let source = tag(block, "source")
            .or_else(|| {
                raw_title
                    .rsplit_once(" - ")
                    .map(|(_, source)| source.to_owned())
            })
            .unwrap_or_default();
        let title = raw_title
            .rsplit_once(" - ")
            .map(|(title, _)| title.to_owned())
            .unwrap_or(raw_title);
        if title.is_empty() || link.is_empty() {
            continue;
        }
        items.push(NewsItem {
            title,
            source,
            link,
            published_at,
        });
    }
    items
}

fn tag(block: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&close)? + start;
    let value = block[start..end].trim();
    let value = value
        .strip_prefix("<![CDATA[")
        .and_then(|inner| inner.strip_suffix("]]>"))
        .unwrap_or(value);
    Some(unescape(value.trim()))
}

fn unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<rss><channel>
      <item><title>Noticia uno - El Comercio</title><link>https://example.test/1</link><pubDate>Tue, 25 Nov 2026 10:00:00 GMT</pubDate><source url="x">El Comercio</source></item>
      <item><title><![CDATA[Noticia dos & más]]></title><link>https://example.test/2</link><pubDate>Tue, 25 Nov 2026 09:00:00 GMT</pubDate></item>
    </channel></rss>"#;

    #[test]
    fn parses_titles_sources_and_links() {
        let items = parse_rss(SAMPLE, 5);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Noticia uno");
        assert_eq!(items[0].source, "El Comercio");
        assert_eq!(items[0].link, "https://example.test/1");
        assert_eq!(items[1].title, "Noticia dos & más");
    }
}
