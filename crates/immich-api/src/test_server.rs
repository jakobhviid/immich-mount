//! A tiny HTTP/1.1 server for unit tests: it answers each connection with the next canned response and records
//! what it was sent. One request per connection (`Connection: close`), which is all the client tests need.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A response the fake server sends.
pub(crate) struct CannedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl CannedResponse {
    pub(crate) fn json(status: u16, body: &str) -> CannedResponse {
        CannedResponse {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn bytes(status: u16, content_type: &str, body: &[u8]) -> CannedResponse {
        CannedResponse {
            status,
            headers: vec![("content-type".to_owned(), content_type.to_owned())],
            body: body.to_vec(),
        }
    }

    pub(crate) fn with_header(mut self, name: &str, value: &str) -> CannedResponse {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// A request the fake server received.
#[derive(Debug, Clone)]
pub(crate) struct RecordedRequest {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    }

    pub(crate) fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub(crate) struct FakeServer {
    pub url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeServer {
    /// Start serving `responses`, one per connection, in order.
    pub(crate) async fn start(responses: Vec<CannedResponse>) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        tokio::spawn(async move {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let Some(request) = read_request(&mut stream).await else {
                    continue;
                };
                recorded.lock().unwrap().push(request);
                let _ = write_response(&mut stream, &response).await;
            }
        });
        FakeServer { url, requests }
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    let mut received = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(position) = find(&received, b"\r\n\r\n") {
            break position;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        received.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&received[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_owned();
    let target = request_line.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let content_length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = received[head_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Some(RecordedRequest {
        method,
        target,
        headers,
        body,
    })
}

async fn write_response(stream: &mut TcpStream, response: &CannedResponse) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} Canned\r\n", response.status);
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", response.body.len()));
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.shutdown().await
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}
