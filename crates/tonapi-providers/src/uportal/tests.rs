#[test]
fn lien_partageable_entre_etudiants() {
    assert!(shareable(
        "https://app.exemple.test/ent/html/edt.html",
        "e2100"
    ));
    assert!(shareable(
        "https://app.exemple.test/ent/Apoweb?v=1",
        "e2100"
    ));
    assert!(!shareable(
        "https://app.exemple.test/ent/dossier?numetu=21901234",
        ""
    ));
    assert!(!shareable(
        "https://ade.exemple.test/direct_cal.jsp?data=8241fc",
        ""
    ));
    assert!(!shareable(
        "https://app.exemple.test/x?ticket=ST-1",
        "e2100"
    ));
    assert!(!shareable("https://app.exemple.test/u/E2100/page", "e2100"));
    assert!(!shareable("https://app.exemple.test/?jsessionid=abc", ""));
}

/// Un jeton ou un numéro dans le chemin passait le filtre, et le lien d'un
/// étudiant pouvait être servi aux autres par le cache partagé.
#[test]
fn jeton_ou_numero_dans_le_chemin_jamais_partage() {
    for link in [
        "https://ade.exemple.test/cal/8241fc0e9a7b/edt.ics",
        "https://app.exemple.test/dossier/21901234",
        "https://app.exemple.test/d/3f2504e0-4f89-11d3-9a0c-0305e82c3301",
        "https://app.exemple.test/s/Ab3dEf6hIj9kLm2nOp5q",
        "https://app.exemple.test/f/21901234.pdf",
        "https://moi:secret@app.exemple.test/",
    ] {
        assert!(!shareable(link, "e2100"), "{link}");
    }
    assert!(shareable(
        "https://app.exemple.test/ent/v2/index.html",
        "e2100"
    ));
}

#[test]
fn lien_du_fragment_de_portlet() {
    let base = Url::parse("https://ent.exemple.test/uPortal/").unwrap();
    let iframe = "\n<iframe src=\"https://app.exemple.test/ent/html/edt.html\" frameBorder=\"0\"\n name=\"x\">\n<a href=\"https://autre.exemple.test/\">ici</a></iframe>";
    assert_eq!(
        fragment_link(iframe, &base).map(String::from).as_deref(),
        Some("https://app.exemple.test/ent/html/edt.html")
    );
    let anchor = "<div><A HREF='https://scolarite.exemple.test/?a=1&amp;b=2'>Ouvrir</A></div>";
    assert_eq!(
        fragment_link(anchor, &base).map(String::from).as_deref(),
        Some("https://scolarite.exemple.test/?a=1&b=2")
    );
    // Fragment d'erreur : seul un lien vers le portail, donc rien.
    let error =
        "<span><a href=\"/uPortal/p/error.ctf2/normal/action.uP?x=1\">Réinitialiser</a></span>";
    assert_eq!(fragment_link(error, &base), None);
    assert_eq!(
        fragment_link("<iframe data-src=\"https://x.test/\">", &base),
        None
    );
    // Lanceur d'uPortal 5 (forme vue à Rennes) : les attributs commencent à la ligne suivante,
    // et le style qui précède ne nomme que des adresses du portail.
    let launcher = "<style>\n  #Pluto_1_app .portlet-icon { background-image: url('/media/icons/x.png'); }\n</style>\n\n<a\n    id=\"Pluto_1_app\"\n    class=\"portlet-link\"\n    href=\"https://planning.exemple.test/portal/\"\n    rel=\"noopener noreferrer\"\n    target=\"_blank\">\n  <span class=\"portlet-title\">Mon emploi du temps</span>\n</a>";
    assert_eq!(
        fragment_link(launcher, &base).map(String::from).as_deref(),
        Some("https://planning.exemple.test/portal/")
    );
    // Une balise qui commence comme `<a` n'en est pas une.
    assert_eq!(
        fragment_link(
            "<abbr href=\"https://x.test/\">x</abbr><article href=\"https://y.test/\">",
            &base
        ),
        None
    );
}

use super::parse::*;
use super::*;

const LAYOUT_JSONP: &str = include_str!("../../tests/fixtures/uportal/layout_prolongation.jsonp");
const LAYOUT_V5: &str = include_str!("../../tests/fixtures/uportal/layout_v5.json");
const LAYOUT_V5_GUEST: &str = include_str!("../../tests/fixtures/uportal/layout_v5_invite.json");
const SESSION_V4: &str = include_str!("../../tests/fixtures/uportal/session_v4.json");
const SESSION_V4_GUEST: &str = include_str!("../../tests/fixtures/uportal/session_v4_invite.json");
const PEOPLE_V4: &str = include_str!("../../tests/fixtures/uportal/people_v4.json");
const USERINFO: &str = include_str!("../../tests/fixtures/uportal/userinfo_v5.jwt");
const USERINFO_GUEST: &str = include_str!("../../tests/fixtures/uportal/userinfo_v5_invite.jwt");

fn base4() -> Url {
    Url::parse("https://ent.exemple.test/uPortal/").unwrap()
}

fn base5() -> Url {
    Url::parse("https://services.exemple.test/").unwrap()
}

fn app<'a>(apps: &'a [App], id: &str) -> &'a App {
    apps.iter()
        .find(|a| a.id == id)
        .unwrap_or_else(|| panic!("application `{id}` absente"))
}

fn ids(apps: &[App]) -> Vec<&str> {
    apps.iter().map(|a| a.id.as_str()).collect()
}

fn code(result: Result<impl std::fmt::Debug, Problem>) -> ErrorCode {
    result.unwrap_err().code
}

// layout.jsp

#[test]
fn layout_jsp_liste_sans_doublon_dans_l_ordre() {
    let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
    assert_eq!(
        ids(&apps),
        [
            "compte-demo",
            "moodle-demo",
            "bibliotheque",
            "planning-demo",
            "sans-titre"
        ]
    );
}

#[test]
fn layout_jsp_icon_url_donne_l_adresse_de_l_application() {
    let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
    let compte = app(&apps, "compte-demo");
    assert_eq!(compte.title, "Compte informatique");
    assert_eq!(
        compte.url,
        "https://apps.exemple.test/compte/protected/authenticated"
    );
    assert_eq!(compte.icon_url, None);
    assert_eq!(
        compte.description.as_deref(),
        Some("Mot de passe, redirection du courrier")
    );
    assert_eq!(compte.category.as_deref(), Some("Accueil"));
}

#[test]
fn layout_jsp_resout_les_urls_relatives_et_garde_l_icone() {
    let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
    let biblio = app(&apps, "bibliotheque");
    // `iconUrl` est une image : l'adresse de l'application est la page uPortal du portlet.
    assert_eq!(
        biblio.url,
        "https://ent.exemple.test/uPortal/f/u1l1s1/p/bibliotheque.u1l1n3/detached/render.uP?pCm=view"
    );
    assert_eq!(
        biblio.icon_url.as_deref(),
        Some("https://ent.exemple.test/uPortal/media/skins/icons/mobile/bibliotheque.png")
    );
    // `iconUrl` vide : repli sur la page uPortal.
    let planning = app(&apps, "planning-demo");
    assert!(
        planning
            .url
            .starts_with("https://ent.exemple.test/uPortal/f/u1l1s2/p/planning-demo.")
    );
    assert_eq!(planning.icon_url, None);
}

#[test]
fn layout_jsp_champs_facultatifs_absents() {
    let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
    let moodle = app(&apps, "moodle-demo");
    // Description vide, dossier « Accueil » (premier) conservé.
    assert_eq!(moodle.description, None);
    assert_eq!(moodle.category.as_deref(), Some("Accueil"));
    // Sans titre : le `fname` en tient lieu.
    assert_eq!(app(&apps, "sans-titre").title, "sans-titre");
}

#[test]
fn layout_jsp_ignore_une_entree_sans_fname() {
    let apps = parse_prolongation_layout(LAYOUT_JSONP, &base4()).unwrap();
    assert!(apps.iter().all(|a| a.title != "Entrée sans identifiant"));
}

#[test]
fn layout_jsp_racine_sans_barre_finale() {
    let base = root(&Url::parse("https://ent.exemple.test/uPortal").unwrap());
    let apps = parse_prolongation_layout(LAYOUT_JSONP, &base).unwrap();
    assert!(
        app(&apps, "bibliotheque")
            .url
            .starts_with("https://ent.exemple.test/uPortal/f/")
    );
}

#[test]
fn layout_jsp_json_nu_et_jsonp_avec_point_virgule() {
    let nu = r#"{"user":"e00000000000","layout":{"folders":[{"title":"A","portlets":[{"fname":"x","title":"X","url":"/uPortal/p/x"}]}]}}"#;
    assert_eq!(parse_prolongation_layout(nu, &base4()).unwrap().len(), 1);
    let jsonp = format!("cb({nu});\n");
    let apps = parse_prolongation_layout(&jsonp, &base4()).unwrap();
    assert_eq!(apps[0].url, "https://ent.exemple.test/uPortal/p/x");
}

#[test]
fn layout_jsp_sans_utilisateur_ou_invite_est_une_session_perdue() {
    let sans_user = r#"cb({"layout":{"folders":[]}})"#;
    assert_eq!(
        code(parse_prolongation_layout(sans_user, &base4())),
        ErrorCode::SessionExpired
    );
    let invite = r#"cb({"user":"guest","layout":{"folders":[]}})"#;
    assert_eq!(
        code(parse_prolongation_layout(invite, &base4())),
        ErrorCode::SessionExpired
    );
}

#[test]
fn layout_jsp_format_inattendu_sans_donnee_dans_l_erreur() {
    let sans_dossiers = r#"cb({"user":"e00000000000","layout":{}})"#;
    let err = parse_prolongation_layout(sans_dossiers, &base4()).unwrap_err();
    assert_eq!(err.code, ErrorCode::UpstreamChanged);
    assert!(!err.to_string().contains("e00000000000"));

    let html = "<html>e00000000000</html>";
    let err = parse_prolongation_layout(html, &base4()).unwrap_err();
    assert_eq!(err.code, ErrorCode::UpstreamChanged);
    assert!(!err.to_string().contains("e00000000000"));
}

#[test]
fn layout_jsp_dossier_vide_donne_une_liste_vide() {
    let vide = r#"cb({"user":"e00000000000","layout":{"folders":[{"title":"A"}]}})"#;
    assert!(
        parse_prolongation_layout(vide, &base4())
            .unwrap()
            .is_empty()
    );
}

// layout.json (uPortal 5)

#[test]
fn layout_v5_liste_sans_doublon_favoris_en_dernier() {
    let apps = parse_uportal5_layout(LAYOUT_V5, &base5()).unwrap();
    assert_eq!(
        ids(&apps),
        [
            "webapp-planning",
            "webapp-courrier",
            "webapp-dossier",
            "webapp-assistance"
        ]
    );
}

#[test]
fn layout_v5_liens_et_champs() {
    let apps = parse_uportal5_layout(LAYOUT_V5, &base5()).unwrap();
    let planning = app(&apps, "webapp-planning");
    assert_eq!(planning.title, "Mon emploi du temps");
    assert_eq!(planning.url, "https://planning.exemple.test/portal/");
    assert_eq!(
        planning.icon_url.as_deref(),
        Some("https://planning.exemple.test/icone.png")
    );
    assert_eq!(planning.category.as_deref(), Some("Scolarité"));

    // `parameters.url` relatif : résolu sur le portail.
    assert_eq!(
        app(&apps, "webapp-dossier").url,
        "https://services.exemple.test/mdw/"
    );

    // Aucun paramètre : page du portail pour ce portlet.
    let assistance = app(&apps, "webapp-assistance");
    assert_eq!(
        assistance.url,
        "https://services.exemple.test/p/webapp-assistance"
    );
    // Le dossier technique « Column » n'est pas une catégorie : l'onglet la remplace.
    assert_eq!(assistance.category.as_deref(), Some("Aide"));
    assert_eq!(assistance.description, None);
    assert_eq!(assistance.icon_url, None);
}

#[test]
fn layout_v5_un_doublon_complete_le_premier() {
    let apps = parse_uportal5_layout(LAYOUT_V5, &base5()).unwrap();
    let courrier = app(&apps, "webapp-courrier");
    // Vu d'abord dans « Scolarité » (sans description ni icône), puis dans « Favoris ».
    assert_eq!(courrier.category.as_deref(), Some("Scolarité"));
    assert_eq!(courrier.description.as_deref(), Some("Boîte aux lettres"));
    assert_eq!(
        courrier.icon_url.as_deref(),
        Some("https://services.exemple.test/images/icons/courrier.svg")
    );
    assert_eq!(courrier.url, "https://courrier.exemple.test/");
}

#[test]
fn layout_v5_ignore_les_portlets_masques_et_les_regions() {
    let apps = parse_uportal5_layout(LAYOUT_V5, &base5()).unwrap();
    assert!(
        apps.iter()
            .all(|a| a.id != "webapp-cache" && a.id != "portal-brand")
    );
}

#[test]
fn layout_v5_favori_seul_garde_l_onglet_comme_categorie() {
    let body = r#"{"user":"e1","authenticated":"true","layout":{"navigation":{"tabs":[
            {"name":"Services","content":[{"_objectType":"folder","name":"Favoris","content":[
              {"_objectType":"portlet","fname":"seul","title":"Seul"}]}]}]}}}"#;
    let apps = parse_uportal5_layout(body, &base5()).unwrap();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].category.as_deref(), Some("Services"));
}

#[test]
fn layout_v5_invite_est_une_session_perdue() {
    assert_eq!(
        code(parse_uportal5_layout(LAYOUT_V5_GUEST, &base5())),
        ErrorCode::SessionExpired
    );
    let invite_sans_drapeau =
        r#"{"user":"guest","authenticated":true,"layout":{"navigation":{"tabs":[]}}}"#;
    assert_eq!(
        code(parse_uportal5_layout(invite_sans_drapeau, &base5())),
        ErrorCode::SessionExpired
    );
}

#[test]
fn layout_v5_accepte_un_vrai_booleen() {
    let body = r#"{"user":"e1","authenticated":true,"layout":{"navigation":{"tabs":[]}}}"#;
    assert!(parse_uportal5_layout(body, &base5()).unwrap().is_empty());
}

#[test]
fn layout_v5_champs_obligatoires_absents() {
    let sans_drapeau = r#"{"user":"e1","layout":{"navigation":{"tabs":[]}}}"#;
    assert_eq!(
        code(parse_uportal5_layout(sans_drapeau, &base5())),
        ErrorCode::UpstreamChanged
    );
    let sans_onglets = r#"{"user":"e1","authenticated":"true","layout":{}}"#;
    assert_eq!(
        code(parse_uportal5_layout(sans_onglets, &base5())),
        ErrorCode::UpstreamChanged
    );
    assert_eq!(
        code(parse_uportal5_layout("<html>", &base5())),
        ErrorCode::UpstreamChanged
    );
}

// profil uPortal 4

#[test]
fn profil_v4_complet() {
    let session = parse_session(SESSION_V4).unwrap();
    let profile = parse_profile_v4(&session, Some(PEOPLE_V4));
    assert_eq!(
        profile,
        Profile {
            username: "e00000000000".into(),
            display_name: Some("Camille Demo".into()),
            first_name: Some("Camille".into()),
            last_name: Some("Demo".into()),
            email: Some("camille.demo@etu.exemple.test".into()),
            student_number: None,
            program: None,
        }
    );
}

#[test]
fn profil_v4_sans_people_ou_person_nul_reste_valide() {
    let session = parse_session(SESSION_V4).unwrap();
    for people in [None, Some(r#"{"person":null}"#), Some("pas du json")] {
        let profile = parse_profile_v4(&session, people);
        assert_eq!(profile.username, "e00000000000");
        assert_eq!(profile.display_name.as_deref(), Some("Camille Demo"));
        assert_eq!(profile.first_name, None);
        assert_eq!(profile.last_name, None);
        assert_eq!(profile.email, None);
        assert_eq!(profile.student_number, None);
    }
}

#[test]
fn profil_v4_nom_affiche_deduit_du_prenom_et_du_nom() {
    let session = V4Session {
        username: "e00000000000".into(),
        display_name: None,
    };
    let people = r#"{"person":{"attributes":{"givenName":["Camille"],"sn":["Demo"],"mail":[],"supannEtuId":["00000000"]}}}"#;
    let profile = parse_profile_v4(&session, Some(people));
    assert_eq!(profile.display_name.as_deref(), Some("Camille Demo"));
    assert_eq!(profile.email, None);
    assert_eq!(profile.student_number.as_deref(), Some("00000000"));
}

#[test]
fn session_v4_invitee_ou_incomplete() {
    assert_eq!(
        code(parse_session(SESSION_V4_GUEST)),
        ErrorCode::SessionExpired
    );
    assert_eq!(
        code(parse_session(r#"{"person":{"displayName":"X"}}"#)),
        ErrorCode::UpstreamChanged
    );
    assert_eq!(code(parse_session("{}")), ErrorCode::UpstreamChanged);
    assert_eq!(code(parse_session("<html>")), ErrorCode::UpstreamChanged);
}

#[test]
fn url_people_encode_l_identifiant() {
    let url = people_url(&base4(), "e00000000000").unwrap();
    assert_eq!(
        url.as_str(),
        "https://ent.exemple.test/uPortal/api/people/e00000000000.json"
    );
    let url = people_url(&base4(), "a b/../c").unwrap();
    assert_eq!(url.host_str(), Some("ent.exemple.test"));
    assert!(url.path().starts_with("/uPortal/api/people/"));
    assert!(!url.path().contains("/../"));
}

// profil uPortal 5

#[test]
fn userinfo_jwt_complet() {
    let profile = parse_userinfo(USERINFO, "autre-compte").unwrap();
    assert_eq!(
        profile,
        Profile {
            username: "e00000000000".into(),
            display_name: Some("Camille Demo".into()),
            first_name: Some("Camille".into()),
            last_name: Some("Demo".into()),
            email: Some("camille.demo@etu.exemple.test".into()),
            student_number: Some("00000000".into()),
            program: None,
        }
    );
}

#[test]
fn userinfo_json_nu_avec_champs_manquants() {
    let body = r#"{"sub":"e00000000000"}"#;
    let profile = parse_userinfo(body, "autre-compte").unwrap();
    assert_eq!(profile.username, "e00000000000");
    assert_eq!(profile.display_name, None);
    assert_eq!(profile.email, None);
    assert_eq!(profile.student_number, None);
}

#[test]
fn userinfo_sans_identifiant_utilise_le_compte_de_connexion() {
    let body = r#"{"name":"Camille Demo","mail":"camille.demo@etu.exemple.test"}"#;
    let profile = parse_userinfo(body, "e00000000000").unwrap();
    assert_eq!(profile.username, "e00000000000");
    assert_eq!(
        profile.email.as_deref(),
        Some("camille.demo@etu.exemple.test")
    );
    assert_eq!(code(parse_userinfo("{}", " ")), ErrorCode::UpstreamChanged);
}

#[test]
fn userinfo_invite_est_une_session_perdue() {
    assert_eq!(
        code(parse_userinfo(USERINFO_GUEST, "e00000000000")),
        ErrorCode::SessionExpired
    );
}

#[test]
fn userinfo_illisible() {
    // Vide, ni JWT ni JSON, base64 invalide, charge utile qui n'est pas un objet.
    for body in ["", "pas un jwt", "a.!!!.c", "[]", "a.W10.c"] {
        assert_eq!(
            code(parse_userinfo(body, "e00000000000")),
            ErrorCode::UpstreamChanged,
            "{body}"
        );
    }
}

#[test]
fn userinfo_base64_avec_remplissage() {
    // `{"sub":"x"}` encodé avec `=` final : accepté.
    let profile = parse_userinfo("a.eyJzdWIiOiJ4In0=.c", "compte").unwrap();
    assert_eq!(profile.username, "x");
}

// aides

#[test]
fn statuts_http() {
    assert!(check_status(StatusCode::OK).is_ok());
    for status in [
        StatusCode::FOUND,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
    ] {
        assert_eq!(code(check_status(status)), ErrorCode::SessionExpired);
    }
    assert_eq!(
        code(check_status(StatusCode::BAD_GATEWAY)),
        ErrorCode::ServiceUnavailable
    );
    assert_eq!(
        code(check_status(StatusCode::NOT_FOUND)),
        ErrorCode::UpstreamChanged
    );
}

#[test]
fn jsonp_et_adresses() {
    assert_eq!(strip_jsonp("cb({\"a\":(1)});"), "{\"a\":(1)}");
    assert_eq!(strip_jsonp("{\"a\":\"(\"}"), "{\"a\":\"(\"}");
    assert_eq!(strip_jsonp("sans parenthèses"), "sans parenthèses");
    assert!(resolve(&base4(), "javascript:alert(1)").is_none());
    assert!(resolve(&base4(), "   ").is_none());
    assert_eq!(
        resolve(&base4(), "/mdw/").unwrap().as_str(),
        "https://ent.exemple.test/mdw/"
    );
    let page = portal_page(&base5(), "a b").unwrap();
    assert_eq!(page.as_str(), "https://services.exemple.test/p/a%20b");
}
