use axum::extract::Request;
use axum::response::IntoResponse;
use axum::routing::{MethodRouter, Route};
use core::convert::Infallible;
use tower_layer::Layer;
use tower_service::Service;
use utoipa::openapi::path::OperationBuilder;
use utoipa::openapi::{HttpMethod, OpenApi, Paths, RefOr, ResponsesBuilder, Schema};
use utoipa_axum::router::OpenApiRouter;

#[must_use]
#[derive(Default)]
pub struct FrameworkRouter(Option<OpenApi>, Vec<RouterOperation>);

enum RouterOperation {
    Nest(&'static str, FrameworkRouter),
    Route(Box<FrameworkRoute>),
    Merge(FrameworkRouter),
    Patch(Box<dyn FnOnce(OpenApiRouter) -> OpenApiRouter>),
}

struct FrameworkRoute {
    path: &'static str,
    method: HttpMethod,
    responses: ResponsesBuilder,
    operations: OperationBuilder,
    schemas: Vec<(String, RefOr<Schema>)>,
    router: MethodRouter,
}

impl FrameworkRouter {
    pub const fn new() -> Self {
        Self(None, Vec::new())
    }

    pub const fn from_openapi(openapi: OpenApi) -> Self {
        Self(Some(openapi), Vec::new())
    }

    pub fn document_middleware<
        F: for<'a> FnMut(
            &'a HttpMethod,
            ResponsesBuilder,
            OperationBuilder,
            Vec<(String, RefOr<Schema>)>,
        ) -> (
            ResponsesBuilder,
            OperationBuilder,
            Vec<(String, RefOr<Schema>)>,
        ),
    >(
        self,
        mut func: F,
    ) -> Self {
        self.document_middleware0(&mut func)
    }

    fn document_middleware0<
        F: for<'a> FnMut(
            &'a HttpMethod,
            ResponsesBuilder,
            OperationBuilder,
            Vec<(String, RefOr<Schema>)>,
        ) -> (
            ResponsesBuilder,
            OperationBuilder,
            Vec<(String, RefOr<Schema>)>,
        ),
    >(
        mut self,
        func: &mut F,
    ) -> Self {
        self.1 = self
            .1
            .into_iter()
            .map(|op| match op {
                RouterOperation::Nest(path, router) => {
                    RouterOperation::Nest(path, router.document_middleware0(func))
                }
                RouterOperation::Route(mut route) => {
                    let (responses, operations, schemas) = func(
                        &route.method,
                        route.responses,
                        route.operations,
                        route.schemas,
                    );
                    route.responses = responses;
                    route.operations = operations;
                    route.schemas = schemas;
                    RouterOperation::Route(route)
                }
                RouterOperation::Merge(router) => {
                    RouterOperation::Merge(router.document_middleware0(func))
                }
                RouterOperation::Patch(func) => RouterOperation::Patch(func),
            })
            .collect();
        self
    }

    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        self.1.push(RouterOperation::Patch(Box::new(move |router| {
            router.layer(layer)
        })));
        self
    }

    pub fn route(
        mut self,
        path: &'static str,
        method: HttpMethod,
        (responses, operations, schemas): (
            ResponsesBuilder,
            OperationBuilder,
            Vec<(String, RefOr<Schema>)>,
        ),
        router: MethodRouter,
    ) -> Self {
        self.1.push(RouterOperation::Route(Box::new(FrameworkRoute {
            path,
            method,
            responses,
            operations,
            schemas,
            router,
        })));
        self
    }

    pub fn nest(mut self, path: &'static str, router: Self) -> Self {
        self.1.push(RouterOperation::Nest(path, router));
        self
    }

    pub fn merge(mut self, other: Self) -> Self {
        self.1.push(RouterOperation::Merge(other));
        self
    }

    #[must_use]
    pub fn into_openapi(self) -> OpenApiRouter {
        let mut openapi = self
            .0
            .map_or_else(OpenApiRouter::new, OpenApiRouter::with_openapi);

        for op in self.1 {
            match op {
                RouterOperation::Nest(path, router) => {
                    openapi = openapi.nest(path, router.into_openapi());
                }
                RouterOperation::Route(route) => {
                    let FrameworkRoute {
                        path,
                        method,
                        responses,
                        operations,
                        schemas,
                        router,
                    } = *route;
                    openapi = openapi.routes((
                        schemas,
                        {
                            let mut paths = Paths::new();
                            paths.add_path_operation(
                                path,
                                vec![method],
                                operations.responses(responses).build(),
                            );
                            paths
                        },
                        router,
                    ));
                }
                RouterOperation::Merge(other) => {
                    openapi = openapi.merge(other.into_openapi());
                }
                RouterOperation::Patch(func) => {
                    openapi = func(openapi);
                }
            }
        }

        openapi
    }
}
