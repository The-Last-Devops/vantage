//! Serves the embedded Vue SPA (frontend/dist). Any request not matched by an
//! API/route falls back here; unknown paths return index.html so the client-side
//! router can handle them (history mode).

use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../frontend/dist"]
struct Assets;

pub async fn handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    serve(path)
        .or_else(|| serve("index.html"))
        .unwrap_or_else(|| (StatusCode::NOT_FOUND, "frontend not built").into_response())
}

fn serve(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    Some(
        Response::builder()
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(Body::from(file.data.into_owned()))
            .unwrap(),
    )
}
