# Esup-Pod — `video.umontpellier.fr`

> Sources : code source Esup-Pod, branche `main` du dépôt `EsupPortail/Esup-Pod` (chemins cités à chaque section : `pod/urls.py`, `pod/main/rest_router.py`, `pod/video/rest_views.py`, `pod/video/feeds.py`, `pod/video/urls*.py`, `pod/video/views.py`, `pod/video/utils.py`, `pod/video/models.py`, `pod/video/context_processors.py`, `pod/video_search/views.py`, `pod/authentication/rest_views.py`, `pod/authentication/views.py`, `pod/settings.py`, `pod/main/rest_pagination.py`) ; deux GET publics sur `video.umontpellier.fr` le 13 oct. 2026 (`/rss-video/` et `/videos/`) ; observation antérieure de `/rest/` (401, [pistes-api.md §7](../research/pistes-api.md)).
> Aucune donnée personnelle reproduite (les flux contiennent nom et e-mail des déposants : masqués ici, à ne pas stocker côté tonAPI).
>
> **Légende :** **[confirmé]** = observé sur `video.umontpellier.fr`. **[code]** = lu dans la branche `main` ; la version déployée est **4.4.0 [confirmé]** (pied de page), proche de `main`. **[incertain]** = hypothèse.

> Voir aussi : [sympa-soap.md](sympa-soap.md), [cas-umontpellier.md](../cas-umontpellier.md), [research/pistes-api.md §7](../research/pistes-api.md).

## En bref

- **Plateforme** : « Plateforme vidéo de l'université de Montpellier (Pod) », Esup-Pod **4.4.0**, **16 994 vidéos** listées publiquement (compteur du titre `/videos/`) **[confirmé]**.
- **API REST `/rest/`** : Django REST Framework, auth **Basic ou Token**, permission par défaut **`IsAdminUser`** (compte *staff*) **[code]** → un étudiant, même avec un jeton, obtient **403**. Anonyme : **401 `WWW-Authenticate: Basic realm="api"`** **[confirmé]**. Inexploitable pour tonAPI.
- **Surfaces publiques sans auth** : `/videos/` (HTML + fragment AJAX, filtres `title`, `type`, `discipline`, `tag`, `cursus`, `sort`), `/video/<id>-<slug>/`, chaînes `/<chaîne>/[<thème>/]`, **`/rss-video/`** et **`/rss-audio/`** (RSS 2.0 + iTunes, 100 derniers items, filtrables par chaîne/thème), `/search/?q=` (Elasticsearch), `/info_pod.json`, JSON des chaînes/thèmes.
- **Vidéos restreintes** : `is_restricted` → login CAS (redirection `/authentication_login/?referrer=…`) ; `restrict_access_to_groups` → groupe d'accès ; `password` → formulaire POST ; `is_draft` → propriétaire ou lien privé `/video/<slug>/<hashkey>/`. Dans les flux RSS, ces vidéos sont listées **sans `<enclosure>`** (4 enclosures sur 100 items observés).

---

## 1. Architecture et routes **[code]**

`pod/urls.py` :

| Préfixe | Include / vue | Rôle |
|---|---|---|
| `videos/` | `pod.video.urls-videos` | liste publique, tableau de bord (connecté) |
| `rss` | `pod.video.urls-rss` | `rss-video/`, `rss-audio/` (+ chaîne/thème) |
| `video/` | `pod.video.urls` | page vidéo, édition, commentaires, oEmbed, JSON chaînes |
| `channels/` | `pod.video.urls-channels` | `my/`, édition (connecté) |
| `` (racine) | `pod.video.urls-channels-video` | `/<slug_c>/`, `/<slug_c>/<slug_t>/`, `/<slug_c>/video/<slug>/` |
| `search/` | `pod.video_search.urls` | recherche Elasticsearch |
| `rest/` | `pod.main.rest_router` | **API DRF** |
| `api-auth/` | `rest_framework.urls` | login/logout de l'interface navigable DRF (compte Django local) |
| `authentication_` | `pod.authentication.urls` | `authentication_login/`, `authentication_logout/` |
| `accounts/login` (sans `/`) | `django_cas_ng` `LoginView` (si `USE_CAS`) | **login CAS** |
| `accounts/login/` | `django.contrib.auth` `LoginView` | login local |
| `info_pod.json` | `pod.main.views.info_pod` | méta-infos instance |
| `robots.txt`, `contact_us/`, `download/`, `captcha/`, `select2/`, `tinymce/`, `jsi18n/`, `admin/` | divers | — |
| `playlist/`, `meeting/`, `xapi/`, `studio/`, `podfile/`, `cut/`, `quiz/`, `dressing/`, `speaker/`, `import_video/`, `duplicate/`, `hyperlinks/`, `ai-enhancement/`, `runner/`, `webpush/`, `oidc/` | modules optionnels (`USE_*`) | présence à l'UM **[incertain]** |

---

## 2. API REST `/rest/` **[code]**

### 2.1 Authentification et permissions

`pod/settings.py` :

```python
REST_FRAMEWORK = {
    "DEFAULT_FILTER_BACKENDS": ("django_filters.rest_framework.DjangoFilterBackend",),
    "DEFAULT_AUTHENTICATION_CLASSES": (
        "rest_framework.authentication.BasicAuthentication",
        "rest_framework.authentication.TokenAuthentication",
    ),
    "DEFAULT_PERMISSION_CLASSES": ("rest_framework.permissions.IsAdminUser",),
    "DEFAULT_PAGINATION_CLASS": "pod.main.rest_pagination.CustomPagination",
    "PAGE_SIZE": 12,
}
```

- **Anonyme** → 401 avec le challenge de la **première** classe d'auth : `WWW-Authenticate: Basic realm="api"` (c'est ce qui a été observé) **[confirmé]**.
- **Basic** : identifiant + mot de passe d'un compte **Django local**. Les comptes créés par CAS (`django_cas_ng`) n'ont pas de mot de passe utilisable → Basic ne sert qu'aux comptes locaux/admin.
- **Token** : en-tête `Authorization: Token <clé>`. Les jetons viennent de `rest_framework.authtoken` (`INSTALLED_APPS`) : ils sont **créés par un administrateur** dans l'admin Django (`/admin/authtoken/tokenproxy/`, « Jetons ») ou en ligne de commande (`manage.py drf_create_token <user>`). **Aucune route `obtain_auth_token` n'est câblée** dans `pod/urls.py` → un utilisateur ne peut pas générer son jeton lui-même. À l'UM, la mention observée « jeton API personnel, réservé aux comptes autorisés » va dans ce sens **[incertain]**.
- **Permission `IsAdminUser`** : `request.user.is_staff` requis sur **tous** les ViewSets (aucun ne redéfinit `permission_classes`). Un étudiant authentifié → **403**. `DublinCoreView` (APIView) hérite de la même permission (l'auth par token y est commentée mais la permission par défaut s'applique).
- CSRF : avec Basic/Token, DRF n'exige pas de jeton CSRF ; avec la session navigateur (`/api-auth/`), oui.
- Format : JSON par défaut ; `?format=json|api` ; interface navigable HTML si `Accept: text/html`.

### 2.2 Pagination et filtres **[code]**

`pod/main/rest_pagination.py` : `PageNumberPagination`, `page_size=12`, `?page_size=` jusqu'à **1000**, `?page=`. Réponse : `{"count", "next", "previous", "results": [...]}`.
Filtres : `django-filter` sur les champs `filterset_fields` de chaque ViewSet (`?owner=<id>&is_draft=false`…). Les sérialiseurs sont des `HyperlinkedModelSerializer` : les relations sont des **URL** (`https://video.umontpellier.fr/rest/users/12/`), pas des id.

### 2.3 Routeur (`pod/main/rest_router.py`)

Tous les `ViewSet` sont des **`ModelViewSet`** : `GET` liste / détail, `POST`, `PUT`, `PATCH`, `DELETE` (CRUD complet, réservé staff).

| Route | ViewSet (fichier) | `filterset_fields` | Actions supplémentaires |
|---|---|---|---|
| `/rest/users/` | `UserViewSet` (`pod/authentication/rest_views.py`) | `id`, `username`, `email` | — |
| `/rest/groups/` | `GroupViewSet` | — | — |
| `/rest/owners/` | `OwnerViewSet` | — | — |
| `/rest/sites/` | `SiteViewSet` | — | — |
| `/rest/accessgroups/` | `AccessGroupViewSet` | `id`, `display_name`, `code_name` | — |
| `/rest/channels/` | `ChannelViewSet` (`pod/video/rest_views.py`) | — | — |
| `/rest/themes/` | `ThemeViewSet` (tri par `channel`) | — | — |
| `/rest/types/` | `TypeViewSet` | — | — |
| `/rest/discipline/` | `DisciplineViewSet` (singulier !) | — | — |
| `/rest/videos/` | `VideoViewSet` | `owner`, `type`, `date_added`, `is_draft`, `is_restricted`, `encoding_in_progress`, `sites` | `GET /rest/videos/user_videos/?username=&encoded=true&search_title=&extensions=mp4` |
| `/rest/view_count/` | `ViewCountViewSet` | — | — |
| `/rest/renditions/` | `VideoRenditionViewSet` (`pod/video_encode_transcript/rest_views.py`) | — | — |
| `/rest/encodings_video/` | `EncodingVideoViewSet` | (voir fichier) | `video_encodedfiles/` |
| `/rest/encodings_audio/` | `EncodingAudioViewSet` | (voir fichier) | `audio_encodedfiles/` |
| `/rest/playlist_videos/` | `PlaylistVideoViewSet` (fichiers HLS) | — | — |
| `/rest/playlists/` | `PlaylistViewSet` (`pod/playlist/rest_views.py`) | `owner`, `visibility`, `autoplay`, `editable` | — |
| `/rest/contributors/`, `/rest/documents/`, `/rest/tracks/`, `/rest/overlays/` | `pod/completion/rest_views.py` | par `video` | — |
| `/rest/chapters/` | `ChapterViewSet` (`pod/chapter/rest_views.py`) | — | — |
| `/rest/recording/`, `/rest/recordingfile/`, `/rest/recordingfiletreatment/`, `/rest/recorder/` | `pod/recorder/rest_views.py` | — | — |
| `/rest/folders/`, `/rest/files/`, `/rest/images/` | `pod/podfile/rest_views.py` (si `USE_PODFILE`) | — | — |
| `/rest/meeting_session/`, `/rest/meeting_internal_recording/`, `/rest/meeting_livestream/`, `/rest/meeting_live_gateway/` | `pod/meeting/rest_views.py` (si `USE_MEETING`) | — | — |
| `/rest/dublincore/` | `DublinCoreView` (APIView, `text/xml`) | tous les champs `Video` via `?champ=valeur` | — |
| `/rest/launch_encode_view/`, `/rest/launch_transcript_view/` (GET) ; `/rest/store_remote_encoded_video*/`, `/rest/store_remote_transcripted_video/` (POST) | vues d'encodage distant | — | — |
| `/rest/accessgroups_set_users_by_name/`, `…_remove_users_by_name/`, `…_set_user_accessgroup/`, `…_remove_user_accessgroup/` (POST) | `pod/authentication/rest_views.py` | — | corps `{"code_name", "users": []}` / `{"username", "groups": []}` |

`GET /rest/` renvoie l'index DRF (`DefaultRouter`) : dictionnaire `nom → URL`.

### 2.4 Champs des sérialiseurs principaux (`pod/video/rest_views.py`, `pod/authentication/rest_views.py`)

**`videos` — `VideoSerializer`** :

| Champ | Type | Note |
|---|---|---|
| `id` | int | |
| `url` | URL | `/rest/videos/<id>/` |
| `video` | fichier | chemin du fichier source déposé |
| `title`, `slug`, `description` | str | `slug` = `<id>-<titre-slugifié>` |
| `allow_downloading`, `is_360`, `is_draft`, `is_restricted`, `disable_comment` | bool | |
| `owner` | URL user | `additional_owners` : liste d'URL |
| `date_added` | datetime | `date_evt` : date (événement) |
| `cursus` | code | `CURSUS_CODES` (ex. `0` tous publics, `L`, `M`, `D`…) **[incertain sur les valeurs]** |
| `main_lang` | code langue | `LANG_CHOICES` |
| `restrict_access_to_groups` | liste d'URL `accessgroups` | |
| `password` | str | **mot de passe de la vidéo exposé** (champ non `write_only`) |
| `tags` | liste de str | `tagulous` |
| `thumbnail` | URL image | |
| `type` | URL `types` | `discipline`, `channel`, `theme` : listes d'URL |
| `licence` | code | `LICENCE_CHOICES` (`by`, `by-nc`, `by-nc-sa`…) |
| `encoding_in_progress`, `duration` (s), `get_encoding_step`, `get_version`, `encoded`, `duration_in_time` (`HH:MM:SS`) | lecture seule | |
| `sites` | liste d'URL | multi-sites |

**`videos/user_videos/` — `VideoUserSerializer`** : `id`, `url`, `get_version`, `type`, `date_added`, `is_draft`, `is_restricted`, `encoding_in_progress`, `encoded` + **`video_data`** = JSON d'indexation (`Video.get_json_to_index`, §4.5) enrichi de `encoded`, `encoding_in_progress`, `get_encoding_step`, `get_thumbnail_admin`, `video_files` (sources mp4/m3u8/m4a filtrées par `?extensions=`).

**`channels` — `ChannelSerializer`** : `id`, `url`, `title`, `description`, `headband` (image), `color`, `style`, `owners`, `users`, `visible`, `themes`, `site`.
**`themes`** : `id`, `url`, `parentId`, `title`, `headband`, `description`, `channel`.
**`types`** : `id`, `url`, `title`, `description`, `icon`, `sites`.
**`discipline`** : `id`, `url`, `title`, `description`, `icon`, `site`.
**`view_count`** : `video`, `date`, `count`.
**`users`** : `id`, `url`, `username`, `first_name`, `is_staff`, `last_name`, `email`, `groups`.
**`owners`** : `id`, `url`, `user`, `auth_type`, `affiliation`, `commentaire`, `hashkey`, `userpicture`, `sites`.
**`accessgroups`** : `id`, `display_name`, `code_name`, `sites`, `users`.
**`playlists`** : `id`, `url`, `name`, `visibility`, `description`, `autoplay`, `editable`, `slug`, `date_created`, `date_updated`, `owner`, `additional_owners`, `videos[]` (chaque entrée `{"video": <VideoSerializer>}`).
**`chapters`** : `id`, `url`, `video`, `title`, `time_start`.
**`tracks`** : `id`, `url`, `video`, `kind`, `lang`, `src` ; **`documents`** : `id`, `url`, `video`, `document`.

### 2.5 Exemple (staff uniquement)

```bash
curl -sS -H 'Authorization: Token <clé>' \
  'https://video.umontpellier.fr/rest/videos/?is_draft=false&is_restricted=false&page_size=50&format=json'
```

Sans jeton staff : `401` (anonyme) ou `403 {"detail":"Vous n'avez pas la permission d'effectuer cette action."}` (étudiant).

---

## 3. Surfaces publiques (sans authentification)

Toutes les vues publiques partent du même sous-ensemble : **`get_available_videos()`** (`pod/video/context_processors.py`) = `encoding_in_progress=False`, `is_draft=False`, `sites=<site courant>`, **et** au moins un encodage disponible (mp4, playlist HLS `application/x-mpegURL`, ou audio). Les vidéos **restreintes, protégées par mot de passe ou par groupe sont incluses** dans ces listes (seule la lecture est bloquée, §5).

### 3.1 `/videos/` — liste **[confirmé] + [code]**

`pod/video/views.py` `videos()` + `get_filtered_videos_list()` :

| Paramètre GET | Effet |
|---|---|
| `title=` | `icontains` sur `title_fr` ou `title_en` |
| `type=` (répétable) | slug ou id de `Type` (ex. `cours`, `colloque-conference`, `podcast`, `travaux-etudiants`, `webinaire`… — 15 types vus dans la page) |
| `discipline=` (répétable) | slug ou id |
| `tag=` (répétable) | tous les tags (ET) |
| `cursus=` (répétable) | codes `CURSUS_CODES` |
| `owner=` (répétable) | `username` — **ignoré pour un anonyme** si `HIDE_USER_FILTER`/`owner_is_searchable` (à l'UM le filtre propriétaire n'est pas proposé à l'anonyme, `owner_filter` false) **[incertain]** |
| `sort=` | champ de tri (`date_added` défaut, `title`, `duration`, `view_count`…) ; `sort_direction=` (`asc` / vide = desc) |
| `page=` | 12 vidéos par page (`Paginator(videos_list, 12)`) |

- Réponse HTML complète ; avec **`X-Requested-With: XMLHttpRequest`** (`is_ajax`) → **fragment** `videos/video_list.html` : uniquement les cartes + le lien « page suivante » (`infinite-more-link`). C'est le moyen le plus léger de scraper la liste.
- Marqueurs utiles dans le HTML **[confirmé]** : conteneur `#videos_list[data-countvideos="16994"][data-nextpage="true"]` ; chaque carte `.infinite-item .pod-card--video` contient `.time` (`HH:MM:SS`), `a[href="/video/<id>-<slug>/"]`, `img.pod-thumbnail[src="/media/cache/…png"][alt=<titre>]`, `.video-title a[title]`, et des icônes d'état : `bi-lock` + « Ce contenu est en accès restreint. », `bi-key` + « Ce contenu est protégé par un mot de passe. », `bi-film` (vidéo) / `bi-soundwave` (audio) **[incertain pour l'audio]**.
- Le titre de page contient le total : `… - 16994 vidéos trouvées`.

### 3.2 `/video/<id>-<slug>/` — page vidéo **[code]**

`pod/video/urls.py` : `^(?P<slug>[\-\d\w]+)/$` → `video()` ; `^(?P<slug>…)/(?P<slug_private>…)/$` → lien privé.

- L'`id` est lu **avant le premier `-`** du slug ; le reste du slug n'est pas vérifié (`/video/26297-nimporte-quoi/` fonctionne) **[code]**.
- `?is_iframe=true` → gabarit `videos/video-iframe.html` (lecteur seul). `?playlist=<slug>` → lecture dans une playlist. `?redirect=false` évite la redirection vers une « version » (enrichie, interactive) par défaut.
- Variantes : `/<slug_c>/video/<slug>/` et `/<slug_c>/<slug_t>/video/<slug>/` (contexte chaîne/thème) ; `/video/<slug>/<hashkey>/` = **lien privé** (`get_hashkey()` = sha256 de `SECRET_KEY-<id>`), qui ouvre aussi les brouillons et les vidéos restreintes.
- La page contient les métadonnées **Dublin Core** en `<meta name="dc.…">` (`videos/dublincore.html`, `get_dublin_core()` : `dc.title`, `dc.creator`, `dc.description`, `dc.subject` (type + disciplines), `dc.publisher`, `dc.contributor`, `dc.date` (`YYYY/MM/DD`), `dc.type`, `dc.identifier`, `dc.language`, `dc.coverage`, `dc.rights`, `dc.format`) — exploitable pour extraire proprement titre/auteur/date sans parser le reste. Les sources du lecteur (`mp4` par hauteur, `m3u8`, `m4a`, pistes, chapitres) sont injectées dans la page par `get_json_to_video_view()` **[incertain sur l'emplacement exact dans le HTML]**.
- Compteur de vues : `POST /video/count/<id>/` (appelé par le lecteur). Statistiques : `/video/stats_view/<slug>/` si `USE_STATS_VIEW`.

### 3.3 Chaînes et thèmes **[code]**

| URL | Vue | Contenu |
|---|---|---|
| `/<slug_c>/` | `channel()` | vidéos disponibles de la chaîne, triées `date_added` desc, pagination `?page=` ; fragment via `X-Requested-With` |
| `/<slug_c>/<slug_t>/` | `channel()` | thème + ses sous-thèmes (`get_all_children_flat`) |
| `/video/get-channel-tabs/` | JSON | onglets de chaînes de la barre de navigation (`channel_tabs_json_format`) |
| `/video/get-channels-for-specific-channel-tab/?id=<tab>&page=` | JSON | `{"channels": [...], "currentPage", "totalPages"}` (chaînes **visibles** du site, tri `title`) |
| `/video/get-themes-for-specific-channel/<slug_c>/` | JSON | arbre des thèmes (`Channel.get_all_theme_json()`) |

Les slugs de chaîne sont `slugify(title)` (`Channel.save`). Attention : la route racine `/<slug_c>/` capture tout slug inconnu → 404 pour une chaîne inexistante.

### 3.4 Flux RSS `/rss-video/` et `/rss-audio/` **[confirmé] + [code]**

`pod/video/urls-rss.py` (préfixe `rss` sans slash + `-video/`) :

| URL | Portée |
|---|---|
| `/rss-video/` | toutes les vidéos disponibles, **100 dernières** (`VIDEO_FEED_NB_ITEMS`, défaut 100 ; 100 items observés), tri `date_added` desc |
| `/rss-audio/` | mêmes items, enclosure = version **mp3** (`EncodingAudio name="audio", encoding_format="audio/mp3"`) |
| `/rss-video/<slug_c>/`, `/rss-audio/<slug_c>/` | une chaîne |
| `/rss-video/<slug_c>/<slug_t>/`, `/rss-audio/<slug_c>/<slug_t>/` | un thème (et sous-thèmes) |

Observé : `HTTP 200`, `Content-Type: application/rss+xml; charset=utf-8`, ~90 Ko pour 100 items. Aucun paramètre de pagination : pour remonter plus loin, utiliser `/videos/`.

**En-tête `<channel>`** (`RssSiteVideosFeed`, `RssFeedGenerator.add_root_elements`) :

| Élément | Valeur observée / source |
|---|---|
| `title` | « Plateforme vidéo de l'université de Montpellier (Pod) » (`TITLE_SITE`) ; pour une chaîne : `TITLE_SITE - <chaîne>` |
| `link` | **`https://`** (bug : `self.link` est le chemin vidé de `rss-video`, ici `/`, préfixé) **[confirmé]** ; pour une chaîne : `/<slug_c>/` |
| `description`, `itunes:summary` | « … plateforme vidéo de Université de Montpellier » |
| `atom:link rel="self"` | `https://video.umontpellier.fr/rss-video/` |
| `language` | `fr` ; `category` `Education` ; `itunes:category text="Education"` > `Higher Education` ; `itunes:explicit` `clean` |
| `copyright` | « Université de Montpellier - Town - Country BY-NC-SA » (`DEFAULT_DC_COVERAGE` non personnalisé + `DEFAULT_DC_RIGHTS`) |
| `lastBuildDate` | date du premier item |
| `image/url`, `itunes:image href` | miniature du premier item (ou bandeau de la chaîne) |
| `itunes:author`, `itunes:owner/itunes:name`, `itunes:owner/itunes:email` | nom de l'établissement et **e-mail de contact** (`CONTACT_US_EMAIL`) — ne pas stocker |
| `itunes:subtitle` | vide (racine) ou titre de la chaîne |

**Chaque `<item>`** (`add_item_elements` + méthodes `item_*`) :

| Élément | Source (`feeds.py`) | Remarque |
|---|---|---|
| `title` | `item_title` | titre nettoyé des caractères de contrôle ; souvent le **nom de fichier** brut (`….m4v`, `….mp4`) **[confirmé]** |
| `link`, `guid` | `item_link` | `https://video.umontpellier.fr/video/<id>-<slug>/` (guid = link, `isPermaLink` absent) |
| `description`, `itunes:summary` | `item_description` | description HTML + `<br/> Durée: HH:MM:SS` (libellé traduit `Duration`) ; souvent réduit à la durée |
| `author` | `item_author_email` + `item_author_name` | **`email (Prénom Nom)` du déposant** → donnée personnelle, à ignorer |
| `pubDate` | `item_pubdate` = `date_added` | RFC 822, UTC ; `updateddate` idem (non émis en RSS 2.0) |
| `category` | `item_categories` = `(type,)` | titre du type : `Cours` (97/100 observés), `Autres`, `Podcast`, `Clip de présentation`… |
| `enclosure url/length/type` | `item_enclosure_*` | **absent si `password` ou `is_restricted`** ; sinon mp4 de **plus petite hauteur** (`sorted(..., key=height)[0]`), `type="video/mp4"`, `length` = taille fichier (0 si fichier manquant) ; audio : m4a (`video/mp4`) ; `/rss-audio/` : mp3 (`audio/mpeg`) |
| `itunes:subtitle` | = titre | |
| `itunes:duration` | `duration_in_time` | `HH:MM:SS` |
| `itunes:explicit` | `clean` | |
| `itunes:image` | miniature `https://video.umontpellier.fr/media/cache/<aa>/<bb>/<hash>.png` | élément texte (pas `href`) |
| `copyright` (item) | `item_copyright` = `obj.licence` | code licence de la vidéo ou vide |

Observation : **4 `<enclosure>` sur 100 items** → l'immense majorité des dépôts récents (cours PASS, formations médicales) est **restreinte ou protégée par mot de passe**. Les miniatures `media/cache/*` sont publiques même pour ces vidéos.

### 3.5 Recherche `/search/` **[code]**

`pod/video_search/views.py` `search_videos()` : requête Elasticsearch (`ES_INDEX=pod`) sur l'index construit avec `Video.get_json_to_index()`.

| Paramètre | Rôle |
|---|---|
| `q=` | texte (champ `name="q"` du formulaire, `SearchForm`) |
| `start_date=`, `end_date=` | bornes sur `date_added` (`YYYY-MM-DD`) |
| `selected_facets=terme:valeur` (répétable) | filtres exacts : `owner_full_name`, `type.title`, `disciplines.title`, `tags.name`, `channels.title` (+ variantes `.slug`/`.raw`) |
| `page=` | 0-based, **max 500** |

Réponse HTML ; fragment `videos/video_list.html` avec `X-Requested-With: XMLHttpRequest`. Les agrégations (facettes) sont rendues dans la page, pas en JSON. Si Elasticsearch est indisponible, la vue échoue (500) **[incertain]**.

### 3.6 Autres endpoints publics **[code]**

| URL | Réponse |
|---|---|
| `/info_pod.json` | `{"TITLE_SITE", "VERSION", "COUNT_VIDEO", "DURATION_VIDEO"}` — compte **toutes** les vidéos (brouillons inclus) ; durée totale en secondes. Non vérifié sur l'UM (limite de requêtes respectée) **[incertain]** |
| `/video/oembed/?url=<page vidéo>&format=json|xml&version=video` | oEmbed (`type: video`, `version 1.0`, `provider_name`, `provider_url`, `width 640`, `height 360`, `html` iframe) — **seulement si `OEMBED=True`** à l'UM **[incertain]** |
| `/rest/dublincore/` | XML Dublin Core de toutes les vidéos disponibles — mais derrière `IsAdminUser` (§2.1) |
| `/robots.txt` | — |

---

## 4. Modèle `Video` (champs utiles) **[code]** — `pod/video/models.py`

| Champ | Type | Sens |
|---|---|---|
| `id`, `slug` | int, slug | `slug` = `<id>-<slugify(title)>` (recalculé à la sauvegarde) |
| `title`, `description` | str, HTML | traduits (`title_fr`, `title_en` via modeltranslation) |
| `owner`, `additional_owners` | User | déposant(s) |
| `date_added` | datetime | ordre par défaut `-date_added` |
| `date_evt` | date | date de l'événement filmé |
| `date_delete` | date | suppression automatique programmée |
| `cursus`, `main_lang`, `licence` | codes | `CURSUS_CODES`, `LANG_CHOICES`, `LICENCE_CHOICES` |
| `type` (FK), `discipline`, `channel`, `theme`, `tags` | relations | |
| `is_draft` | bool | brouillon : visible du propriétaire ou via lien privé |
| `is_restricted` | bool | « accès restreint » = **authentification requise** |
| `restrict_access_to_groups` | M2M `AccessGroup` | groupes autorisés (codes LDAP/CAS, ex. affiliation) |
| `password` | str | mot de passe de lecture (comparé **en clair** dans `render_video`, bien que le modèle hache à la sauvegarde — incohérence du code **[incertain]**) |
| `allow_downloading`, `is_360`, `disable_comment` | bool | |
| `duration` (s), `duration_in_time` (`HH:MM:SS`) | | |
| `encoding_in_progress`, `encoded`, `get_encoding_step`, `is_video` | état | `is_video=False` → contenu audio |
| `thumbnail`, `overview` | images | `get_thumbnail_url(size="x720")` → `/media/cache/…` |
| `get_version` | `O`/`E`/`I`… | version par défaut (originale, enrichie, interactive) |

### 4.5 JSON d'indexation (`get_json_to_index`) — forme des `video_data` de l'API et de l'index ES

`id`, `title`, `owner` (username), `owner_full_name`, `date_added` / `date_evt` (`YYYY-MM-DDTHH:MM:SS`), `description`, `thumbnail`, `duration` (s, en chaîne), `tags[{name, slug}]`, `type{title, slug}`, `disciplines[{title, slug}]`, `channels[{title, slug}]`, `themes[{title, slug}]`, `contributors[{name, role}]`, `chapters[{title, slug}]`, `overlays[{title, slug}]`, `full_url` (`//video.umontpellier.fr/video/<slug>/`), `is_restricted`, `password` (bool), `duration_in_time`, `mediatype` (`video`/`audio`), `cursus` (libellé), `main_lang` (libellé).

---

## 5. Contrôle d'accès et vidéos restreintes **[code]**

`pod/video/utils.py` `get_video_access(request, video, slug_private)` :

```
si ni is_draft, ni is_restricted, ni groupe        → accès libre
privé  = slug_private == get_hashkey()
auteur = connecté et (propriétaire | superuser | perm video.change_video | co-propriétaire)
brouillon  → privé ou auteur
auteur     → oui
restreint  → connecté (si pas de groupe)
groupe     → connecté et membre d'un des AccessGroup (ou propriétaire/superuser/perm recorder.add_recording)
```

Puis `render_video()` (`pod/video/views.py`) :

| Situation | Comportement |
|---|---|
| accès OK, pas de mot de passe | page avec lecteur |
| mot de passe (et accès OK ou vidéo non restreinte) | page avec **formulaire** `VideoPasswordForm` ; `POST password=<…>` sur la même URL (CSRF requis) ; mauvais mot de passe → message « The password is incorrect. » |
| accès refusé, **non connecté** | `302` vers `LOGIN_URL` = **`/authentication_login/?referrer=<URL complète>`** (observé dans le lien « Connexion » de `/videos/` **[confirmé]**) |
| accès refusé, connecté | `403 PermissionDenied` + message « You cannot watch this video. » |

`/authentication_login/` (`pod/authentication/views.py`) affiche une page de choix (CAS / Shibboleth / OIDC / local selon `USE_*`) ; le bouton CAS mène à **`/accounts/login?next=<referrer>`** (`django_cas_ng.views.LoginView`) qui redirige vers `CAS_SERVER_URL/login?service=https://video.umontpellier.fr/accounts/login?next=…` **[incertain : l'ENT déclare pour cette appli le service `video.umontpellier.fr/index.php?action=rediriger`, probablement un reliquat de l'ancienne plateforme]**. Au retour, `django_cas_ng` valide le ticket (`/serviceValidate` ou `/p3/serviceValidate` selon `CAS_VERSION`), crée/maj l'utilisateur (`CAS_APPLY_ATTRIBUTES_TO_USER`, affiliation → `Owner.affiliation`, groupes → `AccessGroup`) et pose le cookie de session Django **`sessionid`** (+ `csrftoken`).

Conséquences pour un client :
- Une vidéo `is_restricted` devient lisible pour **tout compte CAS** ; une vidéo à groupe exige d'être dans le groupe (ex. une promo) ; un brouillon n'est jamais accessible sauf lien privé.
- Les fichiers média (`/media/videos/...mp4`, HLS) sont servis par le serveur web ; leur protection dépend de la conf Nginx/`X-Accel` de l'UM **[incertain]** — ne pas supposer qu'une URL `enclosure` connue est jouable sans session.
- Le lecteur et les URLs de sources ne sont fournis que dans la page vidéo (après contrôle d'accès).

---

## 6. Pièges

1. **`/rest/` est staff-only** : pas de jeton pour un étudiant, et un jeton obtenu « à titre personnel » ne suffirait pas (403). Ne pas construire tonAPI dessus.
2. **RSS sans pagination** (100 items) et **enclosures absentes** pour la majorité des vidéos (restreintes). Le flux reste la meilleure source de **nouveautés** (titre, lien, date, type, durée, miniature).
3. **Données personnelles** dans le RSS (`<author>` = e-mail + nom du déposant, `itunes:owner`) : filtrer avant stockage/affichage.
4. **Titres bruts** (`PASS_UE3_…_20261013.m4v`) : prévoir un nettoyage d'extension et de `_`.
5. **`<link>` du canal = `https://`** (bug) : ne pas l'utiliser ; `atom:link rel="self"` est correct.
6. **Chaînes** : la route `/<slug>/` est à la racine → tout slug inconnu renvoie 404, et `/channels/` sans `my/` n'existe pas.
7. **Recherche** = Elasticsearch côté serveur, HTML seulement (facettes non exposées en JSON).
8. **Login** : `/accounts/login` (sans slash) = CAS ; `/accounts/login/` (avec slash) = formulaire local. Un `WKWebView` sur `/authentication_login/?referrer=…` suffit pour obtenir `sessionid`.
9. **Courtoisie** : un seul site, pas de CDN visible ; espacer les GET de `/videos/` (12 par page, 16 994 vidéos ≈ 1 400 pages — ne jamais tout parcourir, utiliser les filtres `type`/`discipline`/`tag` ou le RSS).

---

## Pour tonAPI

**Verdict** : pas d'API JSON publique ; l'API REST est réservée aux administrateurs. Le service se branche en **lecture publique** (RSS + HTML) et, pour les vidéos restreintes, en **session CAS** côté app.

**À implémenter** :
1. **Nouveautés** : `GET /rss-video/` (et `/rss-video/<chaîne>/` pour une chaîne suivie) → `title`, `link` (→ `id` et `slug`), `pubDate`, `category` (type), `itunes:duration`, `itunes:image`, présence d'`enclosure` (⇔ vidéo publique). Ignorer `author`. Cache 15 min.
2. **Catalogue filtré** : `GET /videos/?type=cours&page=N` avec `X-Requested-With: XMLHttpRequest` → parser les cartes (`a[href^="/video/"]`, `.time`, `img.pod-thumbnail`, icônes `bi-lock`/`bi-key`) ; lire `data-countvideos`/`data-nextpage`.
3. **Fiche vidéo** : `GET /video/<id>-<slug>/` → `<meta name="dc.*">` pour titre/auteur/date/langue ; état d'accès = code HTTP (200 / 302 vers `/authentication_login/` / 403) et présence du formulaire mot de passe.
4. **Lecture** : ouvrir la page dans une `WKWebView` partageant le cookie jar CAS (les cookies CAS `TGC`/`connect.sid` de [cas-umontpellier.md](../cas-umontpellier.md) permettent le SSO silencieux via `/accounts/login?next=…`). Ne pas tenter d'extraire les URLs mp4/HLS pour un lecteur natif tant que la protection des médias n'est pas connue.
5. **Chaînes** : `GET /video/get-channel-tabs/` puis `/video/get-channels-for-specific-channel-tab/?id=…` (JSON) pour proposer un abonnement à une chaîne (→ son RSS).
6. **Méta** : `GET /info_pod.json` une fois par jour pour `VERSION`/`COUNT_VIDEO` (détection de montée de version Pod, qui peut changer le HTML).

**Modèle suggéré** (`PodVideo`) : `id`, `slug`, `title` (nettoyé), `url`, `publishedAt`, `type`, `duration` (s), `thumbnailURL`, `access` (`public` | `restricted` | `password` | `unknown`), `channel?`.

**À tester avec un compte étudiant** :
- [ ] `/authentication_login/?referrer=…` → bouton CAS → URL `service` réelle envoyée au CAS (`/accounts/login?next=…` ?)
- [ ] une vidéo `bi-lock` après login CAS : lisible ? (`is_restricted` simple) ; une vidéo de promo : 403 ? (groupe)
- [ ] URL `enclosure` d'une vidéo publique : jouable sans cookie ? et celle d'une vidéo restreinte (lue dans la page après login) : jouable hors session ?
- [ ] `/info_pod.json`, `/video/oembed/?url=` (activé ?), `/video/get-channel-tabs/`
- [ ] `/rest/videos/` avec le cookie `sessionid` d'un étudiant → 403 attendu (confirme l'inutilité de l'API)
