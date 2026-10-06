//! Tool [`web_search`](super::web_search): tìm kiếm qua Tavily/Brave/SearXNG,
//! lọc URL riêng tư và render kết quả bọc untrusted.

use super::fetch::wrap_untrusted_limited;
use super::*;

/// Tham số tìm kiếm web.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// Tìm kiếm web qua provider đã cấu hình và trả về các URL nguồn cùng đoạn trích.
///
/// Dùng để khám phá nguồn; sau đó gọi `web_fetch` trên URL phù hợp để đọc nội dung đầy đủ.
/// Kết quả tìm kiếm là dữ liệu không tin cậy, không phải chỉ dẫn cho agent.
pub struct WebSearchParams {
    /// Câu truy vấn tìm kiếm bằng ngôn ngữ tự nhiên.
    pub query: String,
    /// Số kết quả tối đa; mặc định 5, tối đa 10.
    #[serde(default = "default_search_results")]
    pub max_results: usize,
}

fn default_search_results() -> usize {
    5
}

/// Lỗi dựng `web_search` từ cấu hình.
#[derive(Debug, thiserror::Error)]
pub enum SearchConfigError {
    /// SearXNG cần `tools.web_search.base_url`.
    #[error("provider searxng cần tools.web_search.base_url")]
    MissingSearxngBaseUrl,
    /// Endpoint cấu hình không phải URL hợp lệ.
    #[error("URL endpoint web_search không hợp lệ: {0}")]
    InvalidEndpoint(#[from] url::ParseError),
    /// Endpoint cấu hình phải là HTTP(S).
    #[error("URL endpoint web_search phải là http:// hoặc https://")]
    InvalidEndpointScheme,
    /// Không chấp nhận credential nhúng trong URL provider.
    #[error("URL endpoint web_search không được chứa username hoặc password")]
    InvalidEndpointCredentials,
    /// Không dựng được HTTP client.
    #[error("không dựng được HTTP client web_search: {0}")]
    Client(#[source] reqwest::Error),
}

#[derive(Debug, Deserialize)]
struct TavilyResponse {
    #[serde(default)]
    results: Vec<TavilyResult>,
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    #[serde(default)]
    title: String,
    url: String,
    #[serde(default)]
    content: String,
}

#[derive(Debug, Deserialize)]
struct BraveResponse {
    web: BraveWeb,
}

#[derive(Debug, Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<BraveResult>,
}

#[derive(Debug, Deserialize)]
struct BraveResult {
    #[serde(default)]
    title: String,
    url: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngResult>,
}

#[derive(Debug, Deserialize)]
struct SearxngResult {
    #[serde(default)]
    title: String,
    url: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    engine: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SearchHit {
    pub(super) title: String,
    pub(super) url: String,
    pub(super) snippet: String,
}

/// Client search không dùng SSRF filter vì endpoint là cấu hình tin cậy; URL trả về vẫn
/// phải qua `web_fetch` trước khi tải.
pub(super) struct SearchClient {
    pub(super) provider: WebSearchProvider,
    pub(super) endpoint: Url,
    pub(super) api_key: Option<SecretString>,
    pub(super) api_key_env: String,
    pub(super) client: reqwest::Client,
}

impl SearchClient {
    fn from_config(
        config: &WebSearchConfig,
        api_key: Option<SecretString>,
    ) -> Result<Self, SearchConfigError> {
        let endpoint = match config.provider {
            WebSearchProvider::Tavily => Url::parse("https://api.tavily.com/search")?,
            WebSearchProvider::Brave => {
                Url::parse("https://api.search.brave.com/res/v1/web/search")?
            }
            WebSearchProvider::Searxng => {
                let base = config
                    .base_url
                    .as_deref()
                    .ok_or(SearchConfigError::MissingSearxngBaseUrl)?;
                searxng_endpoint(base)?
            }
        };
        let client = build_search_http_client()?;
        Ok(Self {
            provider: config.provider,
            endpoint,
            api_key,
            api_key_env: config.api_key_env.clone(),
            client,
        })
    }

    #[cfg(test)]
    pub(super) fn for_test(
        provider: WebSearchProvider,
        endpoint: Url,
        api_key: Option<SecretString>,
    ) -> Result<Self, SearchConfigError> {
        Ok(Self {
            provider,
            endpoint,
            api_key,
            api_key_env: "TEST_SEARCH_KEY".to_string(),
            client: build_search_http_client()?,
        })
    }
}

fn build_search_http_client() -> Result<reqwest::Client, SearchConfigError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .no_proxy()
        .timeout(crate::ssrf::REQUEST_TIMEOUT)
        .connect_timeout(crate::ssrf::CONNECT_TIMEOUT)
        .user_agent(concat!("bean/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(SearchConfigError::Client)
}

pub(super) fn searxng_endpoint(base: &str) -> Result<Url, SearchConfigError> {
    let mut endpoint = Url::parse(base)?;
    if !matches!(endpoint.scheme(), "http" | "https") || endpoint.host().is_none() {
        return Err(SearchConfigError::InvalidEndpointScheme);
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(SearchConfigError::InvalidEndpointCredentials);
    }
    let path = format!("{}/search", endpoint.path().trim_end_matches('/'));
    endpoint.set_path(&path);
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    Ok(endpoint)
}

impl SearchClient {
    async fn search(&self, params: WebSearchParams) -> Result<Vec<SearchHit>, ToolError> {
        let query = params.query.trim();
        if query.is_empty() {
            return Err(ToolError::InvalidArgs("query rỗng".to_string()));
        }
        if query.chars().count() > 2_000 {
            return Err(ToolError::InvalidArgs(
                "query dài hơn 2.000 ký tự".to_string(),
            ));
        }
        if params.max_results == 0 || params.max_results > 10 {
            return Err(ToolError::InvalidArgs(
                "max_results phải trong 1..=10".to_string(),
            ));
        }

        let mut response = match self.provider {
            WebSearchProvider::Tavily => {
                let key = self.api_key()?;
                let mut value = Vec::from(b"Bearer ");
                value.extend_from_slice(key.expose_secret().as_bytes());
                let auth = reqwest::header::HeaderValue::from_bytes(&value).map_err(|_| {
                    ToolError::Internal("API key web_search chứa ký tự header không hợp lệ".into())
                })?;
                self.client
                    .post(self.endpoint.clone())
                    .header(reqwest::header::AUTHORIZATION, auth)
                    .json(&serde_json::json!({
                        "query": query,
                        "search_depth": "basic",
                        "max_results": params.max_results,
                        "include_answer": false,
                        "include_raw_content": false
                    }))
                    .send()
                    .await
                    .map_err(search_request_error)?
            }
            WebSearchProvider::Brave => {
                let key = self.api_key()?;
                let auth =
                    reqwest::header::HeaderValue::from_str(key.expose_secret()).map_err(|_| {
                        ToolError::Internal(
                            "API key web_search chứa ký tự header không hợp lệ".into(),
                        )
                    })?;
                let mut url = self.endpoint.clone();
                url.query_pairs_mut()
                    .append_pair("q", query)
                    .append_pair("count", &params.max_results.to_string());
                self.client
                    .get(url)
                    .header("X-Subscription-Token", auth)
                    .send()
                    .await
                    .map_err(search_request_error)?
            }
            WebSearchProvider::Searxng => {
                let mut url = self.endpoint.clone();
                url.query_pairs_mut()
                    .append_pair("q", query)
                    .append_pair("format", "json");
                self.client
                    .get(url)
                    .send()
                    .await
                    .map_err(search_request_error)?
            }
        };

        let status = response.status();
        if !status.is_success() {
            return Err(ToolError::Io(format!(
                "provider web_search trả HTTP {}",
                status.as_u16()
            )));
        }
        let body = crate::ssrf::read_body_limited(&mut response, DEFAULT_MAX_BODY_BYTES)
            .await
            .map_err(|error| ToolError::Io(error.to_string()))?;
        let hits: Vec<SearchHit> = match self.provider {
            WebSearchProvider::Tavily => {
                let parsed: TavilyResponse = serde_json::from_slice(&body).map_err(|err| {
                    ToolError::InvalidData(format!("JSON Tavily không hợp lệ: {err}"))
                })?;
                parsed
                    .results
                    .into_iter()
                    .map(|item| SearchHit {
                        title: item.title,
                        url: item.url,
                        snippet: item.content,
                    })
                    .collect()
            }
            WebSearchProvider::Brave => {
                let parsed: BraveResponse = serde_json::from_slice(&body).map_err(|err| {
                    ToolError::InvalidData(format!("JSON Brave không hợp lệ: {err}"))
                })?;
                parsed
                    .web
                    .results
                    .into_iter()
                    .map(|item| SearchHit {
                        title: item.title,
                        url: item.url,
                        snippet: item.description,
                    })
                    .collect()
            }
            WebSearchProvider::Searxng => {
                let parsed: SearxngResponse = serde_json::from_slice(&body).map_err(|err| {
                    ToolError::InvalidData(format!("JSON SearXNG không hợp lệ: {err}"))
                })?;
                parsed
                    .results
                    .into_iter()
                    .map(|item| {
                        let snippet = if item.engine.trim().is_empty() {
                            item.content
                        } else {
                            format!("[{}] {}", item.engine, item.content)
                        };
                        SearchHit {
                            title: item.title,
                            url: item.url,
                            snippet,
                        }
                    })
                    .collect()
            }
        };
        Ok(hits
            .into_iter()
            .filter_map(valid_search_hit)
            .take(params.max_results)
            .collect())
    }

    fn api_key(&self) -> Result<&SecretString, ToolError> {
        self.api_key.as_ref().ok_or_else(|| {
            ToolError::Internal(format!(
                "web_search chưa có API key; đặt biến môi trường `{}`",
                self.api_key_env
            ))
        })
    }
}

fn search_request_error(error: reqwest::Error) -> ToolError {
    ToolError::Io(format!("lỗi provider web_search: {}", error.without_url()))
}

pub(super) fn valid_search_hit(hit: SearchHit) -> Option<SearchHit> {
    crate::ssrf::validate_url(&hit.url).ok()?;
    Some(hit)
}

/// Dựng tool `web_search` (Safe) theo provider trong config.
///
/// `api_key` phải đến từ `Config::resolve_secrets`; hàm không tự đọc biến môi trường và
/// không bao giờ log key. Với Tavily/Brave thiếu key sẽ chỉ báo lỗi rõ ràng khi tool được
/// gọi, đúng D6.4.
pub fn web_search(
    config: &WebSearchConfig,
    api_key: Option<SecretString>,
) -> Result<Arc<dyn Tool>, SearchConfigError> {
    let client = Arc::new(SearchClient::from_config(config, api_key)?);
    Ok(web_search_with_client(client))
}

pub(super) fn web_search_with_client(client: Arc<SearchClient>) -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new(
            "web_search",
            Risk::Safe,
            move |ctx: &ToolCtx, params: WebSearchParams| {
                let client = Arc::clone(&client);
                let untrusted_seen = Arc::clone(&ctx.untrusted_seen);
                async move {
                    let hits = client.search(params).await?;
                    let rendered = render_search_results(&client.provider, &hits);
                    untrusted_seen.store(true, Ordering::SeqCst);
                    Ok(wrap_untrusted_limited(&rendered))
                }
            },
        )
        // Handler đã tự bọc `wrap_untrusted_limited` — chỉ khai báo cờ (mục 15.4).
        .declares_untrusted()
        // (M24) Mở cho vai trò marketing; untagged nên vẫn hiện với mọi role như cũ.
        .also_visible_to([MARKETING_READ_TAG]),
    )
}

fn render_search_results(provider: &WebSearchProvider, hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return format!(
            "Không tìm thấy kết quả nào qua provider {}.",
            provider_name(provider)
        );
    }
    let mut rendered = format!("Kết quả tìm kiếm qua {}:\n", provider_name(provider));
    for (index, hit) in hits.iter().enumerate() {
        rendered.push_str(&format!("\n{}. {}\n", index + 1, one_line(&hit.title)));
        rendered.push_str(&format!("URL: {}\n", hit.url));
        let snippet = one_line(&hit.snippet);
        if !snippet.is_empty() {
            rendered.push_str("Trích: ");
            rendered.push_str(&snippet);
            rendered.push('\n');
        }
    }
    truncate_rendered(&rendered, MAX_SEARCH_OUTPUT_CHARS)
}

fn provider_name(provider: &WebSearchProvider) -> &'static str {
    match provider {
        WebSearchProvider::Tavily => "Tavily",
        WebSearchProvider::Brave => "Brave Search",
        WebSearchProvider::Searxng => "SearXNG",
    }
}

fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_rendered(value: &str, max_chars: usize) -> String {
    let total = value.chars().count();
    if total <= max_chars {
        return value.to_string();
    }
    let note = "\n[Đã cắt danh sách kết quả]";
    let note_len = note.chars().count();
    let keep = max_chars.saturating_sub(note_len);
    let mut output: String = value.chars().take(keep).collect();
    output.push_str(note);
    output
}
