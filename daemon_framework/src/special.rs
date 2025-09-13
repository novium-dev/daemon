use axum::extract::Request;
use http::request::Parts;
use reqwest::RequestBuilder;

pub trait IntoRequestParts {
    fn into_request(self, req: RequestBuilder) -> impl Future<Output = RequestBuilder> + Send;
}

pub trait FromRequestParts<E> {
    fn from_request_parts(parts: &mut Parts) -> impl Future<Output = Result<Self, E>> + Send
    where
        Self: Sized;
}

pub trait FromRequest<E> {
    fn from_request(req: Request) -> impl Future<Output = Result<Self, E>> + Send
    where
        Self: Sized;
}
