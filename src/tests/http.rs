use super::*;
use std::{io::Write as _, net::TcpListener};

fn request(url: &str) -> Request<'_> {
    Request {
        method: "GET",
        url,
        token: "disposable-fixture-token",
        body: None,
        timeout: 15,
    }
}

#[test]
fn failed_client_initialisation_is_reported_consistently_without_sending_a_request() {
    let failure = reqwest::blocking::Client::builder()
        .user_agent("invalid\r\nheader")
        .build()
        .unwrap_err();
    let transport = HttpTransport {
        client: std::sync::OnceLock::from(Err(failure)),
    };
    for _ in 0..2 {
        let error = transport
            .response(request("http://fixture.invalid/never-requested"))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("could not initialise HTTP transport:")
        );
        assert!(error.downcast_ref::<TransientRead>().is_none());
        assert!(!error.to_string().contains("disposable-fixture-token"));
    }
}

#[test]
fn oversized_http_observations_are_refused_after_the_bounded_read() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut header = Vec::new();
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        let length = LIMIT + 1;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let block = [b'x'; 8192];
        let mut remaining = length;
        while remaining != 0 {
            let length = usize::try_from(remaining.min(block.len() as u64)).unwrap();
            stream.write_all(&block[..length]).unwrap();
            remaining -= length as u64;
        }
    });
    let error = HttpTransport::default()
        .response(request(&format!("http://{address}/oversized")))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "HTTP response exceeded 64 MiB; refusing an incomplete observation"
    );
    assert!(error.downcast_ref::<TransientRead>().is_none());
    assert!(error.downcast_ref::<HttpStatusError>().is_none());
    worker.join().unwrap();
}

#[test]
fn truncated_http_observations_are_retryable_reads_with_an_unconfirmed_remote_result() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut header = Vec::new();
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npartial",
            )
            .unwrap();
    });
    let error = HttpTransport::default()
        .response(request(&format!("http://{address}/truncated")))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "could not read the complete HTTP response; the remote result is unconfirmed"
    );
    assert!(error.downcast_ref::<TransientRead>().is_some());
    assert!(error.downcast_ref::<HttpStatusError>().is_none());
    worker.join().unwrap();
}

#[test]
fn status_diagnostics_truncate_on_character_boundaries_and_preserve_retry_advice() {
    let error = HttpStatusError {
        url: "https://fixture.invalid/pulls".into(),
        status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
        body: format!("{}unshown-sensitive-detail", "界".repeat(2048)),
        quota: crate::github::limits::Quota::default(),
    };
    let message = error.to_string();
    assert_eq!(message.matches('界').count(), 2048);
    assert!(message.contains("[response diagnostic truncated]"));
    assert!(!message.contains("unshown-sensitive-detail"));
    assert!(message.contains("verify whether any requested mutation completed before retrying"));
}

#[test]
fn status_diagnostics_propagate_output_failures_in_both_the_header_and_truncation_marker() {
    struct RejectingWriter(&'static str);
    impl fmt::Write for RejectingWriter {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            if text.contains(self.0) {
                Err(fmt::Error)
            } else {
                Ok(())
            }
        }
    }
    let error = HttpStatusError {
        url: "https://fixture.invalid/pulls".into(),
        status: reqwest::StatusCode::BAD_GATEWAY,
        body: "x".repeat(2049),
        quota: crate::github::limits::Quota::default(),
    };
    for rejected in ["HTTP", "response diagnostic truncated"] {
        assert!(fmt::write(&mut RejectingWriter(rejected), format_args!("{error}")).is_err());
    }
}
