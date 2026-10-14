// SPDX-FileCopyrightText: The Pixel contributors
// SPDX-License-Identifier: MIT

//! `pixel-rank` — the pure fusion core for `pixel scope-task` and (later) ranked
//! `search`/`resolve`.
//!
//! Task text in, closed prioritized file list out. The `op_targets` service
//! op in `pixel-daemon` gathers `SignalInputs` from the trigram index and the
//! code graph; this crate tokenizes the task, fuses the per-signal ranked
//! lists with reciprocal-rank fusion (mirroring `pixel-recall`'s hybrid
//! channel fusion), and assigns P0/P1/P2 tiers. Everything here is
//! deterministic: total orders everywhere, ties broken by path ascending,
//! scores rounded for byte-stable JSON.
//!
//! No I/O, no daemon, no index — pure functions over caller-supplied
//! `SignalInputs`. Per `PLAN.md`'s crate table this holds "the pure fusion
//! core (port of serve/targets.rs): signal registry + weighted RRF (K=60),
//! P0/P1/P2 tiering; slots for recency/churn/session signals."

use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub mod regions;
pub mod rerank;
pub mod signals;

pub use pixel_graph::split_ident_words;
pub use pixel_graph::store::SymbolRow;
pub use pixel_graph::targets::SymbolHit;

/// A ranked target file from `compute_targets`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TargetFile {
    pub path: String,
    pub tier: String,
    pub score: f64,
    pub reasons: Vec<String>,
    pub symbols: Vec<RankedSymbol>,
}

/// Symbol metadata kept for P0 targets.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RankedSymbol {
    pub name: String,
    pub kind: String,
    pub line: Option<u64>,
    pub uid: String,
}

/// Options controlling target computation.
#[derive(Debug, Clone)]
pub struct TargetsOptions {
    /// Maximum number of target files to return.
    pub limit: usize,
    /// Enable precision mode: more aggressive tiering.
    pub precision_mode: bool,
    /// Maximum tier to include (None = all tiers).
    pub max_tier: Option<String>,
}

/// Default limit for target computation.
pub const DEFAULT_LIMIT: usize = 20;

impl Default for TargetsOptions {
    fn default() -> Self {
        TargetsOptions {
            limit: DEFAULT_LIMIT,
            precision_mode: false,
            max_tier: None,
        }
    }
}

/// Input signals for target computation.
#[derive(Debug, Clone)]
pub struct SignalInputs {
    pub all_paths: Vec<String>,
    pub content_hits: BTreeMap<String, Vec<(String, u32)>>,
    pub symbol_hits: Vec<SymbolHit>,
    pub cluster_neighbors: Vec<(String, String)>,
    pub graph_neighbors: Vec<(String, String)>,
    pub path_hits: Vec<(String, String)>,
    pub graph_available: bool,
    pub envelope: Option<pixel_graph::Envelope>,
    pub caps: Vec<String>,
}

impl Default for SignalInputs {
    fn default() -> Self {
        SignalInputs {
            all_paths: vec![],
            content_hits: BTreeMap::new(),
            symbol_hits: vec![],
            cluster_neighbors: vec![],
            graph_neighbors: vec![],
            path_hits: vec![],
            graph_available: false,
            envelope: None,
            caps: vec![],
        }
    }
}

/// The result of computing ranked targets for a task.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TargetsReport {
    pub task: String,
    pub keywords: Vec<String>,
    pub exact_tokens: Vec<String>,
    pub path_tokens: Vec<String>,
    pub targets: Vec<TargetFile>,
    pub envelope: Value,
    pub closed_world: String,
    pub stats: Value,
}

// BM25 parameters
pub const BM25_K1: f64 = 1.5;
pub const BM25_B: f64 = 0.75;

/// A document for BM25 scoring.
#[derive(Debug, Clone)]
pub struct Bm25Doc {
    pub path: String,
    pub term_freqs: Vec<u32>,
    pub len: u32,
}

/// Compute BM25 scores for a list of documents over a list of terms.
/// Returns None if any term has zero df or any doc has zero length.
pub fn bm25_scores(terms: &[String], docs: &[Bm25Doc]) -> Option<Vec<f64>> {
    if terms.is_empty() || docs.is_empty() {
        return None;
    }
    let avgdl: f64 = docs.iter().map(|d| d.len as f64).sum::<f64>() / docs.len() as f64;
    let df: Vec<usize> = terms
        .iter()
        .map(|t| {
            docs.iter()
                .filter(|d| d.term_freqs.iter().any(|&tf| tf > 0))
                .count()
        })
        .collect();
    if df.iter().any(|&d| d == 0) {
        return None;
    }
    let idf: Vec<f64> = terms
        .iter()
        .zip(df.iter())
        .map(|(_, &d)| (docs.len() as f64 + 0.5) / (d as f64 + 0.5))
        .map(|x| x.ln())
        .collect();
    let scores: Vec<f64> = docs
        .iter()
        .map(|doc| {
            doc.term_freqs
                .iter()
                .zip(terms.iter())
                .zip(idf.iter())
                .map(|((&tf, _), &idf_val)| {
                    if tf == 0 {
                        0.0
                    } else {
                        let norm = 1.0 - BM25_B + BM25_B * (doc.len as f64 / avgdl);
                        (tf as f64 * (BM25_K1 + 1.0)) / ((tf as f64) + BM25_K1 * norm) * idf_val
                    }
                })
                .sum()
        })
        .collect();
    if scores.iter().any(|&s| s.is_nan() || s.is_infinite()) {
        return None;
    }
    Some(scores)
}

/// Round BM25 score to integer micro-units.
pub fn bm25_fixed(score: f64) -> i64 {
    (score * 1_000_000.0) as i64
}

/// Rank docs by BM25 score, descending.
pub fn bm25_rank(terms: &[String], docs: &[Bm25Doc]) -> Option<Vec<String>> {
    let scores = bm25_scores(terms, docs)?;
    let mut indexed: Vec<(usize, f64)> = scores.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Some(
        indexed
            .into_iter()
            .map(|(i, _)| docs[i].path.clone())
            .collect(),
    )
}

// Signal weights
const W_FILENAME: u32 = 1;
const W_SYMBOL: u32 = 2;
const W_CONTENT: u32 = 4;
const W_GRAPH: u32 = 8;
const W_CLUSTER: u32 = 16;
const W_PATH: u32 = 32;
const PATH_MASK: u32 = 64;
const PRIMARY_MASK: u32 = 7; // W_FILENAME | W_SYMBOL | W_CONTENT
const LEXICAL_MASK: u32 = 7;
const EXACT_NAME_BONUS: f64 = 100.0;
pub const RRF_K: f64 = 60.0;
pub const W_FILENAME_F: f64 = 1.0;
pub const W_SYMBOL_F: f64 = 2.0;
pub const W_CONTENT_F: f64 = 4.0;
pub const W_GRAPH_F: f64 = 8.0;
pub const W_CLUSTER_F: f64 = 16.0;
pub const W_SEMANTIC_F: f64 = 3.0;
pub const W_PATH_F: f64 = 32.0;
const PATH_MASK_F: f64 = 64.0;
const P0_CAP: usize = 10;
const SCORE_GAP_RATIO: f64 = 0.5;
const SECONDARY_GAP_RATIO: f64 = 0.5;

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// Intermediate fused entry during target computation.
#[derive(Debug, Clone)]
struct Entry {
    score: f64,
    reasons: Vec<String>,
    symbols: Vec<RankedSymbol>,
    path_hit: bool,
    exact_name_hit: bool,
    families: u32,
}

// ---------------------------------------------------------------------------
// tokenizer
// ---------------------------------------------------------------------------

/// Pure-function words that carry no retrieval signal in a task description.
/// Domain-ish words (error, parse, auth, …) deliberately stay searchable.
const STOPWORDS: &[&str] = &[
    // articles / prepositions / pronouns / auxiliaries
    "the",
    "and",
    "for",
    "with",
    "that",
    "this",
    "its",
    "it",
    "a",
    "an",
    "to",
    "in",
    "of",
    "on",
    "at",
    "by",
    "is",
    "are",
    "be",
    "was",
    "were",
    "or",
    "as",
    "so",
    "we",
    "our",
    "my",
    "your",
    "into",
    "from",
    "when",
    "then",
    "than",
    "them",
    "they",
    "there",
    "all",
    "any",
    "can",
    "will",
    "should",
    "must",
    "not",
    "but",
    "without",
    "how",
    "what",
    "why",
    // task verbs / filler
    "fix",
    "add",
    "implement",
    "support",
    "make",
    "create",
    "update",
    "remove",
    "change",
    "refactor",
    "improve",
    "ensure",
    "allow",
    "use",
    "need",
    "want",
    "please",
    "new",
    "also",
    "file",
    "files",
    "code",
    "feature",
    "bug",
    "task",
    "issue",
];

/// French function words, dropped only from a task detected as French
/// ([`TaskLanguage::French`]): several are English words or identifiers
/// (`comment`, `car`, `plus`, `des`, `est`), and dropping them from an
/// English task lost its subject ("fix comment parsing" searched `parsing`).
/// Post-diacritic-fold forms. Note: These are intentionally excluded from
/// FRENCH_STOPWORDS to prevent over-filtering when French words appear in English context.
const FRENCH_STOPWORDS: &[&str] = &[
    // French articles / prepositions / pronouns / conjunctions
    "le", "la", "les", "un", "une", "des", "du", "de", "et", "ou", "ne", "pas", "que", "qui",
    "dans", "pour", "sur", "avec", "sans", "ce", "cette", "ces", "son", "sa", "ses", "mon", "ma",
    "mes", "ton", "ta", "tes", "notre", "nos", "votre", "vos", "leur", "leurs", "il", "elle",
    "ils", "elles", "nous", "vous", "je", "tu", "se", "lui", "en", "y", "tout", "tous", "toute",
    "toutes", "rien", "personne", "chaque", "quelque", "tres", "plus", "moins", "aussi", "comme",
    "si", "alors", "donc", "mais", "car", "parce",
    // French auxiliaries / question words / adverbs. Post-diacritic-fold forms.
    "est", "sont", "etre", "ete", "avoir", "ont", "faire", "fait", "peut", "doit", "faut",
    "comment", "pourquoi", "quand", "quel", "quelle", "quels", "quelles", "dont", "ca", "cela",
    "meme", "bien", "encore", "deja", "jamais", "toujours", "ici", "ensuite", "puis", "quoi",
];

/// French words no English task uses: two distinct ones (or two
/// unambiguous French relation keys, see [`FRENCH_RELATIONS`]) make a task
/// French. Post-diacritic-fold forms.
const FRENCH_MARKERS: &[&str] = &[
    "le", "les", "une", "des", "du", "et", "pas", "que", "qui", "dans", "pour", "sur", "avec",
    "cette", "ces", "est", "sont", "nous", "vous", "je", "ils", "elles", "mais", "donc",
    "pourquoi", "quand", "quelle", "quels", "quelles", "dont", "cela", "deja", "toujours",
    "jamais", "peut", "doit", "faut", "etre", "avoir", "ne", "au", "aux",
];

/// Distinct French markers a task needs to be treated as French.
const FRENCH_MARKER_THRESHOLD: usize = 2;

/// The language of a task as far as retrieval cares: which stopwords are
/// dropped and whether French relation keys that are also English words
/// expand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskLanguage {
    #[default]
    English,
    French,
}

/// French when [`FRENCH_MARKER_THRESHOLD`] distinct markers occur among the
/// words of `folded_task` (diacritics already folded).
pub fn detect_language(folded_task: &str) -> TaskLanguage {
    let words: HashSet<String> = folded_task
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .collect();
    let relation_keys = FRENCH_RELATIONS
        .iter()
        .map(|(key, _)| *key)
        .filter(|key| !FRENCH_HOMOGRAPHS.contains(key));
    let markers = FRENCH_MARKERS
        .iter()
        .copied()
        .chain(relation_keys)
        .filter(|marker| words.contains(*marker))
        .count();
    if markers >= FRENCH_MARKER_THRESHOLD {
        TaskLanguage::French
    } else {
        TaskLanguage::English
    }
}

const MAX_KEYWORDS: usize = 12;
const MIN_KEYWORD_LEN: usize = 3;

/// Static, code-domain thesaurus for semantic keyword expansion. Each keyword
/// is mapped to a small, conservative set of related terms used only to
/// broaden path/filename matching; the original keywords remain canonical for
/// content probes and report labels.
const SEMANTIC_RELATIONS: &[(&str, &[&str])] = &[
    ("login", &["auth", "authenticate", "signin", "session"]),
    ("auth", &["login", "authenticate", "session", "user"]),
    ("authenticate", &["login", "auth", "session", "user"]),
    ("signin", &["login", "auth", "authenticate"]),
    ("user", &["account", "login", "profile"]),
    ("account", &["user", "login", "profile"]),
    ("session", &["cookie", "login", "auth", "token"]),
    ("cookie", &["session", "auth", "login"]),
    ("token", &["jwt", "session", "auth", "credential"]),
    ("jwt", &["token", "auth", "session"]),
    ("credential", &["token", "password", "auth", "login"]),
    ("password", &["credential", "auth", "login"]),
    ("config", &["settings", "configuration", "options"]),
    ("settings", &["config", "configuration", "options"]),
    ("configuration", &["config", "settings", "options"]),
    ("database", &["sql", "query", "schema"]),
    ("sql", &["database", "query"]),
    ("query", &["sql", "database", "request"]),
    ("api", &["endpoint", "route", "request", "response"]),
    ("endpoint", &["api", "route", "request"]),
    ("request", &["response", "api", "route", "endpoint"]),
    ("response", &["request", "api", "route", "endpoint"]),
    ("route", &["endpoint", "api", "request"]),
    ("error", &["exception", "failure", "panic"]),
    ("exception", &["error", "failure"]),
    ("failure", &["error", "exception"]),
    ("panic", &["error", "exception"]),
    ("cache", &["store", "storage"]),
    ("store", &["cache", "storage"]),
    ("storage", &["cache", "store"]),
    ("queue", &["job", "worker", "background"]),
    ("job", &["queue", "worker", "task"]),
    ("worker", &["queue", "job", "background"]),
    ("background", &["queue", "job", "worker"]),
    ("email", &["mail"]),
    ("mail", &["email"]),
    // Languages, compilers, parser & AST
    (
        "csharp",
        &["language", "extract", "grammar", "c_sharp", "cs"],
    ),
    ("python", &["language", "extract", "grammar", "py"]),
    ("ruby", &["language", "extract", "grammar", "rb", "rails"]),
    ("rust", &["language", "extract", "grammar", "rs"]),
    ("javascript", &["language", "extract", "grammar", "js"]),
    ("typescript", &["language", "extract", "grammar", "ts"]),
    ("golang", &["language", "extract", "grammar", "go"]),
    ("java", &["language", "extract", "grammar"]),
    ("cpp", &["language", "extract", "grammar", "cplusplus"]),
    ("language", &["extract", "grammar", "parser", "syntax"]),
    ("parser", &["language", "extract", "grammar", "syntax"]),
    ("grammar", &["language", "extract", "parser"]),
];

/// French → English cross-lingual mappings (software-development terms).
const FRENCH_RELATIONS: &[(&str, &[&str])] = &[
    // French → English cross-lingual mappings (software-development terms).
    // Keys must be producible by `tokenize_task`: no accents (folded), no
    // underscores (split_ident_words splits on `_`), single words only.
    // Multi-word compounds live in `normalize_task_compounds` instead.
    ("connexion", &["login", "auth", "connection", "session"]),
    ("utilisateur", &["user", "account", "profile"]),
    ("formulaire", &["form", "input", "submit"]),
    ("bouton", &["button", "click", "action"]),
    ("lien", &["link", "navigation", "route"]),
    ("erreur", &["error", "exception", "fault"]),
    ("corriger", &["fix", "repair", "patch", "resolve"]),
    ("ajouter", &["add", "create", "insert"]),
    ("supprimer", &["remove", "delete", "destroy"]),
    ("modifier", &["edit", "update", "modify", "change"]),
    ("afficher", &["display", "show", "render", "view"]),
    ("chercher", &["search", "find", "query", "lookup"]),
    ("sauvegarder", &["save", "store", "persist"]),
    ("charger", &["load", "fetch", "retrieve"]),
    ("envoyer", &["send", "submit", "dispatch"]),
    ("recevoir", &["receive", "fetch", "get"]),
    ("valider", &["validate", "check", "verify"]),
    ("deconnexion", &["logout", "signout", "disconnect"]),
    ("inscription", &["register", "signup"]),
    ("profil", &["profile", "account", "user"]),
    ("message", &["notification", "alert"]),
    ("fichier", &["file", "document", "attachment"]),
    ("dossier", &["folder", "directory", "path"]),
    ("accueil", &["home", "dashboard", "index"]),
    ("parametre", &["settings", "config", "preference"]),
    ("recherche", &["search", "query", "find"]),
    ("resultat", &["result", "output", "response"]),
    ("commande", &["command", "order", "cmd"]),
    ("panier", &["cart", "basket", "order"]),
    ("produit", &["product", "item", "goods"]),
    ("paiement", &["payment", "checkout", "transaction"]),
    ("facture", &["invoice", "bill", "receipt"]),
    ("client", &["customer", "user"]),
    ("serveur", &["server", "backend", "api"]),
    ("requete", &["query", "request", "sql"]),
    ("reponse", &["response", "reply", "result"]),
    ("evenement", &["event", "trigger", "handler"]),
    ("ecouter", &["listen", "subscribe", "observe"]),
    ("arreter", &["stop", "halt", "cancel"]),
    ("demarrer", &["start", "init", "launch", "boot"]),
    // Additional FR→EN: common dev-domain terms.
    ("authentification", &["auth", "login", "authentication"]),
    ("autorisation", &["permission", "auth", "authorize"]),
    ("donnees", &["data", "store", "payload"]),
    ("fonctionnalite", &["feature", "functionality"]),
    ("probleme", &["problem", "issue", "bug"]),
    ("bogue", &["bug", "issue", "defect"]),
    ("echec", &["failure", "error", "fail"]),
    ("panne", &["failure", "outage", "error"]),
    ("cle", &["key", "token"]),
    ("valeur", &["value", "val"]),
    ("champ", &["field", "column", "property"]),
    ("entree", &["input", "entry"]),
    ("sortie", &["output", "exit"]),
    ("fonction", &["function", "method", "fn"]),
    ("variable", &["variable", "var"]),
    ("chaine", &["string", "str"]),
    ("tableau", &["array", "table", "list"]),
    ("liste", &["list", "array"]),
    ("boucle", &["loop", "iterate"]),
    ("tri", &["sort", "order"]),
    ("filtre", &["filter", "where"]),
    ("journal", &["log", "journal"]),
    ("deboguer", &["debug", "troubleshoot"]),
    ("deploiement", &["deploy", "deployment"]),
    ("securite", &["security", "auth", "permission"]),
    ("reseau", &["network", "net"]),
    ("memoire", &["memory", "cache"]),
    ("compte", &["account", "user", "profile"]),
    ("identifiant", &["identifier", "id", "username"]),
    ("courriel", &["email", "mail"]),
    ("adresse", &["address", "url", "email"]),
    ("retour", &["return", "back"]),
    ("ouvrir", &["open", "read"]),
    ("fermer", &["close", "shutdown"]),
    ("lire", &["read", "load"]),
    ("ecrire", &["write", "save"]),
    ("creer", &["create", "make"]),
    ("executer", &["run", "execute"]),
    ("appeler", &["call", "invoke"]),
    ("installer", &["install", "setup"]),
    ("configurer", &["configure", "setup"]),
    ("tache", &["task", "job"]),
    ("etat", &["state", "status"]),
    ("modele", &["model", "schema"]),
    ("vue", &["view", "render"]),
    ("controleur", &["controller", "handler"]),
    ("dependance", &["dependency", "dep"]),
    ("contenu", &["content", "body"]),
    ("nom", &["name", "label"]),
    ("numero", &["number", "id"]),
    ("prix", &["price", "cost"]),
    ("taille", &["size", "length"]),
    ("couleur", &["color", "style"]),
    ("theme", &["theme", "style"]),
    ("image", &["image", "img", "icon"]),
    ("onglet", &["tab"]),
    ("menu", &["menu", "navigation"]),
    ("fenetre", &["window", "dialog"]),
    ("ecran", &["screen", "display"]),
    ("affichage", &["display", "render"]),
    ("chargement", &["loading", "load"]),
    ("enregistrer", &["save", "store"]),
    ("telecharger", &["download", "fetch"]),
    ("importer", &["import"]),
    ("exporter", &["export"]),
    ("synchroniser", &["sync", "synchronize"]),
    ("deployer", &["deploy"]),
];

/// [`FRENCH_RELATIONS`] keys that are also English words: in an English task
/// `client` is an HTTP client, not a customer, and `modifier` a keyword, not
/// "edit". They expand only in a French task.
const FRENCH_HOMOGRAPHS: &[&str] = &[
    "client",
    "charger",
    "dossier",
    "image",
    "importer",
    "installer",
    "journal",
    "lien",
    "menu",
    "message",
    "modifier",
    "tableau",
    "theme",
    "variable",
    "vue",
];

pub const SHORT_TECH_KEYWORDS: &[&str] = &[
    "c", "r", "go", "rs", "ts", "js", "py", "rb", "sh", "ui", "ci", "cd", "db", "os", "io", "ip",
    "ai", "ml",
];

/// Normalize compound technical terms (e.g. "c#", "c++", ".net", "node.js")
/// into clean alphanumeric words before word splitting.
pub fn normalize_task_compounds(task: &str) -> String {
    let mut out = task.to_string();
    let replacements = [
        ("c#", "csharp"),
        ("c-sharp", "csharp"),
        ("c_sharp", "csharp"),
        ("ruby on rails", "ruby"),
        ("c++", "cpp"),
        ("f#", "fsharp"),
        (".net", "dotnet"),
        ("node.js", "nodejs"),
        ("vue.js", "vuejs"),
        ("next.js", "nextjs"),
        ("react.js", "reactjs"),
        // French multi-word compounds (post-diacritic-fold). These must be
        // replaced before tokenization because `split_ident_words` would
        // split them into unrecognizable parts.
        ("mot de passe", "password"),
        ("base de donnees", "database"),
        ("coup de main", "help"),
    ];
    for (from, to) in replacements {
        let mut lower = out.to_ascii_lowercase();
        while let Some(idx) = lower.find(from) {
            let mut result = String::with_capacity(out.len() + to.len());
            result.push_str(&out[..idx]);
            result.push_str(to);
            result.push_str(&out[idx + from.len()..]);
            out = result;
            lower = out.to_ascii_lowercase();
        }
    }
    out
}

/// Expand a single keyword of a task in `language` to its semantic relatives.
pub fn semantic_expand(word: &str, language: TaskLanguage) -> Vec<&'static str> {
    let french = language == TaskLanguage::French || !FRENCH_HOMOGRAPHS.contains(&word);
    let french_relations: &[(&str, &[&str])] = if french { FRENCH_RELATIONS } else { &[] };
    SEMANTIC_RELATIONS
        .iter()
        .chain(french_relations)
        .find(|(term, _)| *term == word)
        .map(|(_, related)| related.to_vec())
        .unwrap_or_default()
}

/// Expand a list of keywords, returning related terms that are not already
/// present in the original list. Deterministic: preserves keyword order and
/// then first-occurrence order of the thesaurus.
pub fn expand_keywords(keywords: &[String], language: TaskLanguage) -> Vec<String> {
    let mut seen: HashSet<&str> = keywords.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for kw in keywords {
        for related in semantic_expand(kw, language) {
            if seen.insert(related) {
                out.push(related.to_string());
            }
        }
    }
    out
}

/// Input for target computation: task tokens and language.
#[derive(Debug, Clone, Default)]
pub struct TaskQuery {
    /// Backticked/quoted identifiers taken verbatim: `` `login_user` `` →
    /// exact symbol-name probe (case preserved). Unquoted `snake_case` words
    /// are identifiers too and land here as well.
    pub exact_tokens: Vec<String>,
    /// File paths named in the task (`upgrade_cli.rs`, `src/auth.rs`),
    /// matched against the live tree by the path signal.
    pub path_tokens: Vec<String>,
    /// Lowercased keywords, first-occurrence order, deduped, len ≥ 3 (or short tech keyword), ≤ 12.
    pub keywords: Vec<String>,
    /// True when the task contained MORE searchable keywords than
    /// [`MAX_KEYWORDS`]: the dropped words contributed no signal, so any
    /// result built from this query is a lower bound, not exhaustive.
    pub keywords_truncated: bool,
    /// Detected language: selects the stopwords and the expansions.
    pub language: TaskLanguage,
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Extensions that mark a dotted token as a file path rather than an
/// identifier or a sentence-ending abbreviation. Lowercase; compared
/// case-insensitively.
const PATH_EXTENSIONS: &[&str] = &[
    "rs", "rb", "go", "ts", "js", "jsx", "tsx", "java", "py", "cs", "php", "pl", "pm", "sh",
    "bash", "zsh", "fish", "ps1", "bat", "cmd", "swift", "kt", "kts", "scala", "clj", "cljs",
    "elm", "ml", "elm", "jl", "r", "R", "lua", "dart", "ex", "exs", "hs", "hs-boot", "f90", "f95",
    "f03", "f08", "f", "for", "f77", "f90",
];

/// Tokens that look like identifiers but should be treated as file paths
/// when they contain one of the above extensions.
fn is_path_token(token: &str) -> bool {
    token.contains('.') && {
        let ext = token.rsplit('.').next().unwrap_or("");
        PATH_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }
}

/// A task word that is a backtick-quoted identifier: `` `login_user` `` →
/// exact match, exact_tokens
fn try_extract_backticked_ident(s: &str) -> Option<String> {
    let s = s.trim();
    if s.starts_with('`') && s.ends_with('`') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        if inner
            .chars()
            .next()
            .map_or(false, |c| c.is_ascii_alphabetic())
            || inner.contains('_')
        {
            return Some(inner.to_string());
        }
    }
    None
}

/// Tokenize a task description into structured query components.
pub fn tokenize_task(task: &str) -> Result<TaskQuery, String> {
    // Normalize technical compounds first (e.g. "c#" → "csharp")
    let task = normalize_task_compounds(task);
    // Fold diacritics for case-insensitive matching
    let task = fold_diacritics(&task);
    // Split into whitespace-separated tokens
    let tokens: Vec<String> = task.split_whitespace().map(|s| s.to_string()).collect();

    let mut exact_tokens: Vec<String> = Vec::new();
    let mut path_tokens: Vec<String> = Vec::new();
    let mut keywords: HashSet<String> = HashSet::new();
    let mut found_keyword = false;

    for token in tokens {
        // Check for backticked identifier
        if let Some(ident) = try_extract_backticked_ident(&token) {
            exact_tokens.push(ident);
            continue;
        }

        // Check for file path (contains dot + known extension)
        if is_path_token(&token) {
            // Extract full path if present
            let path = extract_path(&token);
            if !path.is_empty() {
                path_tokens.push(path);
                continue;
            }
        }

        // Process as potential keyword
        let lower = token.to_ascii_lowercase();
        if lower.len() >= MIN_KEYWORD_LEN || SHORT_TECH_KEYWORDS.contains(&lower.as_str()) {
            // Skip if it's a known keyword
            if is_stopword(&lower, TaskLanguage::English) {
                continue;
            }
            keywords.insert(lower);
            found_keyword = true;
        }
    }

    if !found_keyword && keywords.is_empty() {
        return Err("no searchable keywords in task".to_string());
    }

    let keyword_count = keywords.len();
    let mut keyword_vec: Vec<String> = keywords.into_iter().collect();
    keyword_vec.sort(); // Deterministic ordering

    if keyword_vec.len() > MAX_KEYWORDS {
        keyword_vec.truncate(MAX_KEYWORDS);
    }

    Ok(TaskQuery {
        exact_tokens,
        path_tokens,
        keywords: keyword_vec.clone(),
        keywords_truncated: keyword_vec.len() < keyword_count,
        language: TaskLanguage::English,
    })
}

fn is_stopword(word: &str, lang: TaskLanguage) -> bool {
    match lang {
        TaskLanguage::English => STOPWORDS.contains(&word),
        TaskLanguage::French => FRENCH_STOPWORDS.contains(&word),
    }
}

fn bump<'a>(
    fused: &'a mut HashMap<String, Entry>,
    path: &str,
    rank: usize,
    weight: u32,
    family: u32,
) -> &'a mut Entry {
    let score = weight as f64 / (RRF_K + rank as f64);
    fused
        .entry(path.to_string())
        .and_modify(|e| {
            e.score += score;
            e.families |= family;
        })
        .or_insert(Entry {
            score,
            reasons: Vec::new(),
            symbols: Vec::new(),
            path_hit: false,
            exact_name_hit: false,
            families: family,
        })
}

fn fold_diacritics(s: &str) -> String {
    // Simple diacritic folding: remove common combining marks
    s.chars()
        .filter(|c| !matches!(*c, '\u{0300}'..='\u{036F}'))
        .collect()
}

fn extract_path(token: &str) -> String {
    // For a token like "upgrade_cli.rs" or "src/auth.rs", extract the path
    // If it contains slashes, use as-is; otherwise just the basename
    if token.contains('/') {
        token.to_string()
    } else {
        token.to_string()
    }
}

/// The main entry point: compute ranked target files for a task.
///
/// Takes task text, tokenizes it, fuses per-signal ranked lists via RRF,
/// assigns P0/P1/P2 tiers, and returns a closed-world answer with
/// explicit caps when the result may be incomplete.
pub fn compute_targets(
    task: &str,
    query: &TaskQuery,
    inputs: SignalInputs,
    opts: &TargetsOptions,
) -> TargetsReport {
    let limit = opts.limit;

    // Canonical keyword list used for content/symbol probes.
    let ranking_keywords: Vec<String> = query
        .keywords
        .iter()
        .cloned()
        .chain(expand_keywords(&query.keywords, query.language))
        .collect();

    // Per-signal ranked lists.
    let s1 = filename_rank(&inputs.all_paths, &ranking_keywords);
    let s2 = &inputs.symbol_hits;
    let s3 = content_rank(&inputs.content_hits);
    let s4 = &inputs.graph_neighbors;
    let s5 = &inputs.cluster_neighbors;
    let s6 = &inputs.path_hits;

    // Fuse.
    let mut fused: HashMap<String, Entry> = HashMap::new();

    for (rank, (path, matched)) in s1.iter().enumerate() {
        let e = bump(&mut fused, path, rank, W_FILENAME, W_FILENAME);
        e.reasons
            .push(format!("filename match: {}", matched.join(", ")));
    }
    for (rank, hit) in s2.iter().enumerate() {
        let e = bump(&mut fused, &hit.path, rank, W_SYMBOL, W_SYMBOL);
        e.exact_name_hit |= hit.exact_name_hit;
        for (sym, _) in hit.symbols.iter().take(3) {
            e.reasons.push(format!("defines symbol `{}`", sym.name));
            e.symbols.push(symbol_value(sym));
        }
        if hit.symbols.len() > 3 {
            e.reasons
                .push(format!("+{} more matching symbols", hit.symbols.len() - 3));
        }
    }
    for (rank, (path, per_kw)) in s3.iter().enumerate() {
        let e = bump(&mut fused, path, rank, W_CONTENT, W_CONTENT);
        let detail: Vec<String> = per_kw
            .iter()
            .take(3)
            .map(|(kw, n)| format!("{n} for \"{kw}\""))
            .collect();
        e.reasons
            .push(format!("content matches: {}", detail.join(", ")));
    }
    for (rank, (path, reason)) in s4.iter().enumerate() {
        let e = bump(&mut fused, path, rank, W_GRAPH, W_GRAPH);
        e.reasons.push(reason.clone());
    }
    for (rank, (path, reason)) in s5.iter().enumerate() {
        let e = bump(&mut fused, path, rank, W_CLUSTER, W_CLUSTER);
        e.reasons.push(reason.clone());
    }
    for (rank, (path, token)) in s6.iter().enumerate() {
        let e = bump(&mut fused, path, rank, W_PATH, PATH_MASK);
        e.path_hit = true;
        e.reasons.push(format!("path match: {token}"));
    }

    // Exact-identifier definitions get a flat bonus: a backticked name that is
    // defined in the file is the strongest evidence a task can carry.
    for e in fused.values_mut() {
        if e.exact_name_hit {
            e.score += EXACT_NAME_BONUS;
        }
    }

    // Deterministic fused order.
    let mut ordered: Vec<(String, Entry)> = fused.into_iter().collect();
    ordered.sort_by(|a, b| b.1.score.total_cmp(&a.1.score).then(a.0.cmp(&b.0)));

    // Tier assignment. fams = number of signal families; lexical = S1|S2|S3.
    let p2_cap = limit.div_ceil(4);
    let mut targets: Vec<TargetFile> = Vec::new();
    let mut p0 = 0usize;
    let mut p2 = 0usize;
    // Cap accounting: every candidate this loop DROPS (p2 cap, limit) is a
    // file the caller will never see — that must surface as lower_bound +
    // a named cap, never be silently absorbed into an "exhaustive" claim.
    let mut p2_dropped = 0usize;
    let mut beyond_limit = 0usize;
    for (path, e) in ordered {
        if targets.len() >= limit {
            beyond_limit += 1;
            continue;
        }
        let lexical = e.families & PRIMARY_MASK != 0;
        // Two LEXICAL families, not merely two families: a filename or
        // symbol hit plus a graph neighbor is not enough to claim the file
        // is primary. An explicit path named in the task is primary on its
        // own; it boosts and promotes, but never hides other lexical targets.
        let lexical_families = (e.families & LEXICAL_MASK).count_ones();
        let p0_rule = e.path_hit || e.exact_name_hit || lexical_families >= 2;
        let tier = if p0_rule && p0 < P0_CAP.min(limit) {
            p0 += 1;
            "P0"
        } else if lexical {
            "P1"
        } else {
            // Graph/cluster-only evidence: peripheral and droppable.
            if p2 >= p2_cap {
                p2_dropped += 1;
                continue;
            }
            p2 += 1;
            "P2"
        };
        // Structured symbol metadata (kind/line/uid) is dropped for P1/P2:
        // `reasons` already names the same symbols in prose ("defines
        // symbol `X`"), and duplicating that as a structured array for
        // every tier measurably bloated the response (3.6KB of an 11.8KB
        // response on a real 20-target query, 2026-08-30) without adding
        // information the caller needs for files it's told are peripheral
        // and droppable. P0 keeps it — that's the tier the doctrine
        // mandates checking, where the uid is worth the bytes for a
        // follow-up `pixel pack-context`/`pixel impact` call.
        let symbols = if tier == "P0" { e.symbols } else { Vec::new() };
        targets.push(TargetFile {
            path,
            tier: tier.to_string(),
            score: round6(e.score),
            reasons: e.reasons,
            symbols,
        });
    }
    // Present P0 first, then P1, then P2, score order inside each tier
    // (already score-ordered globally; stable sort by tier preserves it).
    targets.sort_by(|a, b| a.tier.cmp(&b.tier));

    // Precision mode: score-gap cutoff. If the top file's score is much
    // higher than the rest, there's a clear winner and the lower files are
    // noise. Two strategies:
    // 1. P0→P1 gap: if last P0 score S, drop P1/P2 below S * (1 - RATIO).
    // 2. Top-file gap: if no P0, compare #1 vs #2. If #1 is > 1/RATIO times
    //    #2, keep only files within RATIO of #1.
    let mut precision_dropped = 0usize;
    if opts.precision_mode && targets.len() > 1 {
        let p0_scores: Vec<f64> = targets
            .iter()
            .filter(|t| t.tier == "P0")
            .map(|t| t.score)
            .collect();
        let threshold = if let Some(&last_p0_score) = p0_scores.last() {
            // P0 present: keep files scoring >= last_p0 * RATIO.
            Some(last_p0_score * SCORE_GAP_RATIO)
        } else {
            // No P0: use top-file gap with the more aggressive secondary ratio.
            let top = targets[0].score;
            let second = targets[1].score;
            if top > 0.0 && second < top * SECONDARY_GAP_RATIO {
                Some(top * SECONDARY_GAP_RATIO)
            } else {
                None
            }
        };
        if let Some(threshold) = threshold {
            let before = targets.len();
            let top_score = targets[0].score;
            // Always keep the top file; drop others below threshold.
            targets.retain(|t| t.score >= threshold || t.score == top_score);
            precision_dropped = before - targets.len();
        }
    }

    // Max-tier filter: drop files above the requested tier.
    let mut tier_dropped = 0usize;
    if let Some(ref max_tier) = opts.max_tier {
        let allow_p1 = max_tier == "P1" || max_tier == "P2";
        let allow_p2 = max_tier == "P2";
        let before = targets.len();
        targets.retain(|t| match t.tier.as_str() {
            "P0" => true,
            "P1" => allow_p1,
            "P2" => allow_p2,
            _ => true,
        });
        tier_dropped = before - targets.len();
    }

    // Envelope + closed-world claim. Three-outcome contract: a complete
    // answer (no cap fired, graph closed), an explicitly-bounded partial
    // answer (lower_bound + every cap NAMED), or the caller's ambiguity
    // path. "Exhaustive" may only be claimed when NOTHING was capped.
    let (graph_lower_bound, unresolved, graph_state) = if inputs.graph_available {
        let env = inputs.envelope.clone().unwrap_or(pixel_graph::Envelope {
            lower_bound: false,
            unresolved_same_name: 0,
        });
        (env.lower_bound, env.unresolved_same_name, "fresh")
    } else {
        (true, 0, "unavailable")
    };
    let mut caps: Vec<String> = inputs.caps.clone();
    if query.keywords_truncated {
        caps.push(format!(
            "task keywords truncated at {MAX_KEYWORDS}; later task words contributed no signal"
        ));
    }
    if p2_dropped > 0 {
        caps.push(format!(
            "P2 tier capped at {p2_cap}: dropped {p2_dropped} graph/cluster-evidenced file(s)"
        ));
    }
    if beyond_limit > 0 {
        caps.push(format!(
            "target list truncated at limit {limit}: {beyond_limit} scored candidate file(s) \
             beyond it"
        ));
    }
    if precision_dropped > 0 {
        caps.push(format!(
            "precision mode: {precision_dropped} low-score P1/P2 file(s) dropped by score-gap cutoff"
        ));
    }
    if tier_dropped > 0 {
        caps.push(format!(
            "max-tier filter: {tier_dropped} file(s) above tier {} dropped",
            opts.max_tier.as_deref().unwrap_or("?")
        ));
    }
    // ANY fired cap makes the list a lower bound — a capped signal cannot
    // support an exhaustive claim.
    let lower_bound = graph_lower_bound || !caps.is_empty();
    let note = if graph_state == "unavailable" {
        "code graph unavailable — lexical signals only; graph-adjacent files may be missing"
            .to_string()
    } else if graph_lower_bound {
        format!(
            "{unresolved} unresolved call site(s) share a matched symbol name; callers beyond this list may exist"
        )
    } else {
        String::new()
    };
    let mut closed_world = String::from(
        "Restrict reads and edits to the files listed. P2 entries are peripheral and droppable. ",
    );
    if lower_bound {
        // Explicitly-bounded partial answer: name every reason the list may
        // be incomplete instead of claiming exhaustiveness.
        closed_world
            .push_str("This list is a bounded partial answer, NOT exhaustive — EXCEPT clauses: ");
        let mut reasons: Vec<String> = Vec::new();
        if !note.is_empty() {
            reasons.push(note.clone());
        }
        reasons.extend(caps.iter().cloned());
        closed_world.push_str(&reasons.join("; "));
        closed_world.push('.');
    } else {
        // Only reachable when no cap fired and the graph closed the world.
        closed_world.push_str("This list is exhaustive for the indexed tree.");
    }

    let signal_hits = json!({
        "filename": s1.len(),
        "symbol": s2.len(),
        "content": s3.len(),
        "graph": s4.len(),
        "cluster": s5.len(),
    });

    TargetsReport {
        task: task.to_string(),
        keywords: query.keywords.clone(),
        exact_tokens: query.exact_tokens.clone(),
        path_tokens: query.path_tokens.clone(),
        targets,
        envelope: json!({
            "lower_bound": lower_bound,
            "graph": graph_state,
            "unresolved_same_name": unresolved,
            "note": note,
            "caps": caps,
        }),
        closed_world,
        stats: json!({
            "files_considered": inputs.all_paths.len(),
            "signal_hits": signal_hits,
            "limit": limit,
        }),
    }
}

fn filename_rank(paths: &[String], keywords: &[String]) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for path in paths {
        let name = path.rsplit('/').next().unwrap_or(path);
        let matched: Vec<String> = keywords
            .iter()
            .filter(|kw| name.contains(*kw))
            .cloned()
            .collect();
        if !matched.is_empty() {
            out.push((path.clone(), matched));
        }
    }
    out
}

fn content_rank(hits: &BTreeMap<String, Vec<(String, u32)>>) -> Vec<(String, Vec<(String, u32)>)> {
    hits.iter()
        .flat_map(|(kw, path_ranks)| {
            path_ranks
                .iter()
                .map(move |(path, rank)| (path.clone(), vec![(kw.clone(), *rank)]))
        })
        .collect()
}

/// Rank paths by matching path tokens (exact or suffix matches).
pub fn path_rank(paths: &[String], tokens: &[String]) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for path in paths {
        for token in tokens {
            if path == token
                || path.ends_with(token)
                || path.rsplit('/').next() == Some(token.as_str())
            {
                results.push((path.clone(), token.clone()));
                break;
            }
        }
    }
    results
}

/// Lexical rank: simple keyword matching on filenames.
pub fn lexical_rank(
    paths: &[String],
    keywords: &[String],
    _language: TaskLanguage,
    _symbol_hits: &[SymbolHit],
    _content_hits: &BTreeMap<String, Vec<(String, u32)>>,
) -> Vec<String> {
    let mut results = Vec::new();
    for path in paths {
        let name = path.rsplit('/').next().unwrap_or(path);
        if keywords.iter().any(|kw| name.contains(kw.as_str())) {
            results.push(path.clone());
        }
    }
    results
}

fn symbol_value(sym: &SymbolRow) -> RankedSymbol {
    RankedSymbol {
        name: sym.name.clone(),
        kind: sym.kind.as_str().to_string(),
        line: Some(sym.start_line as u64),
        uid: sym.uid.clone(),
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pixel_graph::store::{SymbolKind, SymbolRow};

    fn hit(path: &str, name: &str, exact: bool, distinct: usize) -> SymbolHit {
        SymbolHit {
            path: path.to_string(),
            symbols: vec![(
                SymbolRow {
                    id: 1,
                    uid: format!("{path}#{name}#function"),
                    file_id: 1,
                    name: name.to_string(),
                    qualified: name.to_string(),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 5,
                    sig: String::new(),
                },
                name.to_string(),
            )],
            distinct_keywords: distinct,
            exact_name_hit: exact,
        }
    }

    #[test]
    fn tokenize_splits_idents_and_drops_stopwords() {
        let q = tokenize_task("fix loginUser session handling in AuthService").unwrap();
        assert_eq!(
            q.keywords,
            vec!["login", "user", "session", "handling", "auth", "service"]
        );
        assert!(q.exact_tokens.is_empty());
    }

    #[test]
    fn tokenize_extracts_backticked_identifiers() {
        let q = tokenize_task("`login_user` panics on empty token").unwrap();
        assert_eq!(q.exact_tokens, vec!["login_user"]);
        assert_eq!(
            q.keywords,
            vec!["login", "user", "panics", "empty", "token"]
        );
    }

    /// French function words that are English words stay searchable in an
    /// English task: "fix comment parsing" is about comments.
    #[test]
    fn english_task_keeps_words_that_are_french_stopwords() {
        let q = tokenize_task("fix comment parsing for car plus sans rental").unwrap();
        assert_eq!(q.language, TaskLanguage::English);
        assert_eq!(
            q.keywords,
            vec!["comment", "parsing", "car", "plus", "sans", "rental"]
        );
    }

    #[test]
    fn french_task_drops_french_function_words() {
        let q = tokenize_task("Pourquoi la connexion est très lente quand même ?").unwrap();
        assert_eq!(q.language, TaskLanguage::French);
        assert_eq!(q.keywords, vec!["connexion", "lente"]);

        let q = tokenize_task("comment corriger le bouton de connexion").unwrap();
        assert_eq!(q.language, TaskLanguage::French);
        assert_eq!(q.keywords, vec!["corriger", "bouton", "connexion"]);
    }

    #[test]
    fn two_distinct_french_markers_make_a_task_french() {
        use TaskLanguage::{English, French};
        assert_eq!(detect_language("fix the login"), English);
        assert_eq!(detect_language("le login"), English, "one marker");
        assert_eq!(
            detect_language("le le login"),
            English,
            "one distinct marker"
        );
        assert_eq!(detect_language("le login est lent"), French);
        assert_eq!(
            detect_language("bouton connexion casse"),
            French,
            "unambiguous French relation keys count"
        );
        assert_eq!(
            detect_language("client message queue"),
            English,
            "homograph keys are English words and prove nothing"
        );
        assert_eq!(detect_language("LE login EST lent"), French, "case-folded");
        assert_eq!(detect_language(""), English);
    }

    /// Homograph keys expand only in French tasks. English tasks keep their
    /// original meaning; French tasks get the translation.
    #[test]
    fn homograph_keys_expand_only_in_a_french_task() {
        let words = |ws: &[&str]| -> Vec<String> { ws.iter().map(|w| (*w).to_string()).collect() };
        // English: homographs stay unexpanded.
        let english = expand_keywords(
            &words(&["client", "charger", "variable"]),
            TaskLanguage::English,
        );
        assert!(english.is_empty(), "{english:?}");
        // French: homographs expand to English synonyms.
        let french = expand_keywords(
            &words(&["client", "charger", "variable"]),
            TaskLanguage::French,
        );
        assert!(french.contains(&"customer".to_string()));
        assert!(french.contains(&"user".to_string()));
        assert!(french.contains(&"load".to_string()));
        assert!(french.contains(&"fetch".to_string()));
        assert!(french.contains(&"var".to_string()));

        // An unambiguous French key expands whatever the detected language.
        assert_eq!(
            semantic_expand("connexion", TaskLanguage::English),
            ["login", "auth", "connection", "session"]
        );
        // General relations do not depend on the language.
        assert_eq!(
            semantic_expand("cookie", TaskLanguage::French),
            ["session", "auth", "login"]
        );
        assert!(semantic_expand("unrelated", TaskLanguage::French).is_empty());
        // Terms already present are not repeated.
        assert_eq!(
            expand_keywords(&words(&["cookie", "auth"]), TaskLanguage::English),
            ["session", "login", "authenticate", "user"]
        );
    }

    #[test]
    fn diacritics_fold_before_compounds_and_tokens() {
        assert_eq!(
            fold_diacritics("déconnexion paramètre très façade"),
            "deconnexion parametre tres facade"
        );
        let q = tokenize_task("réparer la déconnexion de la base de données et le mot de passe")
            .unwrap();
        assert_eq!(q.language, TaskLanguage::French);
        assert_eq!(
            q.keywords,
            vec!["reparer", "deconnexion", "database", "password"]
        );
        assert_eq!(
            normalize_task_compounds("C# and c# then C++"),
            "csharp and csharp then cpp"
        );
    }

    #[test]
    fn tokenize_all_stopwords_errors() {
        assert!(tokenize_task("fix the code in a file").is_err());
        assert!(tokenize_task("").is_err());
    }

    /// A task that names a file must not turn the extension into a keyword:
    /// `rs` is a word in the filename of every Rust file, so it would hand
    /// the whole tree a filename hit.
    #[test]
    fn tokenize_lifts_file_paths_out_of_the_keywords() {
        let q = tokenize_task(
            "de-flake upgrade_reports_unresponsive_daemon_without_claiming_completion test in upgrade_cli.rs",
        )
        .unwrap();
        assert_eq!(q.path_tokens, vec!["upgrade_cli.rs"]);
        assert!(
            q.keywords.contains(&"upgrade".to_string()),
            "{:?}",
            q.keywords
        );
        assert!(q.keywords.contains(&"cli".to_string()), "{:?}", q.keywords);
        assert!(!q.keywords.contains(&"rs".to_string()), "{:?}", q.keywords);
        // An underscore-joined blob is an identifier candidate, not a
        // symbol name that exists — harmless, and the words still search.
        assert!(
            q.exact_tokens.contains(
                &"upgrade_reports_unresponsive_daemon_without_claiming_completion".to_string()
            ),
            "{:?}",
            q.exact_tokens
        );
    }

    #[test]
    fn tokenize_full_path_keeps_only_the_basename_stem() {
        let q = tokenize_task("fix the timeout in crates/pixel/tests/cli/upgrade_cli.rs").unwrap();
        assert_eq!(q.path_tokens, vec!["crates/pixel/tests/cli/upgrade_cli.rs"]);
        assert_eq!(q.keywords, vec!["timeout", "upgrade", "cli"]);
        // The basename stem is itself an underscore identifier.
        assert_eq!(q.exact_tokens, vec!["upgrade_cli"]);
    }

    /// An unquoted `snake_case` word is a code identifier: the exact-name
    /// probe must see it, because the task named the function itself.
    #[test]
    fn tokenize_unquoted_snake_identifier_is_exact() {
        let q = tokenize_task("cap_message finds the boundary without a mutable loop").unwrap();
        assert_eq!(q.exact_tokens, vec!["cap_message"]);
        assert!(q.keywords.contains(&"cap".to_string()));
        assert!(q.keywords.contains(&"message".to_string()));
        assert!(
            !q.keywords.contains(&"without".to_string()),
            "{:?}",
            q.keywords
        );
    }

    #[test]
    fn tokenize_records_each_named_path_once() {
        let q = tokenize_task("touch guard.rs, then guard.rs again, and rename.rs").unwrap();
        assert_eq!(q.path_tokens, vec!["guard.rs", "rename.rs"]);
        assert!(
            q.keywords.contains(&"rename".to_string()),
            "{:?}",
            q.keywords
        );
    }

    #[test]
    fn tokenize_leaves_unknown_dotted_words_alone() {
        // `flake.lock` has a dot but no source extension: it is not a path,
        // and both words stay in the keyword text.
        let q = tokenize_task("bump the flake.lock pin to stable").unwrap();
        assert!(q.path_tokens.is_empty(), "{:?}", q.path_tokens);
        assert!(
            q.keywords.contains(&"flake".to_string()),
            "{:?}",
            q.keywords
        );
        assert!(q.keywords.contains(&"lock".to_string()), "{:?}", q.keywords);
        // A basename that is nothing but an extension (`x/.rs`) is not a
        // path: the stem would be empty once the dot is removed.
        let q = tokenize_task("remove the stale x/.rs entry").unwrap();
        assert!(q.path_tokens.is_empty(), "{:?}", q.path_tokens);
    }

    #[test]
    fn tokenize_technical_compounds_and_short_keywords() {
        let q_csharp = tokenize_task("add c# support").unwrap();
        assert_eq!(q_csharp.keywords, vec!["csharp"]);

        let q_cpp = tokenize_task("add c++ support").unwrap();
        assert_eq!(q_cpp.keywords, vec!["cpp"]);

        let q_go = tokenize_task("add go support").unwrap();
        assert_eq!(q_go.keywords, vec!["go"]);

        let q_dotnet = tokenize_task("support for .NET and Node.js").unwrap();
        assert_eq!(q_dotnet.keywords, vec!["dotnet", "nodejs"]);

        let q_ruby = tokenize_task("add ruby on rails support").unwrap();
        assert_eq!(q_ruby.keywords, vec!["ruby"]);

        let expanded = expand_keywords(&q_csharp.keywords, q_csharp.language);
        assert!(expanded.contains(&"language".to_string()));
        assert!(expanded.contains(&"extract".to_string()));
    }

    #[test]
    fn multi_signal_file_beats_single_signal_top() {
        // a.rs: filename + content. b.rs: content only (top of that channel).
        let mut content = BTreeMap::new();
        content.insert(
            "login".to_string(),
            vec![
                ("src/b.rs".to_string(), 40u32),
                ("src/login.rs".to_string(), 2u32),
            ],
        );
        let inputs = SignalInputs {
            all_paths: vec!["src/b.rs".into(), "src/login.rs".into()],
            content_hits: content,
            graph_available: true,
            ..Default::default()
        };
        let q = TaskQuery {
            keywords_truncated: false,
            language: TaskLanguage::English,
            exact_tokens: vec![],
            path_tokens: vec![],
            keywords: vec!["login".into()],
        };
        let report = compute_targets("t", &q, inputs, &TargetsOptions::default());
        assert_eq!(report.targets[0].path, "src/login.rs");
        assert_eq!(report.targets[0].tier, "P0"); // filename + content = 2 families
        assert_eq!(report.targets[1].path, "src/b.rs");
        assert_eq!(report.targets[1].tier, "P1"); // content only
    }

    /// Reciprocal rank fusion: each family adds `weight / (k + rank + 1)`,
    /// so rank 0 of a family scores `w / 61`, rank 1 `w / 62`, and a file
    /// hit by two families sums both. The tiers, the P0 score gap and the
    /// cross-family ordering all sit on this formula.
    #[test]
    fn fused_score_is_the_sum_of_reciprocal_ranks_per_family() {
        let inputs = SignalInputs {
            all_paths: vec!["src/a.rs".into(), "src/b.rs".into(), "src/zz.rs".into()],
            cluster_neighbors: vec![
                ("src/zz.rs".into(), "same cluster".into()),
                ("src/b.rs".into(), "same cluster".into()),
            ],
            graph_neighbors: vec![("src/b.rs".into(), "calls".into())],
            graph_available: true,
            ..Default::default()
        };
        let q = TaskQuery {
            keywords_truncated: false,
            language: TaskLanguage::English,
            exact_tokens: vec![],
            path_tokens: vec![],
            keywords: vec!["nothing".into()],
        };
        let report = compute_targets("t", &q, inputs, &TargetsOptions::default());
        let score = |path: &str| {
            report
                .targets
                .iter()
                .find(|t| t.path == path)
                .unwrap_or_else(|| panic!("{path} missing from {:?}", report.targets))
                .score
        };
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(
            close(score("src/zz.rs"), W_CLUSTER_F / (RRF_K + 1.0)),
            "{}",
            score("src/zz.rs")
        );
        assert!(
            close(
                score("src/b.rs"),
                W_GRAPH_F / (RRF_K + 1.0) + W_CLUSTER_F / (RRF_K + 2.0)
            ),
            "{}",
            score("src/b.rs")
        );
        assert_eq!(report.targets[0].path, "src/b.rs");
    }

    #[test]
    fn exact_name_hit_is_p0_and_cluster_only_is_p2() {
        let inputs = SignalInputs {
            all_paths: vec!["src/a.rs".into(), "src/z.rs".into()],
            symbol_hits: vec![hit("src/a.rs", "login_user", true, 1)],
            cluster_neighbors: vec![("src/z.rs".into(), "same cluster 'auth'".into())],
            graph_available: true,
            ..Default::default()
        };
        let q = TaskQuery {
            keywords_truncated: false,
            language: TaskLanguage::English,
            exact_tokens: vec!["login_user".into()],
            path_tokens: vec![],
            keywords: vec!["login".into(), "user".into()],
        };
        let report = compute_targets("t", &q, inputs, &TargetsOptions::default());
        let a = report
            .targets
            .iter()
            .find(|t| t.path == "src/a.rs")
            .unwrap();
        assert_eq!(a.tier, "P0");
        assert!(
            a.reasons
                .iter()
                .any(|r| r.contains("defines symbol `login_user`"))
        );
        let z = report
            .targets
            .iter()
            .find(|t| t.path == "src/z.rs")
            .unwrap();
        assert_eq!(z.tier, "P2");
    }

    /// An explicit path named in the task is primary, and it does not demote
    /// another file with strong lexical evidence: both stay P0. A graph-only
    /// neighbor stays peripheral.
    #[test]
    fn explicit_path_is_p0_without_demoting_lexical_targets() {
        let mut content = BTreeMap::new();
        content.insert(
            "daemon".to_string(),
            vec![("crates/pixel-daemon/src/daemon.rs".to_string(), 50u32)],
        );
        let inputs = SignalInputs {
            all_paths: vec![
                "crates/pixel/tests/cli/upgrade_cli.rs".into(),
                "crates/pixel-daemon/src/daemon.rs".into(),
                "crates/pixel/src/main.rs".into(),
            ],
            content_hits: content,
            symbol_hits: vec![hit(
                "crates/pixel-daemon/src/daemon.rs",
                "run_daemon",
                false,
                2,
            )],
            graph_neighbors: vec![(
                "crates/pixel/src/main.rs".into(),
                "callee of `run_daemon`".into(),
            )],
            path_hits: vec![(
                "crates/pixel/tests/cli/upgrade_cli.rs".into(),
                "upgrade_cli.rs".into(),
            )],
            graph_available: true,
            ..Default::default()
        };
        let q = TaskQuery {
            keywords_truncated: false,
            language: TaskLanguage::English,
            exact_tokens: vec![],
            path_tokens: vec!["upgrade_cli.rs".into()],
            keywords: vec!["daemon".into()],
        };
        let report = compute_targets("t", &q, inputs, &TargetsOptions::default());
        let tier = |path: &str| {
            report
                .targets
                .iter()
                .find(|t| t.path == path)
                .unwrap()
                .tier
                .clone()
        };
        assert_eq!(tier("crates/pixel/tests/cli/upgrade_cli.rs"), "P0");
        assert_eq!(tier("crates/pixel-daemon/src/daemon.rs"), "P0");
        assert_eq!(tier("crates/pixel/src/main.rs"), "P2");
        assert!(
            report
                .targets
                .iter()
                .find(|t| t.path.ends_with("upgrade_cli.rs"))
                .unwrap()
                .reasons
                .iter()
                .any(|r| r == "path match: upgrade_cli.rs")
        );
    }

    /// A graph neighbor plus a filename coincidence is not primary evidence:
    /// P0 needs two lexical families, not merely two families.
    #[test]
    fn filename_plus_graph_neighbor_is_p1_not_p0() {
        let inputs = SignalInputs {
            all_paths: vec!["src/daemon.rs".into(), "src/daemon_client.rs".into()],
            symbol_hits: vec![hit("src/daemon.rs", "run_daemon", false, 1)],
            graph_neighbors: vec![(
                "src/daemon_client.rs".into(),
                "callee of `run_daemon`".into(),
            )],
            graph_available: true,
            ..Default::default()
        };
        let q = TaskQuery {
            keywords_truncated: false,
            language: TaskLanguage::English,
            exact_tokens: vec![],
            path_tokens: vec![],
            keywords: vec!["daemon".into()],
        };
        let report = compute_targets("t", &q, inputs, &TargetsOptions::default());
        let tier = |path: &str| {
            report
                .targets
                .iter()
                .find(|t| t.path == path)
                .unwrap()
                .tier
                .clone()
        };
        assert_eq!(tier("src/daemon.rs"), "P0", "filename + symbol");
        assert_eq!(tier("src/daemon_client.rs"), "P1", "filename + graph");
    }

    /// A path match past the P0 cap is still primary: P1, never the P2
    /// bucket graph-only evidence falls into.
    #[test]
    fn path_hit_past_the_p0_cap_stays_p1() {
        let path_hits: Vec<(String, String)> = (0..6)
            .map(|i| (format!("src/named_{i}.rs"), format!("named_{i}.rs")))
            .collect();
        let inputs = SignalInputs {
            all_paths: path_hits.iter().map(|(path, _)| path.clone()).collect(),
            path_hits,
            graph_available: true,
            ..Default::default()
        };
        let q = TaskQuery {
            keywords_truncated: false,
            language: TaskLanguage::English,
            exact_tokens: vec![],
            path_tokens: vec![],
            keywords: vec![],
        };
        let report = compute_targets("t", &q, inputs, &TargetsOptions::default());
        let tier = |path: &str| {
            report
                .targets
                .iter()
                .find(|t| t.path == path)
                .unwrap()
                .tier
                .clone()
        };
        assert_eq!(tier("src/named_0.rs"), "P0");
        assert_eq!(tier("src/named_5.rs"), "P1");
    }

    #[test]
    fn limit_is_hard_and_p2_capped() {
        let all: Vec<String> = (0..30).map(|i| format!("src/login_{i:02}.rs")).collect();
        let cluster: Vec<(String, String)> = (0..10)
            .map(|i| (format!("src/extra_{i:02}.rs"), "same cluster 'x'".into()))
            .collect();
        let inputs = SignalInputs {
            all_paths: all.clone(),
            cluster_neighbors: cluster,
            graph_available: true,
            ..Default::default()
        };
        let q = TaskQuery {
            keywords_truncated: false,
            language: TaskLanguage::English,
            exact_tokens: vec![],
            path_tokens: vec![],
            keywords: vec!["login".into()],
        };
        let report = compute_targets(
            "t",
            &q,
            inputs,
            &TargetsOptions {
                limit: 8,
                max_tier: None,
                precision_mode: false,
            },
        );
        assert_eq!(report.targets.len(), 8);
        let p2 = report.targets.iter().filter(|t| t.tier == "P2").count();
        assert!(p2 <= 2); // ceil(8/4)
    }

    #[test]
    fn deterministic_output() {
        let mk = || {
            let mut content = BTreeMap::new();
            content.insert(
                "auth".to_string(),
                vec![
                    ("src/x.rs".to_string(), 3u32),
                    ("src/y.rs".to_string(), 3u32),
                ],
            );
            SignalInputs {
                all_paths: vec!["src/x.rs".into(), "src/y.rs".into()],
                content_hits: content,
                graph_available: true,
                ..Default::default()
            }
        };
        let q = TaskQuery {
            keywords_truncated: false,
            language: TaskLanguage::English,
            exact_tokens: vec![],
            path_tokens: vec![],
            keywords: vec!["auth".into()],
        };
        let a = serde_json::to_string(&compute_targets("t", &q, mk(), &TargetsOptions::default()))
            .unwrap();
        let b = serde_json::to_string(&compute_targets("t", &q, mk(), &TargetsOptions::default()))
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn graph_unavailable_sets_lower_bound() {
        let inputs = SignalInputs {
            all_paths: vec!["src/login.rs".into()],
            graph_available: false,
            ..Default::default()
        };
        let q = TaskQuery {
            keywords_truncated: false,
            language: TaskLanguage::English,
            exact_tokens: vec![],
            path_tokens: vec![],
            keywords: vec!["login".into()],
        };
        let report = compute_targets("t", &q, inputs, &TargetsOptions::default());
        assert_eq!(report.envelope["lower_bound"], true);
        assert_eq!(report.envelope["graph"], "unavailable");
        assert!(report.closed_world.contains("EXCEPT"));
    }

    #[test]
    fn rrf_fuse_sums_weighted_ranks_and_breaks_ties_by_path() {
        let a = vec!["x.rs".to_string(), "y.rs".to_string()];
        let b = vec!["y.rs".to_string(), "z.rs".to_string()];
        let fused = rrf_fuse(&[(&a, 3.0), (&b, 1.5)], RRF_K);
        // y.rs appears in both lists → strictly higher score than x.rs/z.rs.
        assert_eq!(fused[0].0, "y.rs");
        assert!(fused[0].1 > fused[1].1);
        // Deterministic tie-break: x.rs before z.rs at equal score.
        assert_eq!(fused[1].0, "x.rs");
        assert_eq!(fused[2].0, "z.rs");
    }

    fn bm25_doc(path: &str, tfs: &[u32], len: u32) -> Bm25Doc {
        Bm25Doc {
            path: path.into(),
            term_freqs: tfs.to_vec(),
            len,
        }
    }

    /// `bm25_scores` is the formula itself, aligned with the documents:
    /// two docs over one term, df 1 of 2 (idf ln(1 + 1.5/1.5) = ln 2),
    /// lengths 10 and 30 (avgdl 20).
    #[test]
    fn bm25_scores_are_the_okapi_formula_aligned_with_the_docs() {
        let terms = vec!["ledger".to_string()];
        let docs = vec![bm25_doc("a.rs", &[2], 10), bm25_doc("b.rs", &[0], 30)];
        let scores = bm25_scores(&terms, &docs).expect("signal present");
        let norm = 1.0 - BM25_B + BM25_B * 0.5;
        let expected = 2.0f64.ln() * (2.0 * (BM25_K1 + 1.0)) / (2.0 + BM25_K1 * norm);
        assert_eq!(scores.len(), 2);
        assert!(
            (scores[0] - expected).abs() < 1e-12,
            "{scores:?} vs {expected}"
        );
        assert_eq!(scores[1], 0.0);
        assert_eq!(bm25_fixed(1.234_567_89), 1_234_567);
        assert_eq!(bm25_fixed(0.0), 0);
        assert!(bm25_scores(&[], &docs).is_none());
        assert!(bm25_scores(&terms, &[]).is_none());
        assert!(bm25_scores(&terms, &[bm25_doc("z.rs", &[0], 5)]).is_none());
        assert!(bm25_scores(&terms, &[bm25_doc("z.rs", &[1], 0)]).is_none());
    }

    #[test]
    fn bm25_idf_beats_raw_count() {
        // terms[0]="fn" (in both docs, common), terms[1]="ledger" (rare).
        // a.rs has 40 hits of the common term; b.rs has 3 of the rare one.
        // Raw density ranks a.rs first; BM25 must rank b.rs first.
        let terms = vec!["fn".to_string(), "ledger".to_string()];
        let docs = vec![
            bm25_doc("a.rs", &[40, 0], 100),
            bm25_doc("b.rs", &[1, 3], 100),
        ];
        let ranked = bm25_rank(&terms, &docs).expect("signal present");
        assert_eq!(
            ranked[0], "b.rs",
            "rare-term match must outrank common-term volume"
        );
    }

    #[test]
    fn bm25_saturates_term_frequency() {
        // Same single term, same length: 5 hits vs 50 hits. The 50-hit doc
        // still wins, but by far less than 10x — that is saturation.
        let terms = vec!["ledger".to_string()];
        let few = vec![
            bm25_doc("few.rs", &[5], 100),
            bm25_doc("many.rs", &[50], 100),
        ];
        let ranked = bm25_rank(&terms, &few).expect("signal present");
        assert_eq!(ranked[0], "many.rs");
        // Score ratio must be well under the 10x raw-count ratio.
        let score = |tf: f64| {
            let norm = 1.0 - BM25_B + BM25_B * 1.0;
            (tf * (BM25_K1 + 1.0)) / (tf + BM25_K1 * norm)
        };
        assert!(
            score(50.0) / score(5.0) < 2.0,
            "tf must saturate, not scale linearly"
        );
    }

    #[test]
    fn bm25_normalizes_document_length() {
        // Equal term frequency, very different lengths: the short, dense doc
        // must win. Raw count would tie them.
        let terms = vec!["ledger".to_string()];
        let docs = vec![
            bm25_doc("long.rs", &[3], 1000),
            bm25_doc("short.rs", &[3], 50),
        ];
        let ranked = bm25_rank(&terms, &docs).expect("signal present");
        assert_eq!(
            ranked[0], "short.rs",
            "shorter doc with same tf must rank first"
        );
    }

    #[test]
    fn bm25_returns_none_without_signal() {
        let terms = vec!["ledger".to_string()];
        assert!(
            bm25_rank(&[], &[bm25_doc("a.rs", &[], 10)]).is_none(),
            "no terms"
        );
        assert!(bm25_rank(&terms, &[]).is_none(), "no docs");
        // Every doc scores zero -> caller must fall back, not get path order.
        let zero = vec![bm25_doc("a.rs", &[0], 10), bm25_doc("b.rs", &[0], 10)];
        assert!(bm25_rank(&terms, &zero).is_none(), "all-zero pool");
        // Zero-length pool is degenerate (avgdl undefined).
        let empty_len = vec![bm25_doc("a.rs", &[1], 0)];
        assert!(bm25_rank(&terms, &empty_len).is_none(), "zero total length");
    }

    #[test]
    fn bm25_is_deterministic_and_breaks_ties_by_path() {
        let terms = vec!["ledger".to_string()];
        let docs = vec![
            bm25_doc("z.rs", &[3], 100),
            bm25_doc("a.rs", &[3], 100),
            bm25_doc("m.rs", &[3], 100),
        ];
        let first = bm25_rank(&terms, &docs).expect("signal present");
        assert_eq!(
            first,
            vec!["a.rs", "m.rs", "z.rs"],
            "identical scores tie by path asc"
        );
        for _ in 0..5 {
            assert_eq!(
                bm25_rank(&terms, &docs).unwrap(),
                first,
                "must be deterministic"
            );
        }
    }

    #[test]
    fn idf_downweights_common_content_terms() {
        // Two keywords, both matching the same two files. The rare term
        // 'ledger' matches only 1 of the 2 files (df=1) → high IDF; the
        // common term 'fn' matches both (df=2) → low IDF. Even when the
        // common term has a higher raw count, the rare term's file must rank
        // first because IDF dominates.
        let mut content = BTreeMap::new();
        content.insert(
            "fn".to_string(),
            vec![
                ("src/a.rs".to_string(), 40u32),
                ("src/b.rs".to_string(), 1u32),
            ],
        );
        content.insert("ledger".to_string(), vec![("src/b.rs".to_string(), 2u32)]);
        let ranked = content_rank(&content);
        // 'b.rs' hits the rare, high-IDF term 'ledger'; 'a.rs' only hits the
        // common 'fn'. IDF must push b.rs ahead despite a.rs's higher raw count.
        assert_eq!(ranked[0].0, "src/b.rs");
        assert_eq!(ranked[1].0, "src/a.rs");
    }
}

/// Fuse multiple ranked lists using weighted Reciprocal Rank Fusion.
/// Weights are f64 so callers can tune relative signal importance.
pub fn rrf_fuse(lists: &[(&[String], f64)], rrf_k: f64) -> Vec<(String, f64)> {
    use std::collections::HashMap;
    let mut fused: HashMap<String, f64> = HashMap::new();
    for (list, weight) in lists {
        for (rank, path) in list.iter().enumerate() {
            *fused.entry(path.clone()).or_insert(0.0) += weight / (rrf_k + rank as f64);
        }
    }
    let mut result: Vec<_> = fused.into_iter().collect();
    result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then_with(|| a.0.cmp(&b.0)));
    result
}

#[cfg(test)]
mod contract_tests;
