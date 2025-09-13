use crate::responses::DocumentedResponse;
use crate::special::{FromRequest, FromRequestParts};
use axum::extract::Request;
use axum::response::IntoResponse;
use core::marker::PhantomData;
use http::request::Parts;

pub struct AxumSpecialWrapper<T, E>(pub T, pub PhantomData<E>);

impl<V, E: IntoResponse + DocumentedResponse, T: FromRequestParts<E>, S: Send + Sync>
    axum::extract::FromRequestParts<S> for AxumSpecialWrapper<T, Result<V, E>>
{
    type Rejection = E;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        <T as FromRequestParts<E>>::from_request_parts(parts)
            .await
            .map(|var| Self(var, PhantomData))
    }
}

impl<V, E: IntoResponse + DocumentedResponse, T: FromRequest<E>, S: Send + Sync>
    axum::extract::FromRequest<S> for AxumSpecialWrapper<T, Result<V, E>>
{
    type Rejection = E;

    async fn from_request(req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        <T as FromRequest<E>>::from_request(req)
            .await
            .map(|var| Self(var, PhantomData))
    }
}
