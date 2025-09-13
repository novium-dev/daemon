use url::Url;

pub trait FrameworkClientImpl {
    fn base_url(&self) -> &Url;
    fn client(&self) -> &reqwest::Client;
}
