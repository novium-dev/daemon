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
