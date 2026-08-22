use super::models::*;
use serde::Serialize;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};
use tiny_http::{Header, Response, Server, StatusCode};

pub const DEFAULT_API_PORT: u16 = 32947;

#[derive(Debug)]
pub(crate) enum ApiFailure {
    NotFound,
    MethodNotAllowed,
    ProfileNotFound,
    InvalidQuery,
    InvalidPath,
    DatabaseUnavailable,
    Internal,
}

impl From<crate::Error> for ApiFailure {
    fn from(_: crate::Error) -> Self {
        Self::Internal
    }
}

pub(crate) fn validate_database(path: &Path) -> Result<(), crate::Error> {
    crate::db::validate_read_database(path).map_err(|_| {
        crate::Error::Input(
            "cannot open configured database read-only with schema version 2".into(),
        )
    })
}

pub(crate) fn error_response(error: ApiFailure) -> Response<std::io::Cursor<Vec<u8>>> {
    let (status, code, message) = match error {
        ApiFailure::NotFound => (404, "not_found", "not found"),
        ApiFailure::MethodNotAllowed => (405, "method_not_allowed", "method not allowed"),
        ApiFailure::ProfileNotFound => (404, "profile_not_found", "profile not found"),
        ApiFailure::InvalidQuery => (400, "invalid_query", "invalid query parameters"),
        ApiFailure::InvalidPath => (400, "invalid_path", "invalid path"),
        ApiFailure::DatabaseUnavailable => (
            503,
            "database_unavailable",
            "database temporarily unavailable",
        ),
        ApiFailure::Internal => (500, "internal_error", "internal server error"),
    };
    json_response(
        status,
        &ErrorBody {
            error: ErrorDto { code, message },
        },
    )
}
pub(crate) fn json_response<T: Serialize>(
    status: u16,
    value: &T,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let bytes = serde_json::to_vec(value).unwrap_or_else(|_| {
        b"{\"error\":{\"code\":\"internal_error\",\"message\":\"internal server error\"}}".to_vec()
    });
    Response::from_data(bytes)
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
}
pub(crate) fn static_response(
    content_type: &str,
    body: &'static str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(StatusCode(200))
        .with_header(Header::from_bytes("Content-Type", content_type).unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
}

pub struct ApiServer {
    address: SocketAddr,
    shutdown: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl ApiServer {
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    #[allow(dead_code)]
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        self.stop();
    }
}
pub fn start_for_tests(database: &Path, port: u16) -> Result<ApiServer, crate::Error> {
    validate_database(database)?;
    let server =
        Server::http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).map_err(|e| {
            crate::Error::Io(format!("cannot bind local API: display={e}; debug={e:?}"))
        })?;
    let address = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| crate::Error::Io("local API did not bind an IP socket".into()))?;
    let database = PathBuf::from(database);
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let thread = thread::spawn(move || {
        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            if let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(50)) {
                let response = super::routes::route(&database, request.method(), request.url());
                let _ = request.respond(response);
            }
        }
    });
    Ok(ApiServer {
        address,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    })
}
pub fn serve(database: &Path, port: u16) -> Result<(), crate::Error> {
    let server = start_for_tests(database, port)?;
    println!(
        "TruckLedger dashboard and local API listening on http://{}",
        server.address()
    );
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(test)]
#[path = "server/tests.rs"]
mod tests;
