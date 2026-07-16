use std::{
    io::Write,
    net::{Ipv4Addr, TcpListener, TcpStream},
    time::Duration,
};

use super::{request, TestDirectory};
use crate::media_protocol::{http::read_request_with_timeout, MediaOwner};

#[test]
fn head_and_single_ranges_report_exact_lengths_and_media_headers() {
    let directory = TestDirectory::new("ranges");
    let path = directory.join("meeting.wav");
    std::fs::write(&path, b"0123456789").unwrap();
    let owner = MediaOwner::with_capacity_for_test(4);
    let admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();

    let head = request(&admission.url, "HEAD", None);
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    assert!(head.headers.contains("Content-Length: 10"));
    assert!(head.headers.contains("Content-Type: audio/wav"));
    assert!(head.headers.contains("Accept-Ranges: bytes"));

    let middle = request(&admission.url, "GET", Some("bytes=2-5"));
    assert_eq!(middle.status, 206);
    assert_eq!(middle.body, b"2345");
    assert!(middle.headers.contains("Content-Range: bytes 2-5/10"));

    assert_eq!(
        request(&admission.url, "GET", Some("bytes=7-")).body,
        b"789"
    );
    assert_eq!(
        request(&admission.url, "GET", Some("bytes=-3")).body,
        b"789"
    );
}

#[test]
fn malformed_multi_range_and_eof_requests_fail_without_media_bytes() {
    let directory = TestDirectory::new("bad-ranges");
    let path = directory.join("meeting.webm");
    std::fs::write(&path, b"0123456789").unwrap();
    let owner = MediaOwner::with_capacity_for_test(4);
    let admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();

    let malformed = request(&admission.url, "GET", Some("bytes=wat"));
    assert_eq!(malformed.status, 400);
    assert!(malformed.body.is_empty());

    let multiple = request(&admission.url, "GET", Some("bytes=0-1,4-5"));
    assert_eq!(multiple.status, 416);
    assert!(multiple.body.is_empty());

    let eof = request(&admission.url, "GET", Some("bytes=10-"));
    assert_eq!(eof.status, 416);
    assert!(eof.headers.contains("Content-Range: bytes */10"));
    assert!(eof.body.is_empty());
}

#[test]
fn request_headers_have_an_absolute_deadline_across_fragmented_reads() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let authority = listener.local_addr().unwrap().to_string();
    let mut client = TcpStream::connect(&authority).unwrap();
    let (mut server, _) = listener.accept().unwrap();
    let writer = std::thread::spawn(move || {
        for byte in b"GET /media/slow" {
            if client.write_all(std::slice::from_ref(byte)).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    });

    let status =
        read_request_with_timeout(&mut server, &authority, Duration::from_millis(40)).unwrap_err();

    assert_eq!(status, 408);
    drop(server);
    writer.join().unwrap();
}
