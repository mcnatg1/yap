use std::{
    io::{Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

const MAX_REQUEST_HEADER_BYTES: usize = 16 * 1024;
pub(super) const SOCKET_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub(super) struct HttpRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) range: Option<String>,
}

pub(super) fn read_request(stream: &mut TcpStream, authority: &str) -> Result<HttpRequest, u16> {
    read_request_with_timeout(stream, authority, SOCKET_TIMEOUT)
}

pub(super) fn read_request_with_timeout(
    stream: &mut TcpStream,
    authority: &str,
    timeout: Duration,
) -> Result<HttpRequest, u16> {
    let deadline = Instant::now().checked_add(timeout).ok_or(400_u16)?;
    let mut bytes = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(408_u16)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| 400_u16)?;
        let read = stream.read(&mut buffer).map_err(|error| {
            if matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                408_u16
            } else {
                400_u16
            }
        })?;
        if read == 0 {
            return Err(400);
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_REQUEST_HEADER_BYTES {
            return Err(431);
        }
        if Instant::now() >= deadline {
            return Err(408);
        }
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
    };
    let text = std::str::from_utf8(&bytes[..header_end]).map_err(|_| 400_u16)?;
    let mut lines = text.split("\r\n");
    let mut request_line = lines.next().ok_or(400_u16)?.split_whitespace();
    let method = request_line.next().ok_or(400_u16)?.to_string();
    let path = request_line.next().ok_or(400_u16)?.to_string();
    if request_line.next() != Some("HTTP/1.1") || request_line.next().is_some() {
        return Err(400);
    }
    let mut host = None;
    let mut range = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(400_u16)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("host") {
            if host.replace(value).is_some() {
                return Err(400);
            }
        } else if name.eq_ignore_ascii_case("range") && range.replace(value.to_string()).is_some() {
            return Err(400);
        }
    }
    if host != Some(authority) {
        return Err(400);
    }
    Ok(HttpRequest {
        method,
        path,
        range,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SelectedRange {
    pub(super) length: u64,
    pub(super) partial: bool,
    pub(super) start: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RangeError {
    Malformed,
    Multiple,
    Unsatisfiable,
}

pub(super) fn select_range(value: Option<&str>, length: u64) -> Result<SelectedRange, RangeError> {
    let Some(value) = value else {
        return Ok(SelectedRange {
            length,
            partial: false,
            start: 0,
        });
    };
    let Some(specification) = value.strip_prefix("bytes=") else {
        return Err(RangeError::Malformed);
    };
    if specification.contains(',') {
        return Err(RangeError::Multiple);
    }
    let (start, end) = specification.split_once('-').ok_or(RangeError::Malformed)?;
    if length == 0 {
        return Err(RangeError::Unsatisfiable);
    }
    if start.is_empty() {
        let suffix = parse_decimal_u64(end)?;
        if suffix == 0 {
            return Err(RangeError::Unsatisfiable);
        }
        let selected_length = suffix.min(length);
        return Ok(SelectedRange {
            length: selected_length,
            partial: true,
            start: length - selected_length,
        });
    }

    let start = parse_decimal_u64(start)?;
    if start >= length {
        return Err(RangeError::Unsatisfiable);
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        let end = parse_decimal_u64(end)?;
        if end < start {
            return Err(RangeError::Malformed);
        }
        end.min(length - 1)
    };
    Ok(SelectedRange {
        length: end - start + 1,
        partial: true,
        start,
    })
}

fn parse_decimal_u64(value: &str) -> Result<u64, RangeError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RangeError::Malformed);
    }
    value.parse().map_err(|_| RangeError::Malformed)
}

pub(super) fn write_request_error(
    stream: &mut TcpStream,
    status: u16,
    length: u64,
) -> std::io::Result<()> {
    match status {
        408 => write_empty_response(stream, status, "Request Timeout", &[]),
        416 => write_empty_response(
            stream,
            status,
            "Range Not Satisfiable",
            &[("Content-Range", format!("bytes */{length}"))],
        ),
        431 => write_empty_response(stream, status, "Request Header Fields Too Large", &[]),
        _ => write_empty_response(stream, status, "Bad Request", &[]),
    }
}

pub(super) fn write_empty_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    extra_headers: &[(&str, String)],
) -> std::io::Result<()> {
    let mut headers = vec![("Content-Length", "0".into())];
    headers.extend_from_slice(extra_headers);
    write_response_head(stream, status, reason, &headers)
}

pub(super) fn write_response_head(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, String)],
) -> std::io::Result<()> {
    let mut response = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("Access-Control-Allow-Origin: *\r\n");
    response.push_str("Cache-Control: no-store\r\n");
    response.push_str("X-Content-Type-Options: nosniff\r\n");
    response.push_str("Connection: close\r\n\r\n");
    stream.write_all(response.as_bytes())
}
