/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
#![expect(
    clippy::arbitrary_source_item_ordering,
    reason = "This order is more readable"
)]

pub mod sdk;
pub mod types;

#[doc(hidden)]
pub mod internal;
pub mod responses;
pub mod router;
pub(crate) mod utils;

pub use async_trait::async_trait;
pub use axum;
pub use axum_extra;
pub use daemon_framework_macros::define_error;
pub use daemon_framework_macros::error;
pub use daemon_framework_macros::service;
pub use http;
pub use pastey;
pub use reqwest;
pub use serde;
pub use serde_json;
pub use serde_urlencoded;
pub use sqlx;
pub use tap;
pub use url;
pub use urlencoding;
pub use utoipa;
pub use utoipa_axum;

use utoipa_axum::router::OpenApiRouter;

pub const SUCCESS_DESCRIPTION: &str = "Success";

pub trait MultiRouterExtension {
    #[must_use]
    fn merge(self, other: Self) -> Self;
}

impl MultiRouterExtension for (OpenApiRouter, OpenApiRouter) {
    fn merge(self, other: Self) -> Self {
        (self.0.merge(other.0), self.1.merge(other.1))
    }
}

#[macro_export]
macro_rules! define_path_variable {
    ($name:ident $description:literal) => {
        $crate::pastey::paste! {
            pub mod [< variable_path_ $name:snake >] {

                pub const DESCRIPTION: &str = $description;

            }
        }
    };
}

#[macro_export]
macro_rules! define_query_variable {
    ($name:ident $description:literal) => {
        $crate::pastey::paste! {
            pub mod [< variable_query_ $name:snake >] {

                pub const DESCRIPTION: &str = $description;

            }
        }
    };
}

#[macro_export]
macro_rules! define_header_variable {
    ($name:ident $description:literal) => {
        $crate::pastey::paste! {
            pub mod [< variable_header_ $name:snake >] {

                pub const DESCRIPTION: &str = $description;

            }
        }
    };
}

#[macro_export]
macro_rules! framework_router {
    ($base:tt $var:ident { $($tr:path),* $( { $($feat:literal $tr2:path),* } )* }) => {
        {
            use $crate::tap::Pipe;
            ($crate::router::FrameworkRouter::new()
                $(
                    .merge(<$base as $tr>::routes(&$var))
                )*
                $(
                    $(
                        .pipe(|router| {
                            #[cfg(feature = $feat)]
                            {
                                router.merge(<$base as $tr2>::routes(&$var))
                            }
                            #[cfg(not(feature = $feat))]
                            {
                                router
                            }
                        })
                    )*
                )*,
                $crate::router::FrameworkRouter::new()
                $(
                    .merge(<$base as $tr>::routes_user(&$var))
                )*
                $(
                    $(
                        .pipe(|router| {
                            #[cfg(feature = $feat)]
                            {
                                router.merge(<$base as $tr2>::routes_user(&$var))
                            }
                            #[cfg(not(feature = $feat))]
                            {
                                router
                            }
                        })

                    )*
                )*
            )
        }
    };
}
