use patcher_core::{
    model::{Codec, Object},
    transfer::{Control, HttpSource, ObjectSource},
};
use std::{
    fs,
    io::Cursor,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tiny_http::{Header, Response, Server};

type Reply = Response<Cursor<Vec<u8>>>;

struct Fake {
    origin: url::Url,
    /// The Range header of every request received, in order.
    ranges: Arc<Mutex<Vec<Option<String>>>>,
}

fn serve(handler: impl Fn(usize, Option<&str>) -> Reply + Send + 'static) -> Fake {
    let server = Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let seen = ranges.clone();
    std::thread::spawn(move || {
        while let Ok(Some(request)) = server.recv_timeout(Duration::from_secs(10)) {
            let range = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Range"))
                .map(|h| h.value.to_string());
            let n = {
                let mut seen = seen.lock().unwrap();
                seen.push(range.clone());
                seen.len() - 1
            };
            let _ = request.respond(handler(n, range.as_deref()));
        }
    });
    Fake {
        origin: format!("http://127.0.0.1:{port}/").parse().unwrap(),
        ranges,
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).unwrap()
}

fn data() -> Vec<u8> {
    (0..300_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}

fn object(bytes: &[u8]) -> Object {
    let hash = blake3::hash(bytes).to_hex().to_string();
    Object {
        hash: hash.clone(),
        encoded_size: bytes.len() as u64,
        codec: Codec::Raw,
        decoded_hash: hash,
        decoded_size: bytes.len() as u64,
    }
}

fn source(fake: &Fake) -> HttpSource {
    HttpSource {
        origin: fake.origin.clone(),
        max_bytes_per_second: 0,
        retry_delays: vec![Duration::ZERO; 3],
    }
}

fn full(bytes: &[u8], etag: &str) -> Reply {
    Response::from_data(bytes.to_vec()).with_header(header("ETag", etag))
}

fn partial(bytes: &[u8], from: usize, etag: &str) -> Reply {
    let range = format!("bytes {}-{}/{}", from, bytes.len() - 1, bytes.len());
    Response::from_data(bytes[from..].to_vec())
        .with_status_code(206)
        .with_header(header("Content-Range", &range))
        .with_header(header("ETag", etag))
}

/// Leaves a partial download and its saved validator, as an interrupted run would.
fn interrupted(cache: &std::path::Path, o: &Object, prefix: &[u8], etag: &str) {
    fs::create_dir_all(cache).unwrap();
    fs::write(cache.join(format!("{}.part", o.hash)), prefix).unwrap();
    let resume = serde_json::json!({ "etag": etag }).to_string();
    fs::write(cache.join(format!("{}.resume", o.hash)), resume).unwrap();
}

fn fetch(src: &HttpSource, o: &Object, cache: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let path = src.fetch(o, cache, &Control::default(), &|_, _| {})?;
    Ok(fs::read(path)?)
}

#[test]
fn full_download_verifies_and_caches() {
    let bytes = data();
    let o = object(&bytes);
    let body = bytes.clone();
    let fake = serve(move |_, _| full(&body, "\"v1\""));
    let t = tempfile::tempdir().unwrap();
    assert_eq!(fetch(&source(&fake), &o, t.path()).unwrap(), bytes);
    assert_eq!(fetch(&source(&fake), &o, t.path()).unwrap(), bytes);
    assert_eq!(
        fake.ranges.lock().unwrap().len(),
        1,
        "second fetch must use the cache"
    );
}

#[test]
fn resume_continues_from_saved_offset() {
    let bytes = data();
    let o = object(&bytes);
    let body = bytes.clone();
    let fake = serve(move |_, range| match range {
        Some("bytes=100000-") => partial(&body, 100_000, "\"v1\""),
        _ => Response::from_data(vec![]).with_status_code(500),
    });
    let t = tempfile::tempdir().unwrap();
    interrupted(t.path(), &o, &bytes[..100_000], "\"v1\"");
    assert_eq!(fetch(&source(&fake), &o, t.path()).unwrap(), bytes);
    assert_eq!(
        *fake.ranges.lock().unwrap(),
        vec![Some("bytes=100000-".to_string())]
    );
}

#[test]
fn server_ignoring_range_restarts_from_zero() {
    let bytes = data();
    let o = object(&bytes);
    let body = bytes.clone();
    let fake = serve(move |_, _| full(&body, "\"v1\""));
    let t = tempfile::tempdir().unwrap();
    interrupted(t.path(), &o, &vec![7u8; 100_000], "\"v1\"");
    assert_eq!(fetch(&source(&fake), &o, t.path()).unwrap(), bytes);
}

#[test]
fn changed_validator_discards_partial_bytes() {
    let bytes = data();
    let o = object(&bytes);
    let body = bytes.clone();
    let fake = serve(move |n, _| {
        if n == 0 {
            partial(&body, 100_000, "\"v2\"")
        } else {
            full(&body, "\"v2\"")
        }
    });
    let t = tempfile::tempdir().unwrap();
    interrupted(t.path(), &o, &bytes[..100_000], "\"v1\"");
    assert_eq!(fetch(&source(&fake), &o, t.path()).unwrap(), bytes);
    let ranges = fake.ranges.lock().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[1], None, "retry must start from zero");
}

#[test]
fn wrong_content_range_never_yields_a_bad_object() {
    let bytes = data();
    let o = object(&bytes);
    let body = bytes.clone();
    let fake = serve(move |n, _| {
        if n == 0 {
            partial(&body, 90_000, "\"v1\"")
        } else {
            full(&body, "\"v1\"")
        }
    });
    let t = tempfile::tempdir().unwrap();
    interrupted(t.path(), &o, &bytes[..100_000], "\"v1\"");
    assert_eq!(fetch(&source(&fake), &o, t.path()).unwrap(), bytes);
}

#[test]
fn range_not_satisfiable_restarts_cleanly() {
    let bytes = data();
    let o = object(&bytes);
    let body = bytes.clone();
    let fake = serve(move |n, _| {
        if n == 0 {
            Response::from_data(vec![]).with_status_code(416)
        } else {
            full(&body, "\"v1\"")
        }
    });
    let t = tempfile::tempdir().unwrap();
    interrupted(t.path(), &o, &bytes[..100_000], "\"v1\"");
    assert_eq!(fetch(&source(&fake), &o, t.path()).unwrap(), bytes);
    assert_eq!(fake.ranges.lock().unwrap()[1], None);
}

#[test]
fn transient_status_is_retried_after_server_delay() {
    let bytes = data();
    let o = object(&bytes);
    let body = bytes.clone();
    let fake = serve(move |n, _| {
        if n == 0 {
            Response::from_data(vec![])
                .with_status_code(503)
                .with_header(header("Retry-After", "1"))
        } else {
            full(&body, "\"v1\"")
        }
    });
    let t = tempfile::tempdir().unwrap();
    let started = Instant::now();
    assert_eq!(fetch(&source(&fake), &o, t.path()).unwrap(), bytes);
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "Retry-After was ignored"
    );
    assert_eq!(fake.ranges.lock().unwrap().len(), 2);
}

#[test]
fn permanent_status_is_not_retried() {
    let o = object(&data());
    let fake = serve(|_, _| Response::from_data(vec![]).with_status_code(404));
    let t = tempfile::tempdir().unwrap();
    assert!(fetch(&source(&fake), &o, t.path()).is_err());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(fake.ranges.lock().unwrap().len(), 1);
}

#[test]
fn redirect_is_not_followed() {
    let o = object(&data());
    let fake = serve(|_, _| {
        Response::from_data(vec![])
            .with_status_code(302)
            .with_header(header("Location", "http://127.0.0.1:9/elsewhere"))
    });
    let t = tempfile::tempdir().unwrap();
    assert!(fetch(&source(&fake), &o, t.path()).is_err());
    assert_eq!(fake.ranges.lock().unwrap().len(), 1);
}

#[test]
fn corrupt_bytes_exhaust_the_budget_and_leave_no_object() {
    let bytes = data();
    let o = object(&bytes);
    let mut bad = bytes.clone();
    bad[5] ^= 1;
    let fake = serve(move |_, _| full(&bad, "\"v1\""));
    let t = tempfile::tempdir().unwrap();
    assert!(fetch(&source(&fake), &o, t.path()).is_err());
    assert_eq!(
        fake.ranges.lock().unwrap().len(),
        4,
        "first attempt plus three retries"
    );
    assert!(!t.path().join(&o.hash).exists());
    assert!(!t.path().join(format!("{}.part", o.hash)).exists());
}

#[test]
fn oversized_body_is_rejected() {
    let bytes = data();
    let o = object(&bytes);
    let mut long = bytes.clone();
    long.extend_from_slice(b"extra");
    let fake = serve(move |_, _| full(&long, "\"v1\""));
    let t = tempfile::tempdir().unwrap();
    assert!(fetch(&source(&fake), &o, t.path()).is_err());
    assert!(!t.path().join(&o.hash).exists());
}

#[test]
fn bandwidth_limit_is_enforced() {
    let bytes = data();
    let o = object(&bytes);
    let body = bytes.clone();
    let fake = serve(move |_, _| full(&body, "\"v1\""));
    let t = tempfile::tempdir().unwrap();
    let mut src = source(&fake);
    src.max_bytes_per_second = 200_000;
    let started = Instant::now();
    assert_eq!(fetch(&src, &o, t.path()).unwrap(), bytes);
    assert!(
        started.elapsed() >= Duration::from_millis(1300),
        "300 kB at 200 kB/s"
    );
}

#[test]
fn pause_interrupts_a_retry_wait() {
    let o = object(&data());
    let fake = serve(|_, _| Response::from_data(vec![]).with_status_code(503));
    let t = tempfile::tempdir().unwrap();
    let mut src = source(&fake);
    src.retry_delays = vec![Duration::from_secs(30); 3];
    let control = Control::default();
    let stop = control.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        stop.stop();
    });
    let started = Instant::now();
    assert!(src.fetch(&o, t.path(), &control, &|_, _| {}).is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn plain_http_is_refused_for_remote_origins() {
    let o = object(&data());
    let src = HttpSource::new("http://example.com/".parse().unwrap(), 0);
    let t = tempfile::tempdir().unwrap();
    assert!(
        src.fetch(&o, t.path(), &Control::default(), &|_, _| {})
            .is_err()
    );
}
