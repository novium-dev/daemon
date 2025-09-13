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

pub trait OptionalFromRequestParts<E> {
    fn from_request_parts(
        parts: &mut Parts,
    ) -> impl Future<Output = Result<Option<Self>, E>> + Send
    where
        Self: Sized;
}

pub trait OptionalFromRequest<E> {
    fn from_request(req: Request) -> impl Future<Output = Result<Option<Self>, E>> + Send
    where
        Self: Sized;
}

impl<E, T: OptionalFromRequestParts<E>> FromRequestParts<E> for Option<T> {
    async fn from_request_parts(parts: &mut Parts) -> Result<Self, E>
    where
        Self: Sized,
    {
        T::from_request_parts(parts).await
    }
}

impl<E, T: OptionalFromRequest<E>> FromRequest<E> for Option<T> {
    async fn from_request(req: Request) -> Result<Self, E>
    where
        Self: Sized,
    {
        T::from_request(req).await
    }
}
