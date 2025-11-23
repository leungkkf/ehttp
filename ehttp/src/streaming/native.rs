use crate::Request;
use std::{io::Read, ops::ControlFlow};

use super::Part;

pub fn fetch_streaming_blocking(
    request: Request,
    on_data: Box<dyn Fn(crate::Result<Part>) -> ControlFlow<()> + Send>,
) {
    let (ureq_resp, partial_response) = match crate::ureq_ext::get_response(&request) {
        Ok(resp) => resp,
        Err(err) => {
            on_data(Err(err));
            return;
        }
    };

    if on_data(Ok(Part::Response(partial_response))).is_break() {
        return;
    };

    let (_, body) = ureq_resp.into_parts();
    let mut reader = body.into_reader();
    loop {
        let mut buf = vec![0; 2048];
        match reader.read(&mut buf) {
            Ok(n) if n > 0 => {
                // clone data from buffer and clear it
                let chunk = buf[..n].to_vec();
                if on_data(Ok(Part::Chunk(chunk))).is_break() {
                    return;
                };
            }
            Ok(_) => {
                on_data(Ok(Part::Chunk(vec![])));
                break;
            }
            Err(err) => {
                if request.method == "HEAD" && err.kind() == std::io::ErrorKind::UnexpectedEof {
                    // We don't really expect a body for HEAD requests, so this is fine.
                    on_data(Ok(Part::Chunk(vec![])));
                    break;
                } else {
                    on_data(Err(format!("Failed to read response body: {err}")));
                    return;
                }
            }
        };
    }
}

pub(crate) fn fetch_streaming(
    request: Request,
    on_data: Box<dyn Fn(crate::Result<Part>) -> ControlFlow<()> + Send>,
) {
    std::thread::Builder::new()
        .name("ehttp".to_owned())
        .spawn(move || fetch_streaming_blocking(request, on_data))
        .expect("Failed to spawn ehttp thread");
}
