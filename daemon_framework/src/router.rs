use axum::routing::MethodRouter;
use utoipa::openapi::path::OperationBuilder;
use utoipa::openapi::{HttpMethod, Paths, RefOr, ResponsesBuilder, Schema};
use utoipa_axum::router::OpenApiRouter;

#[must_use]
#[derive(Default)]
pub struct FrameworkRouter(Vec<RouterOperation>);

enum RouterOperation {
    Nest(&'static str, FrameworkRouter),
    Route(Box<FrameworkRoute>),
    Merge(FrameworkRouter),
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn document_middleware<
        F: FnMut(
            HttpMethod,
            ResponsesBuilder,
            OperationBuilder,
            Vec<(String, RefOr<Schema>)>,
        ) -> (
            HttpMethod,
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
        F: FnMut(
            HttpMethod,
            ResponsesBuilder,
            OperationBuilder,
            Vec<(String, RefOr<Schema>)>,
        ) -> (
            HttpMethod,
            ResponsesBuilder,
            OperationBuilder,
            Vec<(String, RefOr<Schema>)>,
        ),
    >(
        mut self,
        func: &mut F,
    ) -> Self {
        self.0 = self
            .0
            .into_iter()
            .map(|op| match op {
                RouterOperation::Nest(path, router) => {
                    RouterOperation::Nest(path, router.document_middleware0(func))
                }
                RouterOperation::Route(mut route) => {
                    let (method, responses, operations, schemas) = func(
                        route.method,
                        route.responses,
                        route.operations,
                        route.schemas,
                    );
                    route.method = method;
                    route.responses = responses;
                    route.operations = operations;
                    route.schemas = schemas;
                    RouterOperation::Route(route)
                }
                RouterOperation::Merge(router) => {
                    RouterOperation::Merge(router.document_middleware0(func))
                }
            })
            .collect();
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
        self.0.push(RouterOperation::Route(Box::new(FrameworkRoute {
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
        self.0.push(RouterOperation::Nest(path, router));
        self
    }

    pub fn merge(mut self, other: Self) -> Self {
        self.0.push(RouterOperation::Merge(other));
        self
    }

    #[must_use]
    pub fn into_openapi(self) -> OpenApiRouter {
        let mut openapi = OpenApiRouter::new();

        for op in self.0 {
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
            }
        }

        openapi
    }
}
