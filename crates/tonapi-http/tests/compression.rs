//! Compression des réponses amont : le client l'annonce, la décode, et le plafond de taille
//! s'applique aux octets décompressés. Faux serveur axum local, aucune requête réelle.

use std::fmt::Write as _;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, header};
use axum::response::IntoResponse;
use axum::routing::get;
use reqwest::Method;
use reqwest::header::HeaderMap as ClientHeaders;
use tonapi_http::{Browser, BrowserOptions, CookieJar, HttpError};
use url::Url;

const MAX_BODY: usize = 8 * 1024 * 1024;

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// Page HTML synthétique, répétitive comme un relevé de notes.
fn page(target: usize) -> Vec<u8> {
    let mut html = String::from("<html><body><table>");
    let mut i = 0_usize;
    while html.len() < target {
        let _ = writeln!(
            html,
            "<tr class=\"row\"><td>UE{i:04} Module</td><td>{}.{:02}</td><td>{}</td></tr>",
            i % 20,
            i * 7 % 100,
            i % 6 + 1
        );
        i += 1;
    }
    html.push_str("</table></body></html>");
    html.into_bytes()
}

struct Server {
    url: Url,
    /// Octets de corps réellement envoyés sur la connexion.
    wire: Arc<AtomicUsize>,
    /// Valeurs d'`Accept-Encoding` reçues.
    accepts: Arc<std::sync::Mutex<Vec<String>>>,
}

/// Sert `body` ; compressé en gzip si le client l'accepte, ou toujours si `force_gzip`.
async fn serve(body: Vec<u8>, force_gzip: bool) -> Server {
    let plain = Bytes::from(body);
    let packed = Bytes::from(gzip(&plain));
    let wire = Arc::new(AtomicUsize::new(0));
    let accepts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (w, a) = (wire.clone(), accepts.clone());
    let app = Router::new().route(
        "/",
        get(move |headers: HeaderMap| {
            let accept = headers
                .get(header::ACCEPT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            let compressed = force_gzip || accept.contains("gzip");
            a.lock().unwrap().push(accept);
            let (plain, packed, w) = (plain.clone(), packed.clone(), w.clone());
            async move {
                if compressed {
                    w.fetch_add(packed.len(), Ordering::Relaxed);
                    ([(header::CONTENT_ENCODING, "gzip")], packed).into_response()
                } else {
                    w.fetch_add(plain.len(), Ordering::Relaxed);
                    plain.into_response()
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { url, wire, accepts }
}

async fn fetch(url: &Url) -> Result<Vec<u8>, HttpError> {
    let options = BrowserOptions {
        https_only: false,
        ..BrowserOptions::default()
    };
    let mut browser = Browser::new(&options, CookieJar::default())?;
    let (_, _, bytes) = browser
        .send_bytes(Method::GET, url.clone(), ClientHeaders::new(), None, None)
        .await?;
    Ok(bytes)
}

#[tokio::test]
async fn un_corps_gzip_est_annonce_puis_decompresse() {
    let html = page(300 * 1024);
    let server = serve(html.clone(), false).await;
    assert_eq!(fetch(&server.url).await.unwrap(), html);
    let accepts = server.accepts.lock().unwrap().clone();
    assert!(accepts.iter().all(|a| a.contains("gzip")), "{accepts:?}");
    assert!(server.wire.load(Ordering::Relaxed) < html.len() / 3);
}

#[tokio::test]
async fn le_plafond_s_applique_aux_octets_decompresses() {
    // ~9 Ko sur le fil, plus de 8 Mo une fois décompressé : le `Content-Length` annoncé ne dit rien.
    let bomb = vec![b'a'; MAX_BODY + 1024 * 1024];
    let server = serve(bomb, true).await;
    assert_eq!(server.wire.load(Ordering::Relaxed), 0);
    let result = fetch(&server.url).await;
    assert!(
        matches!(result, Err(HttpError::TooLarge)),
        "{:?}",
        result.map(|b| b.len())
    );
    assert!(server.wire.load(Ordering::Relaxed) < MAX_BODY / 100);
}

#[tokio::test]
async fn un_corps_decompresse_sous_le_plafond_passe() {
    let html = vec![b'a'; MAX_BODY - 1024];
    let server = serve(html.clone(), true).await;
    assert_eq!(fetch(&server.url).await.unwrap().len(), html.len());
}

/// Banc (ignoré) : octets sur le fil et durée d'une récupération, page de 1 Mo.
/// `cargo test -p tonapi-http --release --test compression -- --ignored --nocapture banc`.
#[tokio::test]
#[ignore = "banc : mesure, pas de vérification"]
async fn banc_compression() {
    let html = page(1024 * 1024);
    let server = serve(html.clone(), false).await;
    let mut ms = Vec::new();
    for _ in 0..101 {
        let started = Instant::now();
        assert_eq!(fetch(&server.url).await.unwrap().len(), html.len());
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    ms.sort_by(f64::total_cmp);
    println!(
        "page {} octets ; sur le fil par requête : {} octets ; durée loopback médiane {:.2} ms, p95 {:.2} ms",
        html.len(),
        server.wire.load(Ordering::Relaxed) / 101,
        ms[50],
        ms[95]
    );
}
