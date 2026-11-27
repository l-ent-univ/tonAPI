//! Les sorties de Tafac (Node, sanitize-html 2.18) sur un corpus, relevées dans
//! `tests/fixtures/mail/html.json`, et les cas que Node n'a pas (W1, W2, W6…).

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/mail/html.json")).expect("fixture")
}

fn resolve_logo(cid: &str) -> Option<String> {
    (cid == "logo@x").then(|| "/__ent_auth/mail/part?id=1&part=2".to_owned())
}

fn text(value: &Value) -> &str {
    value.as_str().expect("chaîne")
}

#[test]
fn corpus_du_mail_recu() {
    let fixture = fixture();
    let mut failures = Vec::new();
    for case in fixture["mail"].as_array().expect("liste") {
        let input = text(&case["input"]);
        let result = sanitize_mail_html(input, &[], &mut resolve_logo);
        if result.html != text(&case["html"])
            || result.remote_images as u64 != case["remoteImages"].as_u64().expect("nombre")
        {
            failures.push(format!(
                "{input:?}\n  attendu {:?} ({})\n  obtenu  {:?} ({})",
                case["html"], case["remoteImages"], result.html, result.remote_images
            ));
        }
    }
    for case in fixture["text"].as_array().expect("liste") {
        let html = mail_text_to_html(text(&case["input"]), &[]);
        if html != text(&case["html"]) {
            failures.push(format!(
                "texte {:?}\n  attendu {:?}\n  obtenu  {html:?}",
                case["input"], case["html"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn sources(images: &[Value]) -> Vec<InlineImageSource> {
    images
        .iter()
        .map(|image| InlineImageSource {
            cid: text(&image["cid"]).to_owned(),
            src: "data:image/png;base64,iVBORw0KGgo=".to_owned(),
            width: u32::try_from(image["width"].as_u64().expect("largeur")).expect("u32"),
            height: u32::try_from(image["height"].as_u64().expect("hauteur")).expect("u32"),
            alt: text(&image["alt"]).to_owned(),
        })
        .collect()
}

#[test]
fn corpus_de_l_editeur() {
    let fixture = fixture();
    let references = [
        json!({ "cid": "tafac-img-0123abcd-0000-4000-8000-000000000000@tafac.fr", "source": "upload", "aid": "upload-1", "width": 560, "height": 300 }),
        json!({ "cid": "tafac-img-89abcdef-0000-4000-8000-000000000000@tafac.fr", "source": "message", "messageId": "demo-draft-1", "part": "3", "width": 10, "height": 10 }),
    ];
    let signature = "<div style=\"margin:14px 0 6px;font-size:14px;line-height:1.45;color:#6b625b\"><div>—</div><div style=\"font-weight:600;color:#1d1d1f\">Camille Martin</div></div>";
    let mut failures = Vec::new();
    let mut check = |label: &str, input: &str, expected: &Value, got: &str| {
        if text(expected) != got {
            failures.push(format!(
                "{label} {input:?}\n  attendu {expected:?}\n  obtenu  {got:?}"
            ));
        }
    };
    for case in fixture["composer"].as_array().expect("liste") {
        let input = text(&case["input"]);
        let clean = sanitize_composer_html(input);
        check("clean", input, &case["clean"], &clean);
        check(
            "texte",
            input,
            &case["text"],
            &composer_html_to_text(&clean, ""),
        );
        check(
            "texte+sig",
            input,
            &case["textSig"],
            &composer_html_to_text(&clean, "-- \nCamille Martin\nTP 3"),
        );
        let sets: [&[Value]; 3] = [&references, &references[1..], &[]];
        for (refs, expected) in sets.iter().zip(case["select"].as_array().expect("liste")) {
            let (html, images) = select_inline_images(&clean, refs);
            check("select", input, &expected["html"], &html);
            let images_json = Value::Array(images.clone()).to_string();
            check(
                "images",
                input,
                &Value::String(expected["images"].to_string()),
                &images_json,
            );
            let sources = sources(&images);
            check(
                "built",
                input,
                &expected["built"],
                &build_mail_html(&html, signature, false, &sources),
            );
            check(
                "draft",
                input,
                &expected["draft"],
                &build_mail_html(&html, "", true, &sources),
            );
            check(
                "plain",
                input,
                &expected["plain"],
                &build_mail_html(&html, "", false, &[]),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn corpus_des_signatures() {
    let fixture = fixture();
    let mut failures = Vec::new();
    let mut check = |label: &str, input: &str, expected: &Value, got: &str| {
        if text(expected) != got {
            failures.push(format!(
                "{label} {input:?}\n  attendu {expected:?}\n  obtenu  {got:?}"
            ));
        }
    };
    for case in fixture["signature"].as_array().expect("liste") {
        let content = if let Some(input) = case["input"].as_str() {
            let clean = sanitize_signature_html(input);
            check("html", input, &case["html"], &clean.html);
            assert_eq!(
                Some(clean.has_images),
                case["hasImages"].as_bool(),
                "{input:?}"
            );
            check(
                "texte",
                input,
                &case["text"],
                &signature_html_to_text(&clean.html),
            );
            read_signature_content(Some(input), Some("Repli"))
        } else {
            let input = text(&case["text"]);
            check(
                "plain",
                input,
                &case["plain"],
                &plain_signature_to_html(input),
            );
            read_signature_content(None, Some(input))
        };
        let label = case["input"]
            .as_str()
            .or(case["text"].as_str())
            .unwrap_or_default();
        check(
            "content.html",
            label,
            &case["content"]["html"],
            &content.html,
        );
        check(
            "content.text",
            label,
            &case["content"]["text"],
            &content.text,
        );
        assert_eq!(
            Some(content.has_images),
            case["content"]["hasImages"].as_bool(),
            "{label:?}"
        );
        let framed = frame_signature(&content.html, &content.text);
        check("framed.html", label, &case["framed"]["html"], &framed.html);
        check("framed.text", label, &case["framed"]["text"], &framed.text);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ── Ce que Node n'a pas ──

fn clean(html: &str) -> String {
    sanitize_mail_html(html, &["tafac.fr", "dev.tafac.fr"], &mut resolve_logo).html
}

#[test]
fn une_longue_suite_de_lettres_reste_rapide() {
    // W1 : 1 Mo de lettres dans un attribut style et dans un <style>.
    let letters = "a".repeat(1_000_000);
    for html in [
        format!("<p style=\"color:{letters}\">x</p>"),
        format!("<style>{letters}</style>"),
        format!("<style>url({})</style>", " ".repeat(1_000_000)),
        format!("<p style=\"background:url({}\">x</p>", " ".repeat(200_000)),
    ] {
        let started = Instant::now();
        let result = sanitize_mail_html(&html, &[], &mut resolve_logo);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert!(!result.html.is_empty());
    }
    let started = Instant::now();
    let css = rewrite_css_urls(&letters, &mut |_| None);
    assert_eq!(css.held.len(), letters.len());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn imbrication_limitee() {
    // W2 : 100 000 balises imbriquées, dans les trois nettoyages.
    let deep = format!("{}x{}", "<div>".repeat(100_000), "</div>".repeat(100_000));
    let started = Instant::now();
    let html = clean(&deep);
    assert_eq!(html.matches("<div").count(), NESTING_LIMIT);
    assert!(html.contains('x'));
    let lists = format!("{}x", "<ul><li>".repeat(50_000));
    let composer = sanitize_composer_html(&lists);
    assert!(composer.matches("<ul>").count() <= NESTING_LIMIT);
    let _ = build_mail_html(&composer, "", false, &[]);
    let _ = composer_html_to_text(&composer, "");
    let signature = sanitize_signature_html(&deep);
    assert!(signature.html.matches("<div>").count() <= NESTING_LIMIT);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    // Un <style> trop profond n'est jamais recopié brut.
    let hidden = format!(
        "{}<style><img src=x onerror=alert(1)></style>",
        "<div>".repeat(150)
    );
    let html = clean(&hidden);
    assert!(!html.contains("<img"), "{html}");
}

#[test]
fn javascript_deguise() {
    for href in [
        "&#x6a;avascript:alert(1)",
        "&#106;avascript:alert(1)",
        "java&#x09;script:alert(1)",
        "\u{1}javascript:alert(1)",
        "java\tscript:alert(1)",
        " javascript:alert(1)",
        "javascript&colon;alert(1)",
        "jav&NewLine;ascript:alert(1)",
    ] {
        let html = clean(&format!(
            "<a href=\"{href}\">x</a><blockquote cite=\"{href}\">q</blockquote>"
        ));
        assert!(!html.contains("href"), "{href:?} → {html}");
        assert!(!html.contains("cite"), "{href:?} → {html}");
    }
}

#[test]
fn liens_vers_tafac() {
    // W6 : un lien vers les hôtes de Tafac, ou un http(s) qui ne s'écrit pas `schéma://`, perd
    // son adresse.
    for href in [
        "https://tafac.fr/__ent_auth/launch?url=x",
        "https://TAFAC.fr./x",
        "http://dev.tafac.fr:8443/x",
        "https://taf\tac.fr/x",
        "https:/__ent_proxy/x",
        "https:__ent_proxy/x",
        "https:\\\\tafac.fr/x",
        "https://evil.example\\@tafac.fr/",
        "https://",
    ] {
        let html = clean(&format!("<a href=\"{href}\">x</a>"));
        assert_eq!(
            html, "<a target=\"_blank\" rel=\"noopener noreferrer\">x</a>",
            "{href:?}"
        );
    }
    let html = clean(
        "<a href=\"https://tafac.fr.evil.example/\">x</a><a href=\"https://tafac.fr@evil.example/\">y</a>",
    );
    assert_eq!(html.matches("href=").count(), 2, "{html}");
    let text = mail_text_to_html(
        "voir https://tafac.fr/__ent_auth/logout et https://x.fr",
        &["tafac.fr"],
    );
    assert_eq!(
        text,
        "<div class=\"tafac-mail-text\">voir https://tafac.fr/__ent_auth/logout et <a href=\"https://x.fr\" target=\"_blank\" rel=\"noopener noreferrer\">https://x.fr</a></div>"
    );
}

#[test]
fn porteurs_entrants_et_fin_de_style() {
    let html = clean(
        "<div data-remote-style=\"x\" data-remote-background=\"y\" data-remote-src=\"z\">a</div><img src=\"data:image/png;base64,AA\" data-remote-src=\"https://x\"><style data-remote-css=\"q\">p{}</style>",
    );
    assert_eq!(
        html,
        "<div>a</div><img src=\"data:image/png;base64,AA\" /><style>p{}</style>"
    );
    // </style> dans le CSS (un <style> dans un autre, lu en SVG) : neutralisé, une seule fin.
    let html = clean("<svg><style><style>x</style><img src=x onerror=alert(1)></style></svg>");
    assert_eq!(html.matches("</style").count(), 1, "{html}");
    assert!(html.ends_with("</style>"), "{html}");
}

#[test]
fn image_set_et_url_sans_guillemets() {
    let result = sanitize_mail_html(
        "<p style=\"background:image-set(url(https://a.example/x.png) 1x);list-style:url(https://b.example/y.png)\">x</p><style>p{background:url( https://c.example/z.png )}</style>",
        &[],
        &mut resolve_logo,
    );
    assert_eq!(result.remote_images, 2);
    assert!(!result.html.contains("a.example"));
    assert!(result.html.contains(
        "data-remote-style=\"background:none;list-style:url(&quot;https://b.example/y.png&quot;)\""
    ));
    assert!(
        result
            .html
            .contains("data-remote-css=\"p{background:url(&quot;https://c.example/z.png&quot;)}\"")
    );
    // Un échappement suivi de \r\n compte pour une espace, comme dans le navigateur.
    let css = rewrite_css_urls("a{background:\\75\r\nrl(/__ent_proxy/x)}", &mut |_| None);
    assert_eq!(css.held, "a{background:none}");
}

#[test]
fn cid_vers_un_svg() {
    // Le cid d'une image SVG ne se résout pas (buildMailMessage n'offre que les formats
    // matriciels) : l'image part.
    let mut resolve =
        |cid: &str| (cid == "photo@x").then(|| "/__ent_auth/mail/part?id=1&part=3".to_owned());
    let html = sanitize_mail_html(
        "<img src=\"cid:logo.svg@x\"><img src=\"cid:photo@x\">",
        &[],
        &mut resolve,
    )
    .html;
    assert_eq!(
        html,
        "<img src=\"/__ent_auth/mail/part?id=1&amp;part=3\" />"
    );
}

#[test]
fn bords_vides_d_une_signature() {
    // W2b : <p> + 100 000 espaces + x</p>.
    let html = format!("<p>{}x</p>", " ".repeat(99_000));
    let started = Instant::now();
    let clean = sanitize_signature_html(&html);
    assert!(clean.html.ends_with("x</p>"));
    let blank = format!(
        "<div>{}</div>{}",
        "&nbsp; <br>".repeat(20_000),
        " ".repeat(50_000)
    );
    assert_eq!(sanitize_signature_html(&blank).html, "");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn texte_alternatif_decode_une_fois() {
    // W15 : `&amp;lt;` reste `&lt;` (Node le décodait deux fois).
    let html = sanitize_composer_html(
        "<img data-inline-cid=\"tafac-img-0123abcd-4567@tafac.fr\" alt=\"a &amp;lt; b\">",
    );
    assert_eq!(
        html,
        "<img data-inline-cid=\"tafac-img-0123abcd-4567@tafac.fr\" alt=\"a &amp;lt; b\" />"
    );
}

#[test]
fn signature_sans_motif_de_remplacement() {
    let clean = sanitize_composer_html("<p>a</p><div data-mail-signature></div>");
    let html = build_mail_html(&clean, "<div>$&amp; $1</div>", false, &[]);
    assert!(html.contains("<div>$&amp; $1</div>"), "{html}");
}

#[test]
fn brouillon_et_images() {
    let cids = vec!["tafac-img-0123abcd-4567-89ab@tafac.fr".to_owned()];
    let read = read_draft_images(
        "<div><p>x</p><img src=\"cid:tafac-img-0123abcd-4567-89ab@tafac.fr\" width=\"520\" height=\"300\" alt=\"Note\"><img src=\"cid:tafac-img-fedcba98-7654-3210@tafac.fr\"><img src=\"https://x.example/y.png\"></div>",
        &cids,
    );
    assert_eq!(read.used, cids);
    assert_eq!(read.dropped, 1);
    assert_eq!(
        sanitize_composer_html(&read.html),
        "<p>x</p><img data-inline-cid=\"tafac-img-0123abcd-4567-89ab@tafac.fr\" alt=\"Note\" width=\"520\" height=\"300\" />"
    );
    assert_eq!(
        plain_text_to_composer_html("ligne <b>\r\n\r\nfin"),
        "<p>ligne &lt;b&gt;</p><p></p><p>fin</p>"
    );
}
