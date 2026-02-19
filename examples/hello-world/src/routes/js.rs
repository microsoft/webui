use std::fs;
use tiny_http::{Header, Request, Response, StatusCode};

// JavaScript route for the main JS file
pub fn handle_js(request: Request) {
    let body = match fs::read_to_string("static/app.js") {
        Ok(contents) => contents,
        Err(err) => {
            eprintln!("Failed to read app.js: {err}");
            let response = Response::from_string("Not Found")
                .with_status_code(StatusCode(404));
            let _ = request.respond(response);
            return;
        }
    };

    let mut response = Response::from_string(body).with_status_code(StatusCode(200));

    if let Ok(header) = Header::from_bytes(
        b"Content-Type" as &[u8],
        b"application/javascript; charset=utf-8" as &[u8],
    ) {
        response.add_header(header);
    }

    let _ = request.respond(response);
}
