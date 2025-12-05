use crate::{PartialResponse, Request};
use std::str::FromStr;
use ureq::{http::Method, Agent, Body, RequestExt, ResponseExt};

/// Helper to get the ureq response using ureq and the partial response.
pub(crate) fn get_response(
    request: &Request,
) -> crate::Result<(ureq::http::response::Response<Body>, PartialResponse)> {
    let method = Method::from_str(&request.method)
        .map_err(|_| format!("invalid method '{}'", &request.method))?;

    let mut req_builder = ureq::http::Request::builder()
        .method(method)
        .uri(&request.url);

    for (k, v) in &request.headers {
        req_builder = req_builder.header(k, v);
    }

    let agent = Agent::new_with_defaults();

    // http_status_as_error is set to false to have the response
    // even if the http status code is not OK, e.g. 404.
    let resp = if request.body.is_empty() {
        req_builder
            .body(())
            .map_err(|e| format!("failed to create request (no body) '{:?}'", e))?
            .with_agent(agent)
            .configure()
            .timeout_global(request.timeout)
            .http_status_as_error(false)
            .run()
    } else {
        req_builder
            .body(&request.body)
            .map_err(|e| format!("failed to create request (with body) '{:?}'", e))?
            .with_agent(agent)
            .configure()
            .timeout_global(request.timeout)
            .http_status_as_error(false)
            .run()
    };

    let ureq_resp = resp.map_err(|e| e.to_string())?;

    // Keep the logic in ureq 2.0 where it returns false with http error code >= 400.
    let ok = ureq_resp.status().as_u16() < 400;
    let url = ureq_resp.get_uri().to_owned().to_string();
    let status = ureq_resp.status().as_u16();
    let status_text = ureq_resp
        .status()
        .canonical_reason()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let mut headers = crate::Headers::default();
    for (key, value) in ureq_resp.headers() {
        if let Ok(value) = value.to_str() {
            headers.insert(key.as_str().to_ascii_lowercase(), value.to_owned());
        }
    }
    headers.sort(); // It reads nicer, and matches web backend.

    let partial_response = PartialResponse {
        url,
        ok,
        status,
        status_text,
        headers,
    };

    Ok((ureq_resp, partial_response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{prelude::*, Mock};
    use std::time::Duration;

    /// Set up a mock GET server.
    fn mock_get<'a>(
        server: &'a MockServer,
        status: u16,
        body: &str,
        sleep: Option<Duration>,
    ) -> Mock<'a> {
        server.mock(|when, then| {
            when.method(GET)
                .path("/translate")
                .header("Authorization", "token 123456789")
                .query_param("word", "hello");

            then.status(status)
                .header("content-type", "text/html")
                .body(body)
                .and(|then| {
                    if let Some(sleep) = sleep {
                        then.delay(sleep)
                    } else {
                        then
                    }
                });
        })
    }

    /// Set up a mock POST server.
    fn mock_post<'a>(server: &'a MockServer, status: u16, body: &str) -> Mock<'a> {
        server.mock(|when, then| {
            when.method(POST)
                .path("/books")
                .body("The Fellowship of the Ring");
            then.status(status).body(body);
        })
    }

    #[test]
    fn test_get_status_200() {
        // Start a lightweight mock server.
        let server = MockServer::start();

        // Create a mock on the server.
        let mock = mock_get(&server, 200, "ohi", None);
        let mut request = Request::get(server.url("/translate?word=hello"));

        request.headers.insert("Authorization", "token 123456789");

        let (mut ureq_resp, partial_resp) = get_response(&request).unwrap();

        assert!(partial_resp.ok);
        assert_eq!(partial_resp.status, 200);
        assert_eq!(partial_resp.headers.get("content-type"), Some("text/html"));
        assert_eq!(partial_resp.headers.get("content-length"), Some("3"));
        assert_eq!(partial_resp.status_text, "OK");
        assert!(partial_resp.url.ends_with("/translate?word=hello"));

        assert_eq!(ureq_resp.body_mut().read_to_string().unwrap(), "ohi");

        // Ensure the specified mock was called exactly one time.
        mock.assert();
    }

    #[test]
    fn test_get_status_403() {
        // Start a lightweight mock server.
        let server = MockServer::start();

        // Create a mock on the server.
        let mock = mock_get(&server, 403, "not allowed", None);
        // Send a GET resquest with the wrong authetication.
        let mut request = Request::get(server.url("/translate?word=hello"));

        request.headers.insert("Authorization", "token 123456789");

        let (mut ureq_resp, partial_resp) = get_response(&request).unwrap();

        // Expect not OK.
        assert!(!partial_resp.ok);
        assert_eq!(partial_resp.status, 403);
        assert_eq!(partial_resp.headers.get("content-type"), Some("text/html"));
        assert_eq!(partial_resp.headers.get("content-length"), Some("11"));
        assert_eq!(partial_resp.status_text, "Forbidden");
        assert!(partial_resp.url.ends_with("/translate?word=hello"));

        assert_eq!(
            ureq_resp.body_mut().read_to_string().unwrap(),
            "not allowed"
        );

        // Ensure the specified mock was called exactly one time.
        mock.assert();
    }

    #[test]
    fn test_post() {
        // Start a lightweight mock server.
        let server = MockServer::start();

        // Create a mock on the server.
        let mock = mock_post(&server, 201, "The Lord of the Rings");
        // Send a POST request.
        let request = Request::post(server.url("/books"), b"The Fellowship of the Ring".to_vec());

        let (mut ureq_resp, partial_resp) = get_response(&request).unwrap();

        // Expect OK.
        assert!(partial_resp.ok);
        assert_eq!(partial_resp.status, 201);
        assert_eq!(partial_resp.headers.get("content-length"), Some("21"));
        assert_eq!(partial_resp.status_text, "Created");
        assert!(partial_resp.url.ends_with("/books"));

        assert_eq!(
            ureq_resp.body_mut().read_to_string().unwrap(),
            "The Lord of the Rings"
        );

        mock.assert();
    }

    #[test]
    fn test_get_timeout() {
        // Start a lightweight mock server.
        let server = MockServer::start();

        // Create a mock on the server without a delay.
        let mock = mock_get(&server, 200, "ohi", None);
        // Specify a timeout in the request.
        let mut request = Request::get(server.url("/translate?word=hello"))
            .with_timeout(Some(Duration::from_millis(200)));

        request.headers.insert("Authorization", "token 123456789");

        let (mut ureq_resp, partial_resp) = get_response(&request).unwrap();

        assert!(partial_resp.ok);
        assert_eq!(partial_resp.status, 200);
        assert_eq!(partial_resp.headers.get("content-type"), Some("text/html"));
        assert_eq!(partial_resp.headers.get("content-length"), Some("3"));
        assert_eq!(partial_resp.status_text, "OK");
        assert!(partial_resp.url.ends_with("/translate?word=hello"));

        assert_eq!(ureq_resp.body_mut().read_to_string().unwrap(), "ohi");

        // Ensure the specified mock was called exactly one time.
        mock.assert();
    }

    #[test]
    fn test_get_timeout_with_err() {
        // Start a lightweight mock server.
        let server = MockServer::start();

        // Create a mock on the server with a delay in response.
        let mock = mock_get(&server, 200, "ohi", Some(Duration::from_secs(1)));
        // Set timeout smaller than the server delay.
        let mut request = Request::get(server.url("/translate?word=hello"))
            .with_timeout(Some(Duration::from_millis(200)));

        request.headers.insert("Authorization", "token 123456789");

        // Expect a timeout error.
        assert!(match get_response(&request) {
            Ok(_) => false,
            Err(e) => e.contains("timeout"),
        });

        // Ensure the specified mock was called exactly one time.
        mock.assert();
    }
}
