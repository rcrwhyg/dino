use std::collections::HashMap;

use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{Host, Query, State},
    http::{request::Parts, Response},
    response::IntoResponse,
    routing::any,
    Router,
};
use dashmap::DashMap;
use indexmap::IndexMap;
use matchit::Match;
use tokio::net::TcpListener;
use tracing::info;

pub use config::*;
pub use engine::*;
pub use error::*;
pub use middleware::*;
pub use router::*;

mod config;
mod engine;
mod error;
mod middleware;
mod router;

type ProjectRoutes = IndexMap<String, Vec<ProjectRoute>>;

#[derive(Clone)]
pub struct AppState {
    // key is hostname
    routers: DashMap<String, SwappableAppRouter>,
}

#[derive(Clone)]
pub struct TenantRouter {
    host: String,
    router: SwappableAppRouter,
}

pub async fn start_server(port: u16, routes: Vec<TenantRouter>) -> Result<()> {
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(addr).await?;

    info!("listening on {}", listener.local_addr()?);

    let map = DashMap::new();
    for TenantRouter { host, router } in routes {
        map.insert(host, router);
    }

    let state = AppState::new(map);
    let app = Router::new()
        .route("/*path", any(handler))
        .layer(ServerTimeLayer)
        .with_state(state);

    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

// only support JSON requests and return JSON responses
#[allow(unused)]
async fn handler(
    State(state): State<AppState>,
    parts: Parts,
    Host(mut host): Host,
    Query(query): Query<HashMap<String, String>>,
    body: Option<Bytes>,
) -> Result<impl IntoResponse, AppError> {
    // get router from state
    let router = get_router_by_host(host, state)?;
    // match router with parts.path get a handler
    let matched = router.match_it(parts.method.clone(), parts.uri.path())?;
    // convert request data into Req and call handler with a js runtime
    let req = assemble_req(&matched, &parts, query, body)?;
    let handler = matched.value;
    // TODO: build a JsWorker pool, and send req via mpsc channel and get res from oneshot channel
    // but if code changed we need to recreate the pool
    let worker = JsWorker::try_new(&router.code)?;
    let res = worker.run(handler, req)?;
    // convert Req data into Res and return it
    Ok(Response::from(res))
}

impl AppState {
    pub fn new(routers: DashMap<String, SwappableAppRouter>) -> Self {
        Self { routers }
    }
}

impl TenantRouter {
    pub fn new(host: impl Into<String>, router: SwappableAppRouter) -> Self {
        Self {
            host: host.into(),
            router,
        }
    }
}

fn get_router_by_host(mut host: String, state: AppState) -> Result<AppRouter, AppError> {
    let _ = host.split_off(host.find(":").unwrap_or(host.len()));
    info!("host: {:?}", host);

    let router: AppRouter = state
        .routers
        .get(&host)
        .ok_or(AppError::HostNotFound(host))?
        .load();

    Ok(router)
}

fn assemble_req(
    matched: &Match<&str>,
    parts: &Parts,
    query: HashMap<String, String>,
    body: Option<Bytes>,
) -> Result<Req, AppError> {
    let params: HashMap<String, String> = matched
        .params
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    // convert request data into Req
    let headers = parts
        .headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
        .collect();

    let body = body.and_then(|b| String::from_utf8(b.into()).ok());

    let req = Req::builder()
        .method(parts.method.to_string())
        .url(parts.uri.to_string())
        .query(query)
        .params(params)
        .headers(headers)
        .body(body)
        .build();

    Ok(req)
}
