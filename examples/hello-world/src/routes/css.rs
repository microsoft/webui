use std::fs;
use tiny_http::{Header, Request, Response, StatusCode};

// CSS route for the main stylesheet
pub fn handle_css(request: Request) {
    let body = match fs::read_to_string("static/styles.css") {
        Ok(contents) => contents,
        Err(err) => {
            eprintln!("Failed to read styles.css: {err}");
            let response = Response::from_string("Not Found")
                .with_status_code(StatusCode(404));
            let _ = request.respond(response);
            return;
        }
    };

    let mut response = Response::from_string(body).with_status_code(StatusCode(200));

    if let Ok(header) = Header::from_bytes(
        b"Content-Type" as &[u8],
        b"text/css; charset=utf-8" as &[u8],
    ) {
        response.add_header(header);
    }

    let _ = request.respond(response);
}
