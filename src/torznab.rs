use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::Client;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TorznabError {
    #[error("request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("xml parse error: {0}")]
    XmlError(#[from] quick_xml::Error),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Clone)]
pub struct TorrentResult {
    pub title: String,
    pub link: Option<String>,
    pub magnet_url: Option<String>,
    pub infohash: Option<String>,
    pub size: Option<u64>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub indexer: String,
}

impl TorrentResult {
    pub fn size_human(&self) -> String {
        match self.size {
            Some(bytes) => {
                const GB: u64 = 1024 * 1024 * 1024;
                const MB: u64 = 1024 * 1024;
                if bytes >= GB {
                    format!("{:.2} GB", bytes as f64 / GB as f64)
                } else {
                    format!("{:.1} MB", bytes as f64 / MB as f64)
                }
            }
            None => "?".to_string(),
        }
    }

    /// Get a URL that librqbit can use
    /// magnet, infohash-based magnet, or .torrent URL
    pub fn get_torrent_url(&self) -> Option<String> {
        // prefer magnet url if available
        if let Some(ref magnet) = self.magnet_url {
            return Some(magnet.clone());
        }

        // check if link is a magnet url
        if let Some(ref link) = self.link {
            if link.starts_with("magnet:") {
                return Some(link.clone());
            }
        }

        // construct magnet from infohash if available
        if let Some(ref hash) = self.infohash {
            let encoded_name = urlencoding::encode(&self.title);
            return Some(format!("magnet:?xt=urn:btih:{}&dn={}", hash, encoded_name));
        }

        // fall back to .torrent download link (prowlarr proxy URL)
        if let Some(ref link) = self.link {
            return Some(link.clone());
        }

        None
    }

    /// Check if this result can be streamed
    pub fn is_streamable(&self) -> bool {
        self.magnet_url.is_some() || self.infohash.is_some() || self.link.is_some()
    }
}

pub struct TorznabClient {
    client: Client,
}

impl TorznabClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Search with optional category filter
    /// Categories: 2000 = Movies, 5000 = TV
    pub async fn search(
        &self,
        base_url: &str,
        api_key: &str,
        indexer_id: i32,
        indexer_name: &str,
        query: &str,
        categories: Option<&[u32]>,
    ) -> Result<Vec<TorrentResult>, TorznabError> {
        let cat_param = categories
            .map(|cats| {
                format!(
                    "&cat={}",
                    cats.iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .unwrap_or_default();

        let url = format!(
            "{}/{}/api?t=search&apikey={}&q={}&limit=100{}",
            base_url.trim_end_matches('/'),
            indexer_id,
            api_key,
            urlencoding::encode(query),
            cat_param
        );

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(TorznabError::InvalidResponse(format!(
                "status: {}",
                response.status()
            )));
        }

        let xml = response.text().await?;
        self.parse_response(&xml, indexer_name)
    }

    fn parse_response(
        &self,
        xml: &str,
        indexer_name: &str,
    ) -> Result<Vec<TorrentResult>, TorznabError> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut results = Vec::new();
        let mut current_item: Option<TorrentResult> = None;
        let mut current_element = String::new();

        loop {
            match reader.read_event() {
                Ok(Event::Start(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    current_element = name.clone();

                    if name == "item" {
                        current_item = Some(TorrentResult {
                            title: String::new(),
                            link: None,
                            magnet_url: None,
                            infohash: None,
                            size: None,
                            seeders: None,
                            leechers: None,
                            indexer: indexer_name.to_string(),
                        });
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();

                    // handle <torznab:attr name="X" value="Y" /> elements
                    if name == "torznab:attr" || name == "attr" {
                        if let Some(ref mut item) = current_item {
                            let mut attr_name = String::new();
                            let mut attr_value = String::new();

                            for attr in e.attributes().flatten() {
                                let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                                let val = String::from_utf8_lossy(&attr.value).to_string();

                                if key == "name" {
                                    attr_name = val;
                                } else if key == "value" {
                                    attr_value = val;
                                }
                            }

                            match attr_name.as_str() {
                                "seeders" => item.seeders = attr_value.parse().ok(),
                                "leechers" => item.leechers = attr_value.parse().ok(),
                                "size" => item.size = attr_value.parse().ok(),
                                "magneturl" => item.magnet_url = Some(attr_value),
                                "infohash" => item.infohash = Some(attr_value),
                                _ => {}
                            }
                        }
                    }
                }
                Ok(Event::Text(ref e)) => {
                    if let Some(ref mut item) = current_item {
                        let text = e.unescape().unwrap_or_default().to_string();

                        match current_element.as_str() {
                            "title" => item.title = text,
                            "link" => item.link = Some(text),
                            "size" => {
                                if item.size.is_none() {
                                    item.size = text.parse().ok();
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();

                    if name == "item" {
                        if let Some(item) = current_item.take() {
                            if !item.title.is_empty() {
                                results.push(item);
                            }
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(TorznabError::XmlError(e)),
                _ => {}
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_human() {
        let result = TorrentResult {
            title: "Test".to_string(),
            link: None,
            magnet_url: None,
            infohash: None,
            size: Some(1024 * 1024 * 1024), // 1 GB
            seeders: None,
            leechers: None,
            indexer: "test".to_string(),
        };
        assert_eq!(result.size_human(), "1.00 GB");

        let result = TorrentResult {
            size: Some(500 * 1024 * 1024), // 500 MB
            ..result.clone()
        };
        assert_eq!(result.size_human(), "500.0 MB");

        let result = TorrentResult {
            size: Some(2 * 1024 * 1024 * 1024 + 512 * 1024 * 1024), // 2.5 GB
            ..result.clone()
        };
        assert_eq!(result.size_human(), "2.50 GB");

        let result = TorrentResult {
            size: None,
            ..result.clone()
        };
        assert_eq!(result.size_human(), "?");
    }

    #[test]
    fn test_get_torrent_url_magnet_priority() {
        // Magnet URL should be preferred
        let result = TorrentResult {
            title: "Test".to_string(),
            link: Some("http://example.com/torrent".to_string()),
            magnet_url: Some("magnet:?xt=urn:btih:abc123".to_string()),
            infohash: Some("def456".to_string()),
            size: None,
            seeders: None,
            leechers: None,
            indexer: "test".to_string(),
        };
        assert_eq!(result.get_torrent_url(), Some("magnet:?xt=urn:btih:abc123".to_string()));
    }

    #[test]
    fn test_get_torrent_url_link_is_magnet() {
        // Link that is a magnet URL
        let result = TorrentResult {
            title: "Test".to_string(),
            link: Some("magnet:?xt=urn:btih:fromlink".to_string()),
            magnet_url: None,
            infohash: None,
            size: None,
            seeders: None,
            leechers: None,
            indexer: "test".to_string(),
        };
        assert_eq!(result.get_torrent_url(), Some("magnet:?xt=urn:btih:fromlink".to_string()));
    }

    #[test]
    fn test_get_torrent_url_infohash_fallback() {
        // Construct magnet from infohash
        let result = TorrentResult {
            title: "Test Movie".to_string(),
            link: None,
            magnet_url: None,
            infohash: Some("abc123hash".to_string()),
            size: None,
            seeders: None,
            leechers: None,
            indexer: "test".to_string(),
        };
        let url = result.get_torrent_url().unwrap();
        assert!(url.starts_with("magnet:?xt=urn:btih:abc123hash"));
        assert!(url.contains("dn=Test%20Movie"));
    }

    #[test]
    fn test_get_torrent_url_link_fallback() {
        // Fall back to regular link
        let result = TorrentResult {
            title: "Test".to_string(),
            link: Some("http://example.com/download.torrent".to_string()),
            magnet_url: None,
            infohash: None,
            size: None,
            seeders: None,
            leechers: None,
            indexer: "test".to_string(),
        };
        assert_eq!(result.get_torrent_url(), Some("http://example.com/download.torrent".to_string()));
    }

    #[test]
    fn test_get_torrent_url_none() {
        let result = TorrentResult {
            title: "Test".to_string(),
            link: None,
            magnet_url: None,
            infohash: None,
            size: None,
            seeders: None,
            leechers: None,
            indexer: "test".to_string(),
        };
        assert_eq!(result.get_torrent_url(), None);
    }

    #[test]
    fn test_is_streamable() {
        let streamable = TorrentResult {
            title: "Test".to_string(),
            link: None,
            magnet_url: Some("magnet:?xt=...".to_string()),
            infohash: None,
            size: None,
            seeders: None,
            leechers: None,
            indexer: "test".to_string(),
        };
        assert!(streamable.is_streamable());

        let not_streamable = TorrentResult {
            title: "Test".to_string(),
            link: None,
            magnet_url: None,
            infohash: None,
            size: None,
            seeders: None,
            leechers: None,
            indexer: "test".to_string(),
        };
        assert!(!not_streamable.is_streamable());
    }

    #[test]
    fn test_parse_response_basic() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <item>
      <title>Test Movie 2024 1080p</title>
      <link>http://example.com/download</link>
      <size>1073741824</size>
      <torznab:attr name="seeders" value="50"/>
      <torznab:attr name="leechers" value="10"/>
      <torznab:attr name="magneturl" value="magnet:?xt=urn:btih:abc123"/>
    </item>
  </channel>
</rss>"#;

        let client = TorznabClient::new();
        let results = client.parse_response(xml, "TestIndexer").unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test Movie 2024 1080p");
        assert_eq!(results[0].link, Some("http://example.com/download".to_string()));
        assert_eq!(results[0].size, Some(1073741824));
        assert_eq!(results[0].seeders, Some(50));
        assert_eq!(results[0].leechers, Some(10));
        assert_eq!(results[0].magnet_url, Some("magnet:?xt=urn:btih:abc123".to_string()));
        assert_eq!(results[0].indexer, "TestIndexer");
    }

    #[test]
    fn test_parse_response_multiple_items() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <item>
      <title>Movie One</title>
      <torznab:attr name="seeders" value="100"/>
    </item>
    <item>
      <title>Movie Two</title>
      <torznab:attr name="seeders" value="50"/>
    </item>
    <item>
      <title>Movie Three</title>
      <torznab:attr name="seeders" value="25"/>
    </item>
  </channel>
</rss>"#;

        let client = TorznabClient::new();
        let results = client.parse_response(xml, "Test").unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].title, "Movie One");
        assert_eq!(results[0].seeders, Some(100));
        assert_eq!(results[1].title, "Movie Two");
        assert_eq!(results[2].title, "Movie Three");
    }

    #[test]
    fn test_parse_response_with_infohash() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <item>
      <title>Test</title>
      <torznab:attr name="infohash" value="abcdef1234567890"/>
    </item>
  </channel>
</rss>"#;

        let client = TorznabClient::new();
        let results = client.parse_response(xml, "Test").unwrap();

        assert_eq!(results[0].infohash, Some("abcdef1234567890".to_string()));
    }

    #[test]
    fn test_parse_response_empty() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
  </channel>
</rss>"#;

        let client = TorznabClient::new();
        let results = client.parse_response(xml, "Test").unwrap();

        assert!(results.is_empty());
    }
}
