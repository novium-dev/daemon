use reqwest::RequestBuilder;
use url::Url;

pub trait IntoRequestParts {
    fn into_request(self, req: RequestBuilder) -> impl Future<Output = RequestBuilder> + Send; // TODO:
}

pub trait FrameworkClientImpl {
    fn base_url(&self) -> &Url;
    fn client(&self) -> &reqwest::Client;
}
