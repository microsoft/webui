use tiny_http::{Request, Response, StatusCode};

// 404 route for any routes not defined
pub fn handle_not_found(request: Request) {
    let response = Response::from_string("Not Found").with_status_code(StatusCode(404));
    let _ = request.respond(response);
}
