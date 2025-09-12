use axum_extra::TypedHeader;
use axum_extra::headers::{Header, HeaderMapExt as _};
use http::{HeaderMap, HeaderName};

trait TypedHeaderExt<T: Header> {
    fn inner(self) -> T;
}

impl<T: Header> TypedHeaderExt<T> for TypedHeader<T> {
    fn inner(self) -> T {
        self.0
    }
}

#[expect(
    private_bounds,
    reason = "Trait only implemented for TypedHeader, it should be sealed"
)]
#[must_use]
pub fn header_name<T: TypedHeaderExt<E>, E: Header>() -> &'static HeaderName {
    E::name()
}

#[expect(
    private_bounds,
    reason = "Trait only implemented for TypedHeader, it should be sealed"
)]
pub fn header_insert<T: TypedHeaderExt<E>, E: Header>(value: T, map: &mut HeaderMap) {
    map.typed_insert(value.inner());
}
