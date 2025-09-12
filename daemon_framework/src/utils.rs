use core::any::TypeId;
use core::mem::MaybeUninit;

pub(crate) trait MaybeUnit {
    fn unit() -> Option<Self>
    where
        Self: Sized;
}

impl<T: 'static> MaybeUnit for T {
    fn unit() -> Option<Self>
    where
        Self: Sized,
    {
        // SAFETY: It is certain that the current type is (), and therefore doesn't have any size. There is no undefined behaviour.
        (TypeId::of::<()>() == TypeId::of::<T>())
            .then(|| unsafe { MaybeUninit::zeroed().assume_init() })
    }
}
