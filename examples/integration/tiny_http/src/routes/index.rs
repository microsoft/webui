use std::fs;
use tiny_http::{Header, Request, Response, StatusCode};

// Root route for the index.html page
pub fn handle_index(request: Request) {
    let body = match fs::read_to_string("dist/index.html") {
        Ok(contents) => contents,
        Err(err) => {
            eprintln!("Failed to read index.html: {err}");
            let response = Response::from_string("Internal Server Error")
                .with_status_code(StatusCode(500));
            let _ = request.respond(response);
            return;
        }
    };

    let mut response = Response::from_string(body).with_status_code(StatusCode(200));

    if let Ok(header) = Header::from_bytes(
        b"Content-Type" as &[u8],
        b"text/html; charset=utf-8" as &[u8],
    ) {
        response.add_header(header);
    }

    let _ = request.respond(response);
}
