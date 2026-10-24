// SPDX-FileCopyrightText: The RTO contributors
// SPDX-License-Identifier: MIT

use super::*;

const NONE: [&str; 0] = [];
const SECOND: Duration = Duration::from_secs(1);

/// The old scorer parameter is accepted only by legacy test helpers; brief
/// execution no longer consults it.
fn try_start_with<F>(
    prompt: &str,
    window: Duration,
    open: F,
    _model: Model,
    around: Around,
) -> Result<Pending, Declined>
where
    F: FnOnce(Instant) -> Box<dyn Evidence> + Send + 'static,
{
    super::try_start(prompt, window, open, around)
}

/// [`try_start_with`] without session context.
fn start_with<F>(prompt: &str, window: Duration, open: F) -> Option<Pending>
where
    F: FnOnce(Instant) -> Box<dyn Evidence> + Send + 'static,
{
    try_start_with(prompt, window, open, relevance::judge, Around::default()).ok()
}

fn hit(path: &str, line: u64) -> FileHit {
    FileHit {
        path: path.into(),
        line,
    }
}

fn found(hits: Vec<FileHit>) -> Found {
    Found {
        hits,
        capped: false,
    }
}

fn rhit(path: &str, line: u64, text: &str) -> RichHit {
    RichHit {
        path: path.into(),
        line,
        text: Some(text.into()),
    }
}

fn rhit_no_text(path: &str, line: u64) -> RichHit {
    RichHit {
        path: path.into(),
        line,
        text: None,
    }
}

fn rfound(hits: Vec<RichHit>) -> RichFound {
    RichFound {
        hits,
        capped: false,
    }
}

fn history(sha: &str, subject: &str) -> HistoryHit {
    HistoryHit {
        sha: sha.into(),
        subject: subject.into(),
        ..HistoryHit::default()
    }
}

fn symbol(path: &str, name: &str) -> SymbolHit {
    SymbolHit {
        uid: format!("{path}#{name}#function"),
        name: name.into(),
        kind: "function".into(),
        path: path.into(),
        start_line: 3,
        end_line: 9,
    }
}

fn caller(path: &str, via: &str, line: u64) -> CallerHit {
    CallerHit {
        path: path.into(),
        via: via.into(),
        line,
    }
}

fn anchors(text: &str) -> Vec<String> {
    Anchors::from_text(text).0
}

#[test]
fn anchors_should_prefer_backticked_names_then_paths_and_put_paths_last() {
    assert_eq!(
        anchors("`handleError` in packages/ui/handleError.ts is renamed to reportError"),
        ["handleError", "reportError", "packages/ui/handleError.ts"]
    );
}

#[test]
fn anchors_should_take_cased_words_and_ignore_plain_prose() {
    assert_eq!(
        anchors("why does fetchUser call retry_with_backoff and Foo::bar twice"),
        ["fetchUser", "retry_with_backoff", "Foo::bar"]
    );
    assert_eq!(anchors("where is the parser and how does it work"), NONE);
}

#[test]
fn anchors_should_keep_a_quoted_word_only_when_it_looks_like_code() {
    assert_eq!(anchors("the 'render_page' helper"), ["render_page"]);
    assert_eq!(anchors("it's 'fine' to use \"this\""), NONE);
    assert_eq!(anchors("call `run` now"), ["run"]);
}

#[test]
fn anchors_should_drop_call_parentheses_and_sentence_marks() {
    assert_eq!(anchors("rename `load_config()`."), ["load_config"]);
    assert_eq!(anchors("see `run()`"), ["run"]);
    assert_eq!(anchors("see `src/main.rs`."), ["src/main.rs"]);
    assert_eq!(
        anchors("open ./crates/pixel/src/lib.rs."),
        ["crates/pixel/src/lib.rs"]
    );
}

#[test]
fn anchors_should_not_take_prose_slashes_or_extensionless_paths() {
    assert_eq!(anchors("client/server and/or docs/guide"), NONE);
}

#[test]
fn anchors_should_stop_at_four_and_skip_duplicates() {
    assert_eq!(
        anchors("aaaa_bbbb aaaa_bbbb cccc_dddd eeee_ffff gggg_hhhh iiii_jjjj"),
        ["aaaa_bbbb", "cccc_dddd", "eeee_ffff", "gggg_hhhh"]
    );
}

#[test]
fn anchors_should_skip_a_token_longer_than_the_cap() {
    let long = format!("{}_x", "a".repeat(MAX_ANCHOR_CHARS - 1));
    assert_eq!(anchors(&long), NONE);
    let fits = format!("{}_x", "a".repeat(MAX_ANCHOR_CHARS - 2));
    assert_eq!(anchors(&fits), [fits.as_str()]);
}

#[test]
fn anchors_should_ignore_a_cased_word_below_the_minimum_length() {
    assert_eq!(anchors("a_b and c_de and fgH"), NONE);
    assert_eq!(anchors("x_yzw"), ["x_yzw"]);
}

#[test]
fn search_term_and_symbol_name_should_use_the_identifier_then_the_path_stem() {
    let named = Anchors::from_text("rename Foo::bar in src/foo.rs");
    assert_eq!(named.search_term().as_deref(), Some("Foo::bar"));
    assert_eq!(named.symbol_name().as_deref(), Some("bar"));
    let pathed = Anchors::from_text("who uses crates/pixel/src/handler.rs");
    assert_eq!(pathed.search_term().as_deref(), Some("handler"));
    assert_eq!(pathed.symbol_name().as_deref(), Some("handler"));
    assert_eq!(Anchors::default().search_term(), None);
    assert_eq!(Anchors::default().symbol_name(), None);
}

#[test]
fn change_intent_should_follow_the_stems_and_ignore_a_literal_lookup() {
    for prompt in [
        "rename handleError to reportError",
        "who are the callers of load_config",
        "what depends on the parser",
        "is it safe to delete this",
        "this refactor would break clients",
        "blast radius of the change",
        "mark it as Deprecated",
        "what is the impact on tests",
        "does it affect the cache",
        "remove the flag",
    ] {
        assert!(has_change_intent(prompt), "{prompt}");
    }
    for prompt in [
        "where is fetchUser defined",
        "how does the retry logic work",
        "show me the cache module",
    ] {
        assert!(!has_change_intent(prompt), "{prompt}");
    }
}

#[test]
fn lookup_only_should_keep_usage_blocks_for_edits_and_explicit_references() {
    for (prompt, lookup_only) in [
        ("Where is `toolRoundLimit` defined?", true),
        ("rename `toolRoundLimit` to roundLimit", false),
        ("What should be added to `toolRoundLimit`?", false),
        ("adding support around `toolRoundLimit`", false),
        ("What does `toolRoundLimit` do and who uses it?", false),
        ("show usages of `toolRoundLimit`", false),
        ("What references `toolRoundLimit`?", false),
        ("Qui utilise `toolRoundLimit`?", false),
        ("renomme `toolRoundLimit` en roundLimit", false),
    ] {
        let plan = Plan::from_prompt(prompt).unwrap();
        assert_eq!(is_lookup_only(&plan), lookup_only, "{prompt}");
    }
}

#[test]
fn plan_should_decline_a_prompt_that_asks_nothing_about_code() {
    assert_eq!(Plan::from_prompt("ok thanks"), None);
    assert_eq!(Plan::from_prompt("commit and push this"), None);
    // Harness envelopes are filtered before this typed-text planning helper.
    assert!(Plan::from_prompt("fix the hook: a <task-notification> reaches `fetchUser`").is_some());
    assert!(
        Plan::from_prompt("fix the hook: a <realtime_delegation> reaches `fetchUser`").is_some()
    );
    assert_eq!(
        Plan::from_prompt("<pasted_content id=1>fetchUser retry_count</pasted_content> thanks"),
        None
    );
}

#[test]
fn plan_should_carry_anchors_intent_and_concept_words() {
    let plan = Plan::from_prompt("who are the callers of `fetchUser`?").unwrap();
    assert_eq!(plan.anchors.0, ["fetchUser"]);
    assert!(plan.change_intent);
    assert_eq!(plan.concept.as_deref(), Some("callers fetchuser"));
}

#[test]
fn concept_phrase_should_skip_short_common_and_repeated_words_and_bound_the_chars() {
    // Every significant word rides, deduplicated — nothing is clipped
    // to a word count anymore.
    assert_eq!(
        concept_phrase("how does the session cache expire stale entries after logout today cache"),
        Some("session cache expire stale entries after logout today".to_string())
    );
    let long = format!(
        "find the thing {} tail",
        (0..40)
            .map(|n| format!("paddingword{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let phrase = concept_phrase(&long).expect("a long prompt still has a phrase");
    assert!(phrase.len() <= MAX_CONCEPT_CHARS, "{phrase}");
    assert!(!phrase.ends_with(' '), "{phrase}");
    assert!(phrase.starts_with("find"), "{phrase}");
    assert_eq!(concept_phrase("it is a to do"), None);
}

#[test]
fn generated_files_should_be_data_dumps_and_build_output() {
    for path in [
        "package.json",
        "Cargo.lock",
        "DATA.JSON",
        "output/report.ts",
        "apps/web/dist/main.js",
        "node_modules/pkg/index.js",
    ] {
        assert!(is_generated(path), "{path}");
    }
    for path in [
        "src/json.rs",
        "src/outputs.rs",
        "src/dist_calc.rs",
        "crates/jsonlike/lib.rs",
        "src/lockfile.rs",
    ] {
        assert!(!is_generated(path), "{path}");
    }
}

#[test]
fn pick_uid_should_choose_the_declaration_in_the_named_file() {
    let hits = [
        symbol("apps/web/handleError.ts", "handleError"),
        symbol("packages/ui/handleError.ts", "handleError"),
    ];
    let (pick, note) = pick_uid(&hits, "handleError", &["packages/ui/handleError.ts"]);
    assert_eq!(pick, Some(&hits[1]));
    assert_eq!(note, None);
}

#[test]
fn pick_uid_should_take_a_single_candidate_without_a_note() {
    let hits = [symbol("a.ts", "go")];
    assert_eq!(pick_uid(&hits, "go", &[]), (Some(&hits[0]), None));
    assert_eq!(
        pick_uid(&hits, "go", &["elsewhere/b.ts"]),
        (Some(&hits[0]), None)
    );
}

#[test]
fn pick_uid_should_take_the_first_of_several_and_say_so() {
    let hits = [symbol("a.ts", "go"), symbol("b.ts", "go")];
    let (pick, note) = pick_uid(&hits, "go", &[]);
    assert_eq!(pick, Some(&hits[0]));
    assert_eq!(
        note.as_deref(),
        Some("find-symbol go: 2 candidates, took first")
    );
}

#[test]
fn pick_uid_should_not_guess_when_a_path_anchor_separates_nothing() {
    let hits = [symbol("a.ts", "go"), symbol("b.ts", "go")];
    let (pick, note) = pick_uid(&hits, "go", &["lib/elsewhere.ts"]);
    assert_eq!(pick, None);
    assert_eq!(
        note.as_deref(),
        Some("find-symbol go: 2 candidates, none in the named file, bare name used")
    );
}

#[test]
fn pick_uid_should_fall_back_to_the_bare_name_when_nothing_is_declared() {
    let (pick, note) = pick_uid(&[], "go", &["a.ts"]);
    assert_eq!(pick, None);
    assert_eq!(
        note.as_deref(),
        Some("find-symbol go: no uid, bare name used")
    );
}

#[test]
fn pick_uid_should_match_a_path_anchor_by_whole_components_only() {
    let hits = [
        symbol("src/xhandler.ts", "go"),
        symbol("lib/handler.ts", "go"),
    ];
    let (pick, _) = pick_uid(&hits, "go", &["handler.ts"]);
    assert_eq!(pick, Some(&hits[1]));
}

/// A scripted source: each operation logs its call, pauses, and returns
/// its canned answer.
struct Fake {
    files: Result<Found, String>,
    /// A `files_matching` answer carrying matched text; when `None`
    /// the fake wraps `files` like the trait's default does.
    rows: Option<Result<RichFound, String>>,
    concept: Result<Found, String>,
    concept_rows: Option<Result<RichFound, String>>,
    symbols: Result<Vec<SymbolHit>, String>,
    callers: Result<Vec<CallerHit>, String>,
    context: Result<(String, Vec<String>), String>,
    test_files: Result<(Vec<String>, Vec<String>), String>,
    flow: Result<Flow, String>,
    skeleton: Result<Vec<SymbolHit>, String>,
    status: Result<StatusProbe, String>,
    history: Result<(Vec<HistoryHit>, Vec<String>), String>,
    relevance: Result<RelevanceAnswer, String>,
    meaning: Result<Vec<MeaningHit>, String>,
    /// How long the meaning search takes, on top of `pause`.
    meaning_pause: Duration,
    /// The plain-word leads.
    words: Result<Vec<MeaningHit>, String>,
    /// The semantic hint the fake advertises, when it models a warm index.
    semantic: Option<String>,
    /// Source line a `line_at` read answers with, when the fake models
    /// source at all.
    source: Option<String>,
    /// Files a `lines_at` read answers from, by path.
    sources: Vec<(String, Vec<String>)>,
    pause: Duration,
    log: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    fn new() -> Self {
        Self {
            files: Ok(Found::default()),
            rows: None,
            concept: Ok(Found::default()),
            concept_rows: None,
            symbols: Ok(Vec::new()),
            callers: Ok(Vec::new()),
            context: Err("pack-context unsupported".into()),
            test_files: Err("caller tests unsupported".into()),
            flow: Err("flow unsupported".into()),
            skeleton: Err("list-signatures unsupported".into()),
            status: Err("status unsupported".into()),
            history: Err("history unsupported".into()),
            relevance: Err("unavailable".into()),
            meaning: Err("unavailable".into()),
            meaning_pause: Duration::ZERO,
            words: Err("unavailable".into()),
            semantic: None,
            source: None,
            sources: Vec::new(),
            pause: Duration::ZERO,
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn note(&self, entry: String) {
        std::thread::sleep(self.pause);
        self.log.lock().unwrap().push(entry);
    }

    fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Evidence for Fake {
    fn files_with(&self, anchor: &str, _: Instant) -> Result<Found, String> {
        self.note(format!("files_with {anchor}"));
        self.files.clone()
    }
    fn files_matching(&self, anchor: &str, _: Instant) -> Result<RichFound, String> {
        self.note(format!("files_with {anchor}"));
        self.rows
            .clone()
            .unwrap_or_else(|| self.files.clone().map(RichFound::from))
    }
    fn concept(&self, phrase: &str, _: Instant) -> Result<Found, String> {
        self.note(format!("concept {phrase}"));
        self.concept.clone()
    }
    fn concept_matching(&self, phrase: &str, _: Instant) -> Result<RichFound, String> {
        self.note(format!("concept {phrase}"));
        self.concept_rows
            .clone()
            .unwrap_or_else(|| self.concept.clone().map(RichFound::from))
    }
    fn symbols(&self, name: &str, _: Instant) -> Result<Vec<SymbolHit>, String> {
        self.note(format!("symbols {name}"));
        self.symbols.clone()
    }
    fn callers(&self, target: &str, _: Instant) -> Result<Vec<CallerHit>, String> {
        self.note(format!("callers {target}"));
        self.callers.clone()
    }
    fn context(
        &self,
        hit: &SymbolHit,
        budget_tokens: usize,
        _: Instant,
    ) -> Result<(String, Vec<String>), String> {
        self.note(format!("context {} budget {budget_tokens}", hit.uid));
        self.context.clone()
    }
    fn test_files(&self, uid: &str, _: Instant) -> Result<(Vec<String>, Vec<String>), String> {
        self.note(format!("test_files {uid}"));
        self.test_files.clone()
    }
    fn flow(&self, from: &str, to: &str, budget_ms: u64, _: Instant) -> Result<Flow, String> {
        self.note(format!("flow {from} -> {to} ({budget_ms}ms)"));
        self.flow.clone()
    }
    fn skeleton(&self, file: &str, _: Instant) -> Result<Vec<SymbolHit>, String> {
        self.note(format!("skeleton {file}"));
        self.skeleton.clone()
    }
    fn status(&self, _: Instant) -> Result<StatusProbe, String> {
        self.note("status".to_string());
        self.status.clone()
    }
    fn history(
        &self,
        phrase: &str,
        limit: usize,
        _: Instant,
    ) -> Result<(Vec<HistoryHit>, Vec<String>), String> {
        self.note(format!("history {phrase} limit {limit}"));
        self.history.clone()
    }
    fn relevance(&self, typed: &str, _: Instant) -> Result<RelevanceAnswer, String> {
        self.note(format!("relevance {typed}"));
        self.relevance.clone()
    }
    fn meaning(&self, query: &str, limit: usize, _: Instant) -> Result<Vec<MeaningHit>, String> {
        self.note(format!("meaning {query} limit {limit}"));
        std::thread::sleep(self.meaning_pause);
        self.meaning.clone()
    }
    fn word_leads(&self, _: &str, _: usize, _: Instant) -> Result<Vec<MeaningHit>, String> {
        self.words.clone()
    }
    fn semantic_hint(&self, phrase: &str) -> Option<String> {
        self.note(format!("semantic_hint {phrase}"));
        self.semantic.clone()
    }
    fn line_at(&self, path: &str, line: u64, _: Instant) -> Result<String, String> {
        self.note(format!("line_at {path}:{line}"));
        self.source.clone().ok_or_else(|| "no source".to_string())
    }
    fn lines_at(
        &self,
        path: &str,
        start: u64,
        end: u64,
        _: Instant,
    ) -> Result<Vec<String>, String> {
        self.note(format!("lines_at {path}:{start}-{end}"));
        let (_, all) = self
            .sources
            .iter()
            .find(|(name, _)| name == path)
            .ok_or_else(|| "no source".to_string())?;
        Ok(all
            .iter()
            .skip(usize::try_from(start.saturating_sub(1)).unwrap_or(0))
            .take(usize::try_from(end + 1 - start).unwrap_or(0))
            .cloned()
            .collect())
    }
}

/// A `Fake` the test keeps a handle to after the brief took the source.
struct Shared(Arc<Fake>);

impl Evidence for Shared {
    fn files_with(&self, anchor: &str, deadline: Instant) -> Result<Found, String> {
        self.0.files_with(anchor, deadline)
    }
    fn concept(&self, phrase: &str, deadline: Instant) -> Result<Found, String> {
        self.0.concept(phrase, deadline)
    }
    fn symbols(&self, name: &str, deadline: Instant) -> Result<Vec<SymbolHit>, String> {
        self.0.symbols(name, deadline)
    }
    fn callers(&self, target: &str, deadline: Instant) -> Result<Vec<CallerHit>, String> {
        self.0.callers(target, deadline)
    }
    fn relevance(&self, typed: &str, deadline: Instant) -> Result<RelevanceAnswer, String> {
        self.0.relevance(typed, deadline)
    }
    fn meaning(
        &self,
        query: &str,
        limit: usize,
        deadline: Instant,
    ) -> Result<Vec<MeaningHit>, String> {
        self.0.meaning(query, limit, deadline)
    }
    fn word_leads(
        &self,
        query: &str,
        limit: usize,
        deadline: Instant,
    ) -> Result<Vec<MeaningHit>, String> {
        self.0.word_leads(query, limit, deadline)
    }
}

fn chain(prompt: &str, fake: &Fake, window: Duration) -> Brief {
    let plan = Plan::from_prompt(prompt).unwrap();
    let state = Mutex::new(Brief::default());
    run(&plan, fake, &state, Instant::now() + window);
    state.into_inner().unwrap()
}

#[test]
fn likely_def_should_name_a_line_less_hit_by_its_path_alone() {
    let mut fake = Fake::new();
    // No symbol resolves, so the file whose stem matches the anchor is
    // quoted as the likely definition — with no line number of its own.
    fake.files = Ok(found(vec![hit("src/handleError.ts", 0)]));
    fake.symbols = Ok(Vec::new());
    fake.source = Some("export const handleError = (e) => e".to_string());
    let brief = chain("where is `handleError` defined?", &fake, SECOND);
    assert_eq!(
        brief.likely_def.as_deref(),
        Some("src/handleError.ts — export const handleError = (e) => e")
    );
    assert!(
        fake.calls()
            .contains(&"line_at src/handleError.ts:0".into())
    );
    let text = render(&brief).unwrap();
    assert!(
        text.contains("\nlikely definition: src/handleError.ts — export const handleError"),
        "{text}"
    );
    assert!(!text.contains(":0"), "{text}");
}

#[test]
fn likely_def_should_keep_the_line_of_a_hit_that_has_one() {
    let mut fake = Fake::new();
    fake.files = Ok(found(vec![hit("src/handleError.ts", 7)]));
    fake.symbols = Ok(Vec::new());
    fake.source = Some("export const handleError = (e) => e".to_string());
    let brief = chain("where is `handleError` defined?", &fake, SECOND);
    assert!(
        brief
            .likely_def
            .as_deref()
            .is_some_and(|head| head.starts_with("src/handleError.ts:7 — ")),
        "{:?}",
        brief.likely_def
    );
}

#[test]
fn chain_should_search_resolve_a_uid_and_run_impact_on_it_for_a_change_prompt() {
    let mut fake = Fake::new();
    fake.files = Ok(found(vec![
        hit("apps/web/page.tsx", 4),
        hit("data/out.json", 1),
    ]));
    fake.symbols = Ok(vec![symbol("packages/ui/handleError.ts", "handleError")]);
    fake.callers = Ok(vec![caller("apps/web/page.tsx", "Page", 12)]);
    fake.context = Ok((
        "export function handleError(e) { return \"boom\"; }".into(),
        Vec::new(),
    ));
    let brief = chain(
        "handleError in packages/ui/handleError.ts is being renamed to reportError",
        &fake,
        SECOND,
    );
    assert_eq!(
        fake.calls(),
        [
            "files_with handleError",
            "symbols handleError",
            "callers packages/ui/handleError.ts#handleError#function",
            "context packages/ui/handleError.ts#handleError#function budget 400",
            "lines_at apps/web/page.tsx:1-66",
            "lines_at apps/web/page.tsx:1-20000"
        ]
    );
    assert_eq!(brief.files, [rhit_no_text("apps/web/page.tsx", 4)]);
    assert_eq!(brief.excluded, ["data/out.json"]);
    assert_eq!(brief.callers, [caller("apps/web/page.tsx", "Page", 12)]);
    assert_eq!(brief.kind, Some(QuestionKind::Lookup));
    assert_eq!(
        brief.def_body.as_deref(),
        Some("export function handleError(e) { return \"boom\"; }")
    );
    assert_eq!((brief.ops, brief.answered), (4, 4));
    assert!(brief.finished && !brief.cut);
}

#[test]
fn chain_should_list_files_named_by_path_first_and_never_look_up_their_stem() {
    let mut fake = Fake::new();
    fake.source = Some("/** head */".into());
    fake.files = Ok(found(vec![hit(".agents/skills/arena/SKILL.md", 10)]));
    let brief = chain(
        "summarize extensions/plan/index.ts and extensions/todo/index.ts",
        &fake,
        SECOND,
    );
    let calls = fake.calls();
    assert!(
        !calls
            .iter()
            .any(|c| c.contains("symbols") || c.contains("files_with")),
        "{calls:?}"
    );
    assert_eq!(
        brief.files,
        [
            RichHit {
                path: "extensions/plan/index.ts".into(),
                line: 1,
                text: Some("/** head */".into())
            },
            RichHit {
                path: "extensions/todo/index.ts".into(),
                line: 1,
                text: Some("/** head */".into())
            },
        ]
    );
    assert!(brief.unresolved.is_empty() && brief.likely_def.is_none());
    assert!(brief.answered > 0 && brief.finished && !brief.partial());
}

#[test]
fn chain_should_not_pin_a_path_when_the_prompt_names_an_identifier() {
    let mut fake = Fake::new();
    fake.source = Some("import x".into());
    fake.files = Ok(found(vec![hit("README.md", 3)]));
    let brief = chain(
        "explain src/subagent/scheduler.ts and scheduleLoop()",
        &fake,
        SECOND,
    );
    assert!(
        brief
            .files
            .iter()
            .all(|f| f.path != "src/subagent/scheduler.ts")
    );
}

#[test]
fn anchors_should_not_take_a_generic_stem_as_a_symbol() {
    for text in [
        "explain src/a/index.ts",
        "explain src/a/mod.rs",
        "read a/b/index.ts and c/d/index.ts",
    ] {
        let found = Anchors::from_text(text);
        assert_eq!(found.symbol_name(), None, "{text}");
        assert_eq!(found.search_term(), None, "{text}");
    }
    let one = Anchors::from_text("explain src/subagent/scheduler.ts");
    assert_eq!(one.symbol_name().as_deref(), Some("scheduler"));
    let two = Anchors::from_text("explain src/a/scheduler.ts and src/b/runner.ts");
    assert_eq!(two.symbol_name(), None);
}

#[test]
fn chain_should_skip_impact_for_a_literal_lookup() {
    let mut fake = Fake::new();
    fake.files = Ok(found(vec![hit("src/a.ts", 2)]));
    fake.symbols = Ok(vec![symbol("src/a.ts", "fetchUser")]);
    fake.context = Ok(("function fetchUser() {}".into(), Vec::new()));
    let brief = chain("where is fetchUser defined", &fake, SECOND);
    assert_eq!(
        fake.calls(),
        [
            "files_with fetchUser",
            "symbols fetchUser",
            "context src/a.ts#fetchUser#function budget 400",
            "lines_at src/a.ts:1-20000"
        ]
    );
    assert_eq!(brief.ops, 3);
    assert!(brief.callers.is_empty());
    assert!(!brief.impacted);
    assert_eq!(brief.def_body.as_deref(), Some("function fetchUser() {}"));
}

#[test]
fn lookup_context_should_be_skipped_when_the_pick_is_one_of_several() {
    let mut fake = Fake::new();
    fake.files = Ok(found(vec![hit("src/a.ts", 2)]));
    fake.symbols = Ok(vec![
        symbol("src/a.ts", "fetchUser"),
        symbol("src/b.ts", "fetchUser"),
    ]);
    let brief = chain("where is fetchUser defined", &fake, SECOND);
    // Two candidates made the pick a guess: `pack-context` does not
    // follow it, but the one-line `line_at` read still does.
    assert!(!fake.calls().iter().any(|call| call.starts_with("context")));
    assert_eq!(
        brief.unresolved,
        ["find-symbol fetchUser: 2 candidates, took first"]
    );
    assert!(fake.calls().contains(&"line_at src/a.ts:3".to_string()));
}

#[test]
fn chain_should_fall_back_to_the_bare_name_and_record_it() {
    let fake = Fake::new();
    let brief = chain("rename `fetchUser` everywhere", &fake, SECOND);
    assert_eq!(
        fake.calls(),
        [
            "files_with fetchUser",
            "concept rename fetchuser everywhere",
            "symbols fetchUser",
            "callers fetchUser",
            "semantic_hint rename fetchuser everywhere"
        ]
    );
    assert_eq!(
        brief.unresolved,
        ["find-symbol fetchUser: no uid, bare name used"]
    );
    assert_eq!(brief.ops, 4);
    assert!(brief.impacted);
}

#[test]
fn chain_should_search_concepts_only_when_no_file_was_found() {
    let mut fake = Fake::new();
    fake.files = Ok(found(vec![hit("src/a.ts", 2)]));
    chain("explain how `fetchUser` retries", &fake, SECOND);
    assert!(!fake.calls().iter().any(|call| call.starts_with("concept")));
    let mut none = Fake::new();
    none.concept = Ok(found(vec![hit("src/retry.ts", 7)]));
    let brief = chain("how does the retry logic back off", &none, SECOND);
    assert_eq!(
        none.calls(),
        [
            "concept retry logic back",
            "lines_at src/retry.ts:1-69",
            "lines_at src/retry.ts:1-20000"
        ]
    );
    assert_eq!(brief.files, [rhit_no_text("src/retry.ts", 7)]);
}

#[test]
fn chain_should_note_a_failed_operation_and_not_count_it_as_an_answer() {
    let mut fake = Fake::new();
    fake.files = Err("text index is not current".into());
    fake.concept = Err("needs a running daemon".into());
    fake.symbols = Err("graph is stale".into());
    fake.callers = Err("graph is stale".into());
    let brief = chain("callers of `fetchUser`", &fake, SECOND);
    assert_eq!(brief.answered, 0);
    assert_eq!(brief.ops, 4);
    assert_eq!(
        brief.unresolved,
        [
            "search fetchUser: text index is not current",
            "find-code: needs a running daemon",
            "find-symbol fetchUser: graph is stale",
            "who-calls fetchUser: graph is stale"
        ]
    );
    assert_eq!(render(&brief), None);
    // The reach question asks for the endpoint's callers: the op ran
    // even though the symbol lookup failed, and failed like the rest.
    assert!(fake.calls().contains(&"callers fetchUser".to_string()));
}

#[test]
fn chain_should_report_an_ambiguous_impact_as_unresolved() {
    let mut fake = Fake::new();
    fake.symbols = Ok(vec![symbol("a.ts", "go_now"), symbol("b.ts", "go_now")]);
    fake.callers = Err("ambiguous name; re-call with uid".into());
    let brief = chain("who are the callers of go_now", &fake, SECOND);
    assert!(
        fake.calls()
            .contains(&"callers a.ts#go_now#function".to_string())
    );
    assert_eq!(
        brief.unresolved,
        [
            "find-symbol go_now: 2 candidates, took first",
            "impact go_now: ambiguous name; re-call with uid"
        ]
    );
    assert_eq!(brief.defined[0].path, "a.ts");
}

#[test]
fn a_flow_prompt_should_evaluate_between_the_first_two_symbol_anchors() {
    let mut fake = Fake::new();
    fake.symbols = Ok(vec![symbol("a.ts", "render_page")]);
    fake.flow = Ok(Flow::Path {
        hops: vec![
            "render_page".into(),
            "middleware".into(),
            "send_response".into(),
        ],
        notes: vec!["traversal capped at depth 8".into()],
    });
    let brief = chain(
        "how does `render_page` reach `send_response`",
        &fake,
        SECOND,
    );
    assert_eq!(brief.kind, Some(QuestionKind::Flow));
    let call = fake
        .calls()
        .iter()
        .find(|call| call.starts_with("flow "))
        .cloned();
    assert!(
        call.as_deref().is_some_and(|entry| {
            entry.starts_with("flow a.ts#render_page#function -> send_response (")
                && entry.ends_with("ms)")
        }),
        "{call:?}"
    );
    assert_eq!(
        brief.flow.as_deref(),
        Some("render_page -> middleware -> send_response")
    );
    assert_eq!(brief.caps, ["traversal capped at depth 8"]);
    assert_eq!(
        brief.next.as_deref(),
        Some("pixel call-path 'a.ts#render_page#function' 'send_response'")
    );
    let text = render(&brief).unwrap();
    assert!(
        text.contains("\nflow: render_page -> middleware -> send_response\n"),
        "{text}"
    );
    assert!(
        text.contains("\ncaps: traversal capped at depth 8\n"),
        "{text}"
    );
}

#[test]
fn a_flow_without_a_path_should_render_the_honest_negative() {
    let mut fake = Fake::new();
    fake.symbols = Ok(vec![symbol("a.ts", "go_one")]);
    fake.flow = Ok(Flow::Absent);
    let brief = chain("how does `go_one` reach `go_two`", &fake, SECOND);
    assert_eq!(
        brief.flow.as_deref(),
        Some("no call path in the stored snapshot")
    );
    let text = render(&brief).unwrap();
    assert!(
        text.contains("\nflow: no call path in the stored snapshot\n"),
        "{text}"
    );
}

#[test]
fn a_flow_op_error_should_land_in_unresolved_and_keep_the_next_step() {
    let mut fake = Fake::new();
    fake.symbols = Ok(vec![symbol("a.ts", "go_one")]);
    fake.flow = Err("no call path within depth 8".into());
    let brief = chain("how does `go_one` reach `go_two`", &fake, SECOND);
    assert_eq!(
        brief.unresolved,
        ["evaluate a.ts#go_one#function -> go_two: no call path within depth 8"]
    );
    let text = render(&brief).unwrap();
    assert!(text.contains("\npacket partial —"), "{text}");
    assert!(
        text.contains("\nnext: pixel call-path 'a.ts#go_one#function' 'go_two'"),
        "{text}"
    );
}

#[test]
fn a_tests_prompt_should_list_the_test_files_calling_the_pick() {
    let mut fake = Fake::new();
    fake.symbols = Ok(vec![symbol("src/retry.ts", "retry_loop")]);
    fake.test_files = Ok((
        vec!["tests/retry_test.ts".into(), "specs/retry_spec.ts".into()],
        vec!["callers capped at 200".into()],
    ));
    let brief = chain("which tests cover `retry_loop`", &fake, SECOND);
    assert_eq!(brief.kind, Some(QuestionKind::Tests));
    assert_eq!(
        fake.calls(),
        [
            "files_with retry_loop",
            "concept tests cover retry loop",
            "symbols retry_loop",
            "test_files src/retry.ts#retry_loop#function",
            "line_at src/retry.ts:3",
            "semantic_hint tests cover retry loop",
            "lines_at tests/retry_test.ts:1-4000",
            "lines_at specs/retry_spec.ts:1-4000"
        ]
    );
    assert_eq!(brief.tests, ["tests/retry_test.ts", "specs/retry_spec.ts"]);
    assert!(brief.tested);
    assert_eq!(brief.caps, ["callers capped at 200"]);
    assert_eq!(
        brief.next.as_deref(),
        Some("pixel who-calls 'src/retry.ts#retry_loop#function'")
    );
    let text = render(&brief).unwrap();
    assert!(
        text.contains("\ntests: tests/retry_test.ts specs/retry_spec.ts\n"),
        "{text}"
    );
}

#[test]
fn a_tests_prompt_without_a_pick_should_say_why_no_op_ran() {
    let fake = Fake::new();
    let brief = chain("which tests cover `retry_loop`", &fake, SECOND);
    assert!(
        !fake
            .calls()
            .iter()
            .any(|call| call.starts_with("test_files"))
    );
    assert_eq!(
        brief.unresolved,
        [
            "find-symbol retry_loop: no uid, bare name used",
            "tests: no uid resolved for a callers query"
        ]
    );
}

#[test]
fn a_config_prompt_should_admit_json_and_skeleton_the_named_file() {
    let mut fake = Fake::new();
    fake.rows = Some(Ok(rfound(vec![
        rhit("cfg/settings.json", 2, "\"hook\": \"on\""),
        rhit("src/a.ts", 3, "load_config()"),
    ])));
    fake.skeleton = Ok(vec![symbol("cfg/settings.json", "load_config")]);
    let brief = chain(
        "how is the hook configured in cfg/settings.json",
        &fake,
        SECOND,
    );
    assert_eq!(brief.kind, Some(QuestionKind::Config));
    // The JSON hit was admitted past the generated filter; a non-config
    // question would have named it under `excluded`.
    assert_eq!(
        brief.files,
        [
            rhit("cfg/settings.json", 2, "\"hook\": \"on\""),
            rhit("src/a.ts", 3, "load_config()")
        ]
    );
    assert!(brief.excluded.is_empty());
    assert!(
        fake.calls()
            .contains(&"skeleton cfg/settings.json".to_string()),
        "{:?}",
        fake.calls()
    );
    assert_eq!(brief.skeleton_file.as_deref(), Some("cfg/settings.json"));
    let text = render(&brief).unwrap();
    assert!(
        text.contains(
            "\nfiles: cfg/settings.json:2 — \"hook\": \"on\"; src/a.ts:3 — load_config()\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("\nskeleton cfg/settings.json: function load_config:3-9\n"),
        "{text}"
    );
}

#[test]
fn a_rationale_prompt_should_read_history_and_name_search_history_as_the_next_step() {
    let mut fake = Fake::new();
    fake.status = Ok(StatusProbe {
        facts_fresh: Some(true),
    });
    fake.history = Ok((
        vec![history("abc12345", "introduce the retry loop")],
        Vec::new(),
    ));
    let brief = chain("when was `retry_loop` introduced", &fake, SECOND);
    assert_eq!(brief.history.len(), 1, "the library read needs no command");
    assert_eq!(
        brief.next.as_deref(),
        Some("pixel search-history 'retry loop introduced'")
    );
}

#[test]
fn a_rationale_prompt_should_probe_then_run_history_when_facts_are_fresh() {
    let mut fake = Fake::new();
    fake.status = Ok(StatusProbe {
        facts_fresh: Some(true),
    });
    fake.history = Ok((
        vec![
            history("abc12345", "introduce the retry loop"),
            history("def67890", "bound its backoff"),
        ],
        Vec::new(),
    ));
    let brief = chain("when was `retry_loop` introduced", &fake, SECOND);
    assert_eq!(brief.kind, Some(QuestionKind::Rationale));
    let calls = fake.calls();
    let status_at = calls.iter().position(|call| call == "status");
    let history_at = calls.iter().position(|call| call.starts_with("history"));
    assert!(
        status_at.is_some() && history_at.is_some() && status_at < history_at,
        "{calls:?}"
    );
    assert_eq!(
        calls[history_at.unwrap()],
        "history retry loop introduced limit 3"
    );
    assert_eq!(brief.history.len(), 2);
    assert_eq!(
        brief.next.as_deref(),
        Some("pixel search-history 'retry loop introduced'")
    );
    let text = render(&brief).unwrap();
    assert!(
        text.contains("\nhistory: abc12345 introduce the retry loop\n"),
        "{text}"
    );
    assert!(
        text.contains("\nhistory: def67890 bound its backoff\n"),
        "{text}"
    );
}

#[test]
fn a_rationale_prompt_should_spend_nothing_beyond_the_probe_when_facts_are_stale() {
    for probe in [
        Ok(StatusProbe {
            facts_fresh: Some(false),
        }),
        Ok(StatusProbe { facts_fresh: None }),
        Err("daemon down".into()),
    ] {
        let answered = probe.is_ok();
        let mut fake = Fake::new();
        fake.status = probe.clone();
        let brief = chain("why does the cache expire", &fake, SECOND);
        assert!(
            !fake.calls().iter().any(|call| call.starts_with("history")),
            "{probe:?}"
        );
        let expected = if answered {
            "history probe: facts index absent or not fresh"
        } else {
            "history probe: daemon down"
        };
        assert!(
            brief.unresolved.iter().any(|note| note == expected),
            "{probe:?} {:?}",
            brief.unresolved
        );
    }
}

#[test]
fn a_bugfix_prompt_should_run_impact_then_pack_context_on_the_pick() {
    let mut fake = Fake::new();
    fake.symbols = Ok(vec![symbol("src/cache.ts", "expire")]);
    fake.callers = Ok(vec![caller("src/api.ts", "handler", 12)]);
    fake.context = Ok(("fn expire() {\n  ttl = 0;\n}".into(), Vec::new()));
    let brief = chain("fix the crash in `expire`", &fake, SECOND);
    assert_eq!(brief.kind, Some(QuestionKind::Bugfix));
    // The defect word routed the kind; the kind routed the blast
    // radius even without a change stem in the phrasing.
    assert_eq!(
        fake.calls(),
        [
            "files_with expire",
            "concept crash expire",
            "symbols expire",
            "callers src/cache.ts#expire#function",
            "context src/cache.ts#expire#function budget 400",
            "semantic_hint crash expire"
        ]
    );
    assert_eq!(
        brief.def_body.as_deref(),
        Some("fn expire() {\n  ttl = 0;\n}")
    );
    assert_eq!(
        brief.next.as_deref(),
        Some("pixel impact 'src/cache.ts#expire#function'")
    );
    let text = render(&brief).unwrap();
    assert!(text.contains("\nkind: bugfix\n"), "{text}");
    assert!(text.contains("— fn expire() {   ttl = 0; }"), "{text}");
}

#[test]
fn the_semantic_hint_should_close_an_empty_brief_when_the_index_is_warm() {
    let mut fake = Fake::new();
    fake.semantic = Some("pixel search-meaning 'retry logic'".into());
    let brief = chain("how does the retry logic back off", &fake, SECOND);
    assert_eq!(
        brief.semantic.as_deref(),
        Some("pixel search-meaning 'retry logic'")
    );
    let text = render(&brief).unwrap();
    assert!(
        text.contains("\nsemantic: available — pixel search-meaning 'retry logic'\n"),
        "{text}"
    );
}

#[test]
fn matched_text_should_ride_the_files_line() {
    let mut fake = Fake::new();
    fake.rows = Some(Ok(rfound(vec![
        rhit("src/a.ts", 3, "handleError(e)"),
        rhit("src/a.ts", 9, "return handleError(e)"),
    ])));
    let brief = chain("where is handleError used", &fake, SECOND);
    // Both sites of one file stayed: the production line and the call line.
    assert_eq!(
        brief.files,
        [
            rhit("src/a.ts", 3, "handleError(e)"),
            rhit("src/a.ts", 9, "return handleError(e)")
        ]
    );
    let text = render(&brief).unwrap();
    assert!(
        text.contains("\nfiles: src/a.ts:3 — handleError(e); src/a.ts:9 — return handleError(e)\n"),
        "{text}"
    );
}

#[test]
fn spend_should_refuse_after_the_cap_and_after_the_deadline() {
    let state = Mutex::new(Brief::default());
    let later = Instant::now() + SECOND;
    for _ in 0..MAX_OPS {
        assert!(spend(&state, later));
    }
    assert!(!spend(&state, later));
    let brief = edit(&state, |brief| brief.clone());
    assert_eq!((brief.ops, brief.cut), (MAX_OPS, true));

    let fresh = Mutex::new(Brief::default());
    assert!(!spend(&fresh, Instant::now()));
    let brief = edit(&fresh, |brief| brief.clone());
    assert_eq!((brief.ops, brief.cut), (0, true));
}

#[test]
fn chain_should_stop_at_the_deadline_and_mark_the_brief_cut() {
    let mut fake = Fake::new();
    fake.pause = Duration::from_millis(120);
    fake.files = Ok(found(vec![hit("src/a.ts", 2)]));
    let brief = chain("callers of `fetchUser`", &fake, Duration::from_millis(60));
    assert_eq!(fake.calls(), ["files_with fetchUser"]);
    assert!(brief.cut);
    assert_eq!(brief.ops, 1);
    let text = render(&brief).unwrap();
    assert!(
        text.contains("coverage: 1/1 ops answered | partial: budget"),
        "{text}"
    );
    assert!(
        text.contains("packet partial — open cited regions or run the named op"),
        "{text}"
    );
}

#[test]
fn pending_should_return_nothing_when_no_operation_answers_before_the_deadline() {
    let mut fake = Fake::new();
    fake.pause = Duration::from_millis(400);
    fake.files = Ok(found(vec![hit("src/a.ts", 2)]));
    let started = Instant::now();
    let pending = start_with(
        "where is `fetchUser` used",
        Duration::from_millis(150),
        move |_| Box::new(fake),
    )
    .unwrap();
    assert_eq!(pending.finish(), None);
    assert!(
        started.elapsed() < Duration::from_millis(390),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn pending_should_render_what_answered_before_the_deadline() {
    struct Slow;
    impl Evidence for Slow {
        fn files_with(&self, _: &str, _: Instant) -> Result<Found, String> {
            Ok(found(vec![hit("src/a.ts", 2)]))
        }
        fn concept(&self, _: &str, _: Instant) -> Result<Found, String> {
            Ok(Found::default())
        }
        fn symbols(&self, _: &str, _: Instant) -> Result<Vec<SymbolHit>, String> {
            std::thread::sleep(Duration::from_millis(500));
            Ok(Vec::new())
        }
        fn callers(&self, _: &str, _: Instant) -> Result<Vec<CallerHit>, String> {
            Ok(Vec::new())
        }
    }
    let started = Instant::now();
    let pending = start_with(
        "where is `fetchUser` used",
        Duration::from_millis(200),
        |_| Box::new(Slow),
    )
    .unwrap();
    let text = pending.finish().unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(480),
        "{:?}",
        started.elapsed()
    );
    assert!(text.starts_with(BRIEF_TAG), "{text}");
    assert!(text.contains("files: src/a.ts:2\n"), "{text}");
    assert!(
        text.contains("coverage: 1/2 ops answered | partial: budget"),
        "{text}"
    );
}

fn full_brief() -> Brief {
    Brief {
        anchors: vec!["handleError".into(), "src/handleError.ts".into()],
        kind: Some(QuestionKind::Bugfix),
        files: vec![
            rhit("apps/web/page.tsx", 4, "return handleError(e)"),
            rhit_no_text("apps/web/other.tsx", 9),
        ],
        defined: vec![symbol("src/handleError.ts", "handleError")],
        def_head: Some("export function handleError(e: Error): string {".into()),
        def_body: Some("export function handleError(e) {\n  return \"boom\";\n}".into()),
        likely_def: Some("src/x.ts:1 — export const x".into()),
        callers: vec![caller("apps/web/page.tsx", "Page", 12)],
        flow: Some("one -> two -> three".into()),
        tests: vec!["tests/handleError_test.ts".into()],
        skeleton: vec![symbol("cfg/config.json", "load")],
        skeleton_file: Some("cfg/config.json".into()),
        history: vec![history("abc1234", "fix retry")],
        semantic: Some("pixel search-meaning 'handle errors'".into()),
        caps: vec!["context truncated at budget".into()],
        next: Some("pixel impact 'src/handleError.ts#handleError#function'".into()),
        unambiguous_def: true,
        excluded: vec!["data/out.json".into()],
        unresolved: vec!["find-symbol handleError: 2 candidates, took first".into()],
        ops: 3,
        answered: 3,
        searched: true,
        impacted: true,
        tested: true,
        cut: false,
        finished: true,
        ..Brief::default()
    }
}

#[test]
fn staleness_should_survive_a_partial_snapshot_and_render_in_the_brief() {
    let brief = full_brief();
    let mut partial = brief.clone();
    partial.cut = true;
    assert!(!render(&partial).unwrap().contains("staleness:"));
    // A response arrives after the snapshot, before the worker finishes.
    brief.staleness.mark();
    let text = render(&partial).unwrap();
    assert_eq!(text.matches(STALENESS_LINE).count(), 1);
    assert!(text.contains("packet partial"));
    let complete = full_brief();
    complete.staleness.mark();
    let text = render(&complete).unwrap();
    assert_eq!(text.matches(STALENESS_LINE).count(), 1);
}

#[test]
fn render_should_lay_out_every_line_in_order() {
    assert_eq!(
        render(&full_brief()).unwrap(),
        [
            "[PIXEL:BRIEF]",
            "kind: bugfix",
            "anchors: handleError, src/handleError.ts",
            "defined: function handleError src/handleError.ts:3-9 — export function handleError(e) {   return \"boom\"; }",
            "likely definition: src/x.ts:1 — export const x",
            "skeleton cfg/config.json: function load:3-9",
            "files: apps/web/page.tsx:4 — return handleError(e); apps/web/other.tsx:9",
            "callers (impact d1): apps/web/page.tsx -> Page:12",
            "flow: one -> two -> three",
            "tests: tests/handleError_test.ts",
            "history: abc1234 fix retry",
            "semantic: available — pixel search-meaning 'handle errors'",
            "excluded (generated): data/out.json",
            "caps: context truncated at budget",
            "unresolved: find-symbol handleError: 2 candidates, took first",
            "coverage: 3/3 ops answered",
            "packet partial — open cited regions or run the named op",
            "next: pixel impact 'src/handleError.ts#handleError#function'",
        ]
        .join("\n")
    );
}

#[test]
fn render_should_keep_the_verification_footer_when_nothing_is_missing() {
    let mut brief = Brief {
        anchors: vec!["fetchUser".into()],
        kind: Some(QuestionKind::Lookup),
        files: vec![rhit_no_text("src/a.ts", 2)],
        ops: 2,
        answered: 2,
        searched: true,
        finished: true,
        ..Brief::default()
    };
    let text = render(&brief).unwrap();
    assert!(text.ends_with(UNCORROBORATED_FOOTER), "{text}");
    assert!(!text.contains("packet partial"), "{text}");
    assert!(!text.contains("next:"), "{text}");
    brief.cut = true;
    let text = render(&brief).unwrap();
    assert!(text.contains("packet partial"), "{text}");
    assert!(!text.ends_with(UNCORROBORATED_FOOTER), "{text}");
}

#[test]
fn render_should_omit_the_line_number_of_a_caller_without_one() {
    let mut brief = full_brief();
    brief.callers = vec![caller("apps/web/page.tsx", "Page", 0)];
    assert!(
        render(&brief)
            .unwrap()
            .contains("\ncallers (impact d1): apps/web/page.tsx -> Page\n")
    );
}

#[test]
fn render_should_omit_the_line_number_of_a_file_without_one() {
    let mut brief = full_brief();
    brief.files = vec![
        rhit_no_text("apps/web/page.tsx", 0),
        rhit_no_text("packages/ui/handleError.ts", 1),
    ];
    let text = render(&brief).unwrap();
    assert!(
        text.contains("\nfiles: apps/web/page.tsx packages/ui/handleError.ts:1\n"),
        "{text}"
    );
    // Every shape a `path:0` could take: the bare hit, and the pair with
    // a real line beside it.
    assert!(!text.contains(":0"), "{text}");
}

#[test]
fn render_should_say_none_for_an_answered_empty_list_and_omit_an_unrun_one() {
    let brief = Brief {
        anchors: vec!["fetchUser".into()],
        ops: 1,
        answered: 1,
        searched: true,
        finished: true,
        ..Brief::default()
    };
    let text = render(&brief).unwrap();
    assert!(text.contains("\nfiles: none\n"), "{text}");
    assert!(!text.contains("callers (impact"), "{text}");
    assert!(!text.contains("defined:"), "{text}");
    assert!(text.contains("coverage: 1/1 ops answered\n"), "{text}");
    let impacted = Brief {
        impacted: true,
        ..brief
    };
    assert!(
        render(&impacted)
            .unwrap()
            .contains("\ncallers (impact d1): none\n")
    );
}

#[test]
fn render_should_return_nothing_when_no_operation_answered() {
    assert_eq!(render(&Brief::default()), None);
    let failed = Brief {
        ops: 2,
        unresolved: vec!["search x: down".into()],
        ..Brief::default()
    };
    assert_eq!(render(&failed), None);
}

#[test]
fn render_should_show_the_list_caps_and_name_what_is_hidden() {
    let mut brief = full_brief();
    brief.files = (0..30)
        .map(|n| rhit_no_text(&format!("apps/web/some/deep/dir/component_{n}.tsx"), n))
        .collect();
    brief.callers = (0..30)
        .map(|n| {
            caller(
                &format!("apps/web/some/deep/dir/caller_{n}.tsx"),
                "render",
                n + 1,
            )
        })
        .collect();
    brief.defined = (0..5)
        .map(|n| symbol(&format!("src/d{n}.ts"), "go"))
        .collect();
    let text = render(&brief).unwrap();
    assert!(text.len() <= BRIEF_BYTES, "{} bytes: {text}", text.len());
    assert!(text.contains("component_7.tsx:7 (+22 more)"), "{text}");
    assert!(
        text.contains("caller_9.tsx -> render:10 (+20 more)"),
        "{text}"
    );
    assert!(text.contains("src/d2.ts:3-9 (+2 more)"), "{text}");
    assert_eq!(text.lines().next(), Some(BRIEF_TAG));
    assert_eq!(
        text.lines().last(),
        Some("next: pixel impact 'src/handleError.ts#handleError#function'")
    );
}

#[test]
fn render_should_drop_whole_entries_until_it_fits_and_stop_as_soon_as_it_does() {
    let mut brief = full_brief();
    brief.files = (0..8)
        .map(|n| rhit_no_text(&format!("{}{n}", "d".repeat(MAX_ITEM_CHARS - 1)), 1))
        .collect();
    brief.callers = (0..10)
        .map(|n| {
            caller(
                &format!("{}{n}", "c".repeat(MAX_ITEM_CHARS - 1)),
                &"v".repeat(60),
                1,
            )
        })
        .collect();
    brief.unresolved = (0..10)
        .map(|n| format!("{}{n}", "u".repeat(MAX_ITEM_CHARS)))
        .collect();
    let text = render(&brief).unwrap();
    assert!(text.len() <= BRIEF_BYTES, "{} bytes", text.len());
    assert!(text.len() > BRIEF_BYTES - 200, "{} bytes", text.len());
    assert!(text.contains("coverage: 3/3 ops answered"), "{text}");
    assert!(text.contains("packet partial"), "{text}");
    for label in ["files:", "callers (impact d1):", "unresolved:"] {
        let line = text.lines().find(|line| line.starts_with(label));
        assert!(
            line.is_some_and(|line| line.contains(" more)")),
            "{label}: {text}"
        );
    }
}

#[test]
fn shrink_should_drop_from_the_longest_list_and_break_ties_in_priority_order() {
    let mut shown = Shown {
        files: 1,
        defined: 1,
        callers: 1,
        tests: 1,
        skeleton: 1,
        history: 1,
        excluded: 1,
        caps: 1,
        unresolved: 1,
        excerpt: [0, 0, 0],
    };
    let counts = |shown: &Shown| {
        [
            shown.excluded,
            shown.files,
            shown.tests,
            shown.skeleton,
            shown.history,
            shown.callers,
            shown.defined,
            shown.caps,
            shown.unresolved,
        ]
    };
    let mut order = Vec::new();
    let mut prev = counts(&shown);
    while shown.shrink() {
        let now = counts(&shown);
        order.push(
            now.iter()
                .zip(&prev)
                .position(|(after, before)| after < before)
                .expect("shrink drops exactly one entry"),
        );
        prev = now;
    }
    // All ones drop in the stated priority order, one at a time.
    assert_eq!(order, [0, 1, 2, 3, 4, 5, 6, 7, 8]);
    let mut uneven = Shown {
        files: 2,
        defined: 1,
        callers: 5,
        tests: 0,
        skeleton: 0,
        history: 0,
        excluded: 0,
        caps: 0,
        unresolved: 3,
        excerpt: [0, 0, 0],
    };
    assert!(uneven.shrink());
    assert_eq!((uneven.files, uneven.callers, uneven.unresolved), (2, 4, 3));
}

#[test]
fn list_line_should_cap_and_count_the_hidden_entries() {
    let items = || ["a", "b", "c"].into_iter().map(String::from);
    assert_eq!(
        list_line("x", items(), 3, " ", false).as_deref(),
        Some("x: a b c")
    );
    assert_eq!(
        list_line("x", items(), 2, " ", false).as_deref(),
        Some("x: a b (+1 more)")
    );
    assert_eq!(
        list_line("x", items(), 0, " ", false).as_deref(),
        Some("x: 3 not shown")
    );
    assert_eq!(
        list_line("x", std::iter::empty(), 0, " ", true).as_deref(),
        Some("x: none")
    );
    assert_eq!(list_line("x", std::iter::empty(), 0, " ", false), None);
}

#[test]
fn clean_should_flatten_control_characters_and_bound_the_length() {
    assert_eq!(clean("a\nb\t[PIXEL]\r"), "a b [PIXEL] ");
    assert_eq!(
        clean(&"x".repeat(MAX_ITEM_CHARS + 5)).chars().count(),
        MAX_ITEM_CHARS
    );
    assert_eq!(
        clean(&"é".repeat(MAX_ITEM_CHARS)).chars().count(),
        MAX_ITEM_CHARS
    );
}

#[test]
fn absorb_should_split_generated_files_and_dedupe_by_path_and_line() {
    let mut brief = Brief::default();
    brief.absorb(
        rfound(vec![
            rhit("src/a.ts", 3, "handleError(e)"),
            rhit_no_text("src/a.ts", 9),
            rhit_no_text("src/a.ts", 9),
            rhit_no_text("package.json", 1),
            rhit_no_text("package.json", 4),
        ]),
        false,
    );
    // Distinct same-file sites — the production line and the test line
    // — both survive; only an exact (path, line) repeat is deduped.
    assert_eq!(
        brief.files,
        [
            rhit("src/a.ts", 3, "handleError(e)"),
            rhit_no_text("src/a.ts", 9)
        ]
    );
    assert_eq!(brief.excluded, ["package.json"]);
}

#[test]
fn absorb_should_admit_json_only_for_a_config_question_and_never_a_credential() {
    let mut config = Brief::default();
    config.absorb(
        rfound(vec![
            rhit_no_text("cfg/settings.json", 2),
            rhit_no_text(".aws/credentials.json", 1),
            rhit_no_text("data/out.lock", 1),
        ]),
        true,
    );
    assert_eq!(config.files, [rhit_no_text("cfg/settings.json", 2)]);
    // A credential-shaped path is dropped silently — never even named
    // in `excluded`; a `.lock` stays generated either way.
    assert_eq!(config.excluded, ["data/out.lock"]);
    let mut lookup = Brief::default();
    lookup.absorb(
        rfound(vec![
            rhit_no_text("cfg/settings.json", 2),
            rhit_no_text("src/a.ts", 3),
        ]),
        false,
    );
    assert_eq!(lookup.files, [rhit_no_text("src/a.ts", 3)]);
    assert_eq!(lookup.excluded, ["cfg/settings.json"]);
}

#[test]
fn a_weak_prompt_without_a_verdict_should_keep_the_heuristic_plan() {
    let mut fake = Fake::new();
    fake.concept = Ok(found(vec![hit("src/login.ts", 3)]));
    let pending = start_with("how does the login flow work", SECOND, move |_| {
        Box::new(fake)
    })
    .unwrap();
    let text = pending.finish().unwrap();
    assert!(text.contains("src/login.ts:3"), "{text}");
    assert!(!text.contains("intent:"), "{text}");
}

#[test]
fn a_strong_prompt_should_never_ask_the_judge() {
    let pending = start_with("where is `fetchUser` used", SECOND, |_| {
        Box::new(Fake::new())
    })
    .unwrap();
    let _ = pending.finish();
}

// ---- plain-language and weak prompts: task-intent admission ----

/// A plain-language prompt: no identifier, no code word, no operation.
const PROSE: &str = "the daemon misses changes made during startup";
const DAEMON: &str = "crates/pixel-daemon/src/daemon.rs";

fn keyword(word: &str, weight: f64) -> relevance::KeywordStat {
    relevance::KeywordStat {
        keyword: word.into(),
        weight,
        french_only: false,
    }
}

fn cofile(path: &str, words: &[&str], weight: f64, structural: bool) -> relevance::CoFileStat {
    relevance::CoFileStat {
        path: path.into(),
        keywords: words.iter().map(ToString::to_string).collect(),
        weight,
        structural,
    }
}

/// The repository talks about the prompt: one structural file holds all
/// its words (2 each), a note holds two of them as text.
fn on_topic_answer() -> RelevanceAnswer {
    RelevanceAnswer {
        input: RelevanceInput {
            graph: true,
            files_considered: 1000,
            structural_files: 30,
            keywords: vec![
                keyword("daemon", 2.0),
                keyword("changes", 2.0),
                keyword("startup", 2.0),
            ],
            cofiles: vec![
                cofile(DAEMON, &["daemon", "changes", "startup"], 6.0, true),
                cofile("docs/notes.md", &["daemon", "startup"], 4.0, false),
            ],
        },
        lines: vec![
            rhit(DAEMON, 280, "fn watch_ready"),
            rhit("docs/notes.md", 12, "daemon startup notes"),
        ],
    }
}

/// The repository does not talk about the prompt: two of its words are
/// nowhere (the cap), the third is everywhere (0), and no file holds any.
fn off_topic_answer() -> RelevanceAnswer {
    RelevanceAnswer {
        input: RelevanceInput {
            graph: true,
            files_considered: 1000,
            structural_files: 0,
            keywords: vec![
                keyword("weather", 3.0),
                keyword("tomorrow", 3.0),
                keyword("like", 0.0),
            ],
            cofiles: Vec::new(),
        },
        lines: Vec::new(),
    }
}

/// A test-only lexical verdict; brief output must not depend on it.
fn verdict_in(tier: Tier, input: &GateInput) -> relevance::Verdict {
    relevance::Verdict {
        tier,
        basis: Basis::Model,
        score: Some(match tier {
            Tier::High => 2.0,
            Tier::Low => 1.0,
            Tier::Off => -1.0,
        }),
        best_file: input
            .relevance
            .cofiles
            .first()
            .map(|file| file.path.clone()),
        features: relevance::Features {
            keywords: 3,
            informative: 3,
            shared: 3,
            ..relevance::Features::default()
        },
    }
}

fn model_high(input: &GateInput) -> relevance::Verdict {
    verdict_in(Tier::High, input)
}

fn model_off(input: &GateInput) -> relevance::Verdict {
    verdict_in(Tier::Off, input)
}

fn lead(path: &str, line: u32, snippet: &str) -> MeaningHit {
    MeaningHit {
        path: path.into(),
        start_line: line,
        end_line: line + 9,
        symbol: None,
        score: 0.5,
        snippet: snippet.into(),
    }
}

fn sorted(mut calls: Vec<String>) -> Vec<String> {
    calls.sort();
    calls
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pixel-brief-chain-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Start `prompt` on `fake` with `model`, and wait for the brief.
fn briefed_with(prompt: &str, fake: Fake, model: Model) -> Finished {
    try_start_with(
        prompt,
        SECOND,
        move |_| Box::new(fake),
        model,
        Around::default(),
    )
    .expect("the prompt starts a brief")
    .finish_with_record()
}

/// [`briefed_with`] a model that finds the prompt on topic.
fn briefed(prompt: &str, fake: Fake) -> Finished {
    briefed_with(prompt, fake, model_high)
}

#[test]
fn a_prose_prompt_on_topic_should_render_fused_files_without_a_lexical_tier() {
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.meaning = Ok(vec![
        lead(DAEMON, 280, "fn watch_ready() {"),
        lead("crates/pixel-daemon/src/api.rs", 40, "fn op_status"),
    ]);
    let fake = Arc::new(fake);
    let seen = Arc::clone(&fake);
    let finished = try_start_with(
        PROSE,
        SECOND,
        move |_| Box::new(Shared(fake)),
        model_high,
        Around::default(),
    )
    .unwrap()
    .finish_with_record();
    // The file both lists name leads, on the meaning chunk's line; then
    // the two files one list names, the meaning lead first.
    assert_eq!(
        finished.text.as_deref(),
        Some(
            [
                "[PIXEL:BRIEF]",
                "kind: lookup",
                "searched: content+symbols+paths for daemon, changes, startup (3 terms, 1000 files) · meaning search returned 2 chunks",
                "result: the matches below are the best across both searches; verify they answer the question",
                "files: crates/pixel-daemon/src/daemon.rs:280 — fn watch_ready() {; crates/pixel-daemon/src/api.rs:40 — fn op_status; docs/notes.md:12 — daemon startup notes",
                "coverage: 2/2 ops answered",
                UNCORROBORATED_FOOTER,
            ]
            .join("\n")
            .as_str()
        )
    );
    assert_eq!(
        sorted(seen.calls()),
        [
            format!("meaning {PROSE} limit 8"),
            format!("relevance {PROSE}")
        ]
    );
    let record = finished.record;
    assert_eq!(
        (record.signal, record.gate, record.enforced),
        (Some("prose"), "open", false)
    );
    assert_eq!(record.model, None);
    assert_eq!((record.tier, record.score), (None, None));
    assert_eq!(record.best_file, None);
    assert_eq!((record.ops, record.answered), (2, 2));
    assert_eq!(record.features, None);
    assert_eq!(record.bytes, finished.text.unwrap().len());
    assert_eq!(record.typed, PROSE);
    assert_eq!(record.sha256, pixel_metrics::journal::sha256_hex(PROSE));
}

#[test]
fn a_prose_prompt_without_meaning_leads_should_fuse_the_plain_word_leads_in_their_place() {
    // No model on disk: the meaning search fails, and the file only the
    // plain words found joins the co-files in the fusion, ahead of the
    // doc the co-files name second. Without the fallback it is absent.
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.words = Ok(vec![lead("crates/pixel-daemon/src/api.rs", 40, "")]);
    let finished = briefed(PROSE, fake);
    assert_eq!(
        files_line(finished.text.as_deref().unwrap_or_default()),
        "files: crates/pixel-daemon/src/api.rs:40; crates/pixel-daemon/src/daemon.rs:280 — fn watch_ready; docs/notes.md:12 — daemon startup notes"
    );
    // The receipt still says the meaning search did not answer: the
    // plain words are not counted as one.
    let text = finished.text.unwrap();
    assert!(!text.contains("meaning search returned"), "{text}");
    assert!(text.contains("coverage: 1/2 ops answered"), "{text}");
}

#[test]
fn a_prose_prompt_with_meaning_leads_should_never_read_the_plain_word_leads() {
    // The fallback is a fallback: when the meaning search answers, the
    // brief is byte for byte what it was before the plain words existed.
    let with_words = || {
        let mut fake = Fake::new();
        fake.relevance = Ok(on_topic_answer());
        fake.meaning = Ok(vec![lead(DAEMON, 280, "fn watch_ready() {")]);
        fake.words = Ok(vec![lead("crates/pixel-daemon/src/api.rs", 40, "")]);
        fake
    };
    let mut without = with_words();
    without.words = Err("unavailable".into());
    let with = briefed(PROSE, with_words()).text;
    assert!(
        !with.as_deref().unwrap_or_default().contains("api.rs"),
        "{with:?}"
    );
    assert_eq!(with, briefed(PROSE, without).text);
}

#[test]
fn a_low_lexical_score_should_not_suppress_a_prose_prompt() {
    let mut fake = Fake::new();
    fake.relevance = Ok(off_topic_answer());
    fake.meaning = Ok(vec![lead(DAEMON, 1, "x")]);
    let finished = briefed_with(
        "what's the weather going to be like tomorrow",
        fake,
        model_off,
    );
    assert!(finished.text.is_some());
    let record = finished.record;
    assert_eq!((record.signal, record.gate), (Some("prose"), "open"));
    assert_eq!((record.tier, record.score), (None, None));
    assert_eq!(record.reason, None);
    assert!(record.answered > 0 && record.bytes > 0);
}

#[test]
fn a_relevance_probe_failure_should_not_stop_the_meaning_search() {
    let mut fake = Fake::new();
    fake.relevance = Err("cold".into());
    fake.meaning = Ok(vec![lead(DAEMON, 1, "x")]);
    fake.meaning_pause = Duration::from_millis(1200);
    let started = Instant::now();
    let finished = try_start_with(
        PROSE,
        Duration::from_secs(5),
        move |_| Box::new(fake),
        model_high,
        Around::default(),
    )
    .unwrap()
    .finish_with_record();
    assert!(
        started.elapsed() >= Duration::from_millis(1100),
        "{:?}",
        started.elapsed()
    );
    assert!(finished.text.is_some());
    assert_eq!(finished.record.gate, "open");
    assert_eq!(finished.record.reason, None);
}

#[test]
fn failed_probes_should_not_close_task_intent_admission() {
    let finished = briefed(PROSE, Fake::new());
    assert!(finished.text.is_some());
    assert_eq!(finished.record.gate, "open");
    assert_eq!(finished.record.reason, None);
    assert_eq!((finished.record.ops, finished.record.answered), (3, 1));
}

#[test]
fn a_prose_prompt_should_still_render_the_cofiles_when_the_meaning_search_fails() {
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.meaning = Err("cold".into());
    let finished = briefed(PROSE, fake);
    let text = finished.text.unwrap();
    assert!(
        text.contains(
            "\nfiles: crates/pixel-daemon/src/daemon.rs:280 — fn watch_ready; docs/notes.md:12 — daemon startup notes\n"
        ),
        "{text}"
    );
    assert!(text.contains("\ncoverage: 1/2 ops answered\n"), "{text}");
    assert!(!text.contains("unresolved"), "{text}");
}

#[test]
fn a_prose_prompt_whose_relevance_probe_outlives_the_window_should_render_nothing_on_time() {
    struct Hang;
    impl Evidence for Hang {
        fn files_with(&self, _: &str, _: Instant) -> Result<Found, String> {
            Err("unused".into())
        }
        fn concept(&self, _: &str, _: Instant) -> Result<Found, String> {
            Err("unused".into())
        }
        fn symbols(&self, _: &str, _: Instant) -> Result<Vec<SymbolHit>, String> {
            Err("unused".into())
        }
        fn callers(&self, _: &str, _: Instant) -> Result<Vec<CallerHit>, String> {
            Err("unused".into())
        }
        fn relevance(&self, _: &str, _: Instant) -> Result<RelevanceAnswer, String> {
            std::thread::sleep(Duration::from_millis(500));
            Ok(on_topic_answer())
        }
    }
    let started = Instant::now();
    let pending = start_with(PROSE, Duration::from_millis(150), |_| Box::new(Hang)).unwrap();
    assert_eq!(pending.finish(), None);
    assert!(
        started.elapsed() < Duration::from_millis(450),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_weak_prompt_should_take_the_probe_leads_and_skip_concept_matching() {
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.meaning = Ok(vec![lead(
        "crates/pixel-daemon/src/api.rs",
        40,
        "fn op_status",
    )]);
    fake.concept = Ok(found(vec![hit("src/login.ts", 3)]));
    let finished = briefed("how does the login flow work", fake);
    let text = finished.text.unwrap();
    assert!(
        text.contains(
            "\nfiles: crates/pixel-daemon/src/api.rs:40 — fn op_status; crates/pixel-daemon/src/daemon.rs:280 — fn watch_ready; docs/notes.md:12 — daemon startup notes\n"
        ),
        "{text}"
    );
    assert!(!text.contains("src/login.ts"), "{text}");
    assert_eq!(finished.record.signal, Some("weak"));
    assert_eq!(finished.record.gate, "open");
}

#[test]
fn a_weak_prompt_whose_probes_fail_should_fall_back_to_concept_matching_quietly() {
    let mut fake = Fake::new();
    fake.concept = Ok(found(vec![hit("src/login.ts", 3)]));
    let finished = briefed("how does the login flow work", fake);
    let text = finished.text.unwrap();
    assert!(text.contains("\nfiles: src/login.ts:3\n"), "{text}");
    assert!(!text.contains("unresolved"), "{text}");
    assert!(!text.contains("confidence"), "{text}");
    assert!(text.ends_with(UNCORROBORATED_FOOTER), "{text}");
    assert_eq!(finished.record.gate, "open");
}

#[test]
fn a_low_lexical_score_should_not_suppress_a_weak_prompt() {
    let mut fake = Fake::new();
    fake.relevance = Ok(off_topic_answer());
    fake.concept = Ok(found(vec![hit("src/login.ts", 3)]));
    let finished = briefed_with("how does the login flow work", fake, model_off);
    assert!(finished.text.is_some());
    assert_eq!(finished.record.gate, "open");
    assert_eq!(finished.record.tier, None);
    assert!(!finished.record.enforced);
}

#[test]
fn an_investigation_lead_should_supply_source_evidence_without_keyword_overlap() {
    let prompt = "Explain how this component behaves";
    assert!(Plan::from_typed(prompt).anchors.names().next().is_none());

    let mut fake = Fake::new();
    fake.relevance = Ok(off_topic_answer());
    fake.meaning = Ok(vec![lead_at("src/feed.js", 1, Some("render_feed"))]);
    fake.sources = vec![(
        "src/feed.js".into(),
        [
            "function render_feed(posts) {",
            "  return posts.map(render_post).join(\"\");",
            "}",
            "function render_post(post) {",
            "  return `${post.author}: ${post.body}`;",
            "}",
        ]
        .map(str::to_string)
        .to_vec(),
    )];
    let intent = serde_json::json!({
        "predicted": "investigate",
        "probs": {"investigate": 0.68, "question": 0.32, "none": 0.0},
        "snapshot": {"provider": "jev", "model": "configured-model", "confidence": 0.63}
    });
    let finished = try_start_with(
        prompt,
        SECOND,
        move |_| Box::new(fake),
        model_off,
        Around {
            intent: Some(intent),
            pack: region_pack::Limit::of(2),
            ..Around::default()
        },
    )
    .unwrap()
    .finish_with_record();

    let text = finished
        .text
        .expect("an admitted investigation gets evidence");
    assert!(text.contains("src/feed.js"), "{text}");
    assert!(text.contains("function render_feed(posts)"), "{text}");
    assert!(text.contains("posts.map(render_post)"), "{text}");
    assert!(finished.record.answer || finished.record.pack_parts > 0);
    assert_eq!(
        finished.record.intent.as_ref().unwrap()["label"],
        "investigate"
    );
    assert_eq!(finished.record.gate, "open");
    assert_eq!(finished.record.tier, None);
    assert_eq!(finished.record.model, None);
    assert_eq!(finished.record.score, None);
}

#[test]
fn a_strong_prompt_should_never_ask_the_relevance_probe_or_the_meaning_search() {
    let mut fake = Fake::new();
    fake.files = Ok(found(vec![hit("src/a.ts", 2)]));
    fake.relevance = Ok(on_topic_answer());
    fake.meaning = Ok(vec![lead(DAEMON, 1, "x")]);
    let fake = Arc::new(fake);
    let seen = Arc::clone(&fake);
    let finished = start_with("where is `fetchUser` used", SECOND, move |_| {
        Box::new(Shared(fake))
    })
    .unwrap()
    .finish_with_record();
    assert!(
        seen.calls()
            .iter()
            .all(|call| !call.starts_with("relevance") && !call.starts_with("meaning")),
        "{:?}",
        seen.calls()
    );
    assert_eq!(finished.record.signal, Some("strong"));
    assert_eq!(finished.record.gate, "open");
    assert!(!finished.record.enforced);
    assert_eq!(finished.record.score, None);
}

#[test]
fn gather_should_spend_one_op_per_probe_and_skip_a_probe_the_budget_refuses() {
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.meaning = Ok(vec![lead(DAEMON, 1, "x")]);
    let state = Mutex::new(Brief {
        ops: MAX_OPS - 1,
        ..Brief::default()
    });
    let got = gather(PROSE, &fake, &state, Instant::now() + SECOND);
    assert_eq!(fake.calls(), [format!("relevance {PROSE}")]);
    assert!(matches!(got.relevance, Some(Ok(_))));
    assert!(got.meaning.is_none());
    let brief = edit(&state, |brief| brief.clone());
    assert_eq!((brief.ops, brief.cut), (MAX_OPS, true));

    let fresh = Mutex::new(Brief::default());
    let both = Fake::new();
    gather(PROSE, &both, &fresh, Instant::now() + SECOND);
    assert_eq!(edit(&fresh, |brief| brief.ops), 2);
    assert_eq!(
        sorted(both.calls()),
        [
            format!("meaning {PROSE} limit 8"),
            format!("relevance {PROSE}")
        ]
    );
}

#[test]
fn fuse_should_rank_a_file_both_lists_name_above_one_list_and_keep_the_meaning_row() {
    let fused = fuse(
        &[
            lead("a.rs", 10, "from meaning"),
            lead("b.rs", 20, "b meaning"),
        ],
        &[rhit("b.rs", 99, "from cofile"), rhit("c.rs", 5, "c cofile")],
    );
    assert_eq!(
        fused,
        [
            rhit("b.rs", 20, "b meaning"),
            rhit("a.rs", 10, "from meaning"),
            rhit("c.rs", 5, "c cofile"),
        ]
    );
}

#[test]
fn fuse_should_keep_ties_in_list_order_meaning_first() {
    let fused = fuse(
        &[lead("m1.rs", 1, "m1"), lead("m2.rs", 2, "m2")],
        &[rhit("c1.rs", 1, "c1"), rhit("c2.rs", 2, "c2")],
    );
    let paths: Vec<&str> = fused.iter().map(|hit| hit.path.as_str()).collect();
    assert_eq!(paths, ["m1.rs", "c1.rs", "m2.rs", "c2.rs"]);
}

#[test]
fn fuse_should_give_a_leadless_row_no_text_and_stop_at_the_file_cap() {
    let leads: Vec<MeaningHit> = (0..MAX_FILES + 3)
        .map(|n| lead(&format!("f{n}.rs"), 1, if n == 0 { "" } else { "x" }))
        .collect();
    let fused = fuse(&leads, &[]);
    assert_eq!(fused.len(), MAX_FILES);
    assert_eq!(fused[0], rhit_no_text("f0.rs", 1));
    assert_eq!(fused[MAX_FILES - 1].path, format!("f{}.rs", MAX_FILES - 1));
    assert!(fuse(&[], &[]).is_empty());
}

#[test]
fn fuse_should_never_name_a_credential_shaped_path() {
    let fused = fuse(
        &[lead(".aws/credentials", 1, "key"), lead("src/a.rs", 2, "a")],
        &[rhit(".env", 3, "TOKEN=1")],
    );
    assert_eq!(fused, [rhit("src/a.rs", 2, "a")]);
}

#[test]
fn rendering_should_not_claim_lexical_confidence() {
    let text = render(&full_brief()).unwrap();
    assert!(!text.contains("confidence:"), "{text}");
}

#[test]
fn byte_cap_should_give_a_prose_brief_its_own_bound() {
    assert_eq!(
        byte_cap(Some(Signal::Prose), false),
        PLAIN_PROSE_BRIEF_BYTES
    );
    assert_eq!(byte_cap(Some(Signal::Prose), true), PROSE_BRIEF_BYTES);
    assert_eq!(byte_cap(Some(Signal::Weak), true), PROSE_BRIEF_BYTES);
    assert_eq!(
        byte_cap(Some(Signal::Strong), true),
        STRONG_READ_BRIEF_BYTES
    );
    assert_eq!(byte_cap(Some(Signal::Strong), false), BRIEF_BYTES);
    assert_eq!(byte_cap(Some(Signal::Weak), false), BRIEF_BYTES);
    assert_eq!(byte_cap(None, false), BRIEF_BYTES);
    assert_eq!(PLAIN_PROSE_BRIEF_BYTES, 2048);
}

#[test]
fn a_prose_brief_should_fit_its_cap() {
    let mut brief = full_brief();
    brief.signal = Some(Signal::Prose);
    brief.files = (0..30)
        .map(|n| {
            rhit(
                &format!("crates/long/path/number/{n}/file.rs"),
                n + 1,
                "some matched text on the line",
            )
        })
        .collect();
    let text = render(&brief).unwrap();
    assert!(text.len() <= PLAIN_PROSE_BRIEF_BYTES, "{}", text.len());
    assert!(text.contains("(+"), "{text}");
}

#[test]
fn admission_should_name_itself_and_its_reason_for_the_log() {
    assert_eq!(Admission::default(), Admission::Unjudged);
    for (admission, label, reason) in [
        (Admission::Unjudged, "unjudged", None),
        (Admission::Open, "open", None),
    ] {
        assert_eq!(admission.label(), label);
        assert_eq!(admission.reason(), reason);
    }
}

#[test]
fn declined_should_name_why_no_brief_started() {
    let open = |_: Instant| -> Box<dyn Evidence> { Box::new(Fake::new()) };
    let why = |prompt: &str| {
        try_start_with(prompt, SECOND, open, model_high, Around::default())
            .err()
            .map(Declined::as_str)
    };
    assert_eq!(why("ok"), None);
    // Codex's envelopes are the harness speaking, whatever their body names.
    let delegation = [
        "<realtime_delegation>",
        "  <input>where is fetchUser</input>",
        "</realtime_delegation>",
    ]
    .join("\n");
    let reply = [
        "<send_user_message_question_reply>",
        "[{\"question\":\"where is fetchUser\",\"answer\":\"src/api.rs\"}]",
        "</send_user_message_question_reply>",
    ]
    .join("\n");
    assert_eq!(why(&delegation), Some("continuation"));
    assert_eq!(why(&reply), Some("continuation"));
    assert_eq!(why("thanks, that works"), None);
    assert_eq!(why("commit and push"), None);
    assert_eq!(why("callers of `fetchUser`"), None);
    assert_eq!(why(PROSE), None);
}

#[test]
fn an_expired_shared_deadline_should_not_start_the_evidence_worker() {
    let fake = Fake::new();
    let log = Arc::clone(&fake.log);
    let result = try_start_with(
        "how does the service start",
        Duration::ZERO,
        move |_| Box::new(fake),
        model_high,
        Around::default(),
    );
    assert_eq!(result.err(), Some(Declined::DeadlineExpired));
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn an_expired_absolute_deadline_should_not_be_reset_by_a_fresh_relative_window() {
    let root = scratch_dir("expired-absolute-deadline");
    let request = Request {
        prompt: "how does the service start",
        intent: None,
        root: &root,
        command: true,
        parts: 1,
        window_ms: Some(750),
        deadline_unix_ms: Some(now_ms().saturating_sub(1)),
        session: None,
        french: false,
        steps: Steps::default(),
        probes: None,
    };
    let result = start(&request, |_| Box::new(Fake::new()));
    assert_eq!(result.err(), Some(Declined::DeadlineExpired));
}

#[test]
fn confident_none_should_suppress_a_code_shaped_prompt_before_any_evidence_or_probe() {
    let fake = Fake::new();
    let log = Arc::clone(&fake.log);
    let intent = serde_json::json!({
        "predicted": "none",
        "probs": {"none": 0.7, "question": 0.2, "investigate": 0.1},
        "snapshot": {"provider": "jev", "model": "configured-model", "confidence": 0.81}
    });
    let declined = try_start_with(
        "where is `parse_config`",
        SECOND,
        move |_| Box::new(fake),
        model_high,
        Around {
            intent: Some(intent.clone()),
            ..Around::default()
        },
    )
    .err();
    assert_eq!(declined, Some(Declined::IntentNone));
    assert!(log.lock().unwrap().is_empty());
    let record = declined_record(
        Declined::IntentNone,
        "where is `parse_config`",
        Some(&intent),
    );
    assert!(record.enforced);
    assert_eq!(
        record.intent,
        Some(serde_json::json!({
            "label": "none", "probability": 0.7, "provider": "jev",
            "model": "configured-model", "confidence": 0.81
        }))
    );
}

#[test]
fn absent_invalid_or_unconfident_intent_should_fail_open_without_keyword_admission() {
    for intent in [
        None,
        Some(serde_json::json!({"predicted": "none", "probs": {"none": 0.49, "question": 0.51}})),
        Some(serde_json::json!({"predicted": "none", "probs": {"none": 1.01}})),
        Some(serde_json::json!({"predicted": "not-a-label", "probs": {"not-a-label": 1.0}})),
    ] {
        let mut fake = Fake::new();
        fake.relevance = Ok(off_topic_answer());
        fake.meaning = Ok(vec![lead("src/flarnix.rs", 3, "pub fn quibbler()")]);
        let finished = try_start_with(
            "How does the flarnix quibbler work?",
            SECOND,
            move |_| Box::new(fake),
            model_off,
            Around {
                intent,
                ..Around::default()
            },
        )
        .expect("no confident none declines the task")
        .finish_with_record();
        assert_eq!(finished.record.gate, "open");
        assert!(finished.text.is_some());
    }
}

#[test]
fn declined_should_spell_every_reason_for_the_log() {
    let spelled: Vec<&str> = [
        Declined::Disabled,
        Declined::Unindexed,
        Declined::Continuation,
        Declined::IntentNone,
        Declined::NotAboutCode,
        Declined::NoWorker,
        Declined::DeadlineExpired,
    ]
    .into_iter()
    .map(Declined::as_str)
    .collect();
    assert_eq!(
        spelled,
        [
            "disabled",
            "unindexed",
            "continuation",
            "intent_none",
            "not_about_code",
            "no_worker",
            "deadline_expired"
        ]
    );
}

#[test]
fn declined_record_should_carry_the_typed_text_only() {
    let record = declined_record(
        Declined::NotAboutCode,
        "<pasted_content>secret thread</pasted_content> thanks, that works",
        None,
    );
    assert_eq!(record.gate, "declined");
    assert_eq!(record.reason.as_deref(), Some("not_about_code"));
    assert_eq!(record.typed, "thanks, that works");
    assert_eq!(
        record.sha256,
        pixel_metrics::journal::sha256_hex("thanks, that works")
    );
    assert_eq!(record.signal, None);
    assert_eq!((record.ops, record.answered, record.bytes), (0, 0, 0));
    assert!(record.ts_ms > 1_577_836_800_000, "{}", record.ts_ms);
}

/// A prompt that is only a pasted block: what the thread is about, a log
/// nobody asks about, and what is wanted of it.
fn pasted_only_prompt() -> String {
    [
        "<pasted_content id=\"1\">",
        PROSE,
        "",
        "a log that matters little",
        "",
        "and make the watcher wait for the first scan",
        "</pasted_content id=\"1\">",
    ]
    .join("\n")
}

#[test]
fn a_prompt_that_is_only_a_pasted_block_should_be_briefed_on_its_head_and_tail() {
    let prompt = pasted_only_prompt();
    let task = [PROSE, "and make the watcher wait for the first scan"].join("\n");
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.meaning = Ok(vec![lead(DAEMON, 280, "fn watch_ready() {")]);
    let fake = Arc::new(fake);
    let seen = Arc::clone(&fake);
    let finished = try_start_with(
        &prompt,
        SECOND,
        move |_| Box::new(Shared(fake)),
        model_high,
        Around::default(),
    )
    .expect("a paste with a task starts a brief")
    .finish_with_record();
    // The probes read the task, never the log between its two ends.
    assert_eq!(
        sorted(seen.calls()),
        [
            format!("meaning {task} limit 8"),
            format!("relevance {task}")
        ]
    );
    let text = finished.text.expect("an on-topic paste is briefed");
    assert!(text.contains(DAEMON), "{text}");
    let record = finished.record;
    assert_eq!(
        (record.signal, record.gate, record.enforced),
        (Some("prose"), "open", false)
    );
    assert_eq!(record.typed, task);
    assert_eq!(record.sha256, pixel_metrics::journal::sha256_hex(&task));
}

#[test]
fn a_pasted_block_task_should_reach_the_log_masked() {
    let mut off = Fake::new();
    off.relevance = Ok(off_topic_answer());
    let closed = briefed_with(&pasted_only_prompt(), off, model_off);
    assert!(closed.text.is_some());
    assert_eq!(
        (closed.record.signal, closed.record.gate),
        (Some("prose"), "open")
    );
    // A credential in the paste is masked in the log like one typed.
    let leaking = [
        "<pasted_content>",
        "the daemon misses changes, the key is sk-abcdef123456 here",
        "</pasted_content>",
    ]
    .join("\n");
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    let logged = briefed(&leaking, fake).record.typed;
    assert_eq!(
        logged,
        "the daemon misses changes, the key is <redacted> here"
    );
}

#[test]
fn a_pasted_block_beside_typed_words_should_still_classify_the_task() {
    let beside =
        "<pasted_content>the daemon misses changes made during startup</pasted_content> thanks";
    let declined = try_start_with(
        beside,
        SECOND,
        |_| Box::new(Fake::new()),
        model_high,
        Around::default(),
    );
    assert_eq!(declined.err(), None);
    // Its record keeps the typed words only: the paste is not logged.
    let record = declined_record(Declined::NotAboutCode, beside, None);
    assert_eq!(record.typed, "thanks");
}

#[test]
fn a_decision_should_be_stamped_with_the_clock_and_the_time_it_took() {
    let before = now_ms();
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.pause = Duration::from_millis(30);
    let finished = briefed(PROSE, fake);
    let after = now_ms();
    let record = finished.record;
    assert!(
        (before..=after).contains(&record.ts_ms),
        "{before} <= {} <= {after}",
        record.ts_ms
    );
    assert!(record.elapsed_ms >= 30, "{}", record.elapsed_ms);
    assert!(record.elapsed_ms < 1000, "{}", record.elapsed_ms);
}

#[test]
fn a_brief_its_window_cut_should_log_cut_and_the_deadline_it_was_given() {
    let mut fake = Fake::new();
    fake.pause = Duration::from_millis(300);
    fake.files = Ok(found(vec![hit("src/a.ts", 2)]));
    let started = Instant::now();
    let record = try_start_with(
        "callers of `fetchUser`",
        Duration::from_millis(60),
        move |_| Box::new(fake),
        model_high,
        Around::default(),
    )
    .expect("the prompt starts a brief")
    .finish_with_record()
    .record;
    assert!(started.elapsed() < Duration::from_millis(250));
    // Without these two, a brief that lost its evidence to the window
    // reads like a complete one in the log.
    assert!(record.cut);
    assert_eq!(record.deadline_ms, Some(60));
    // The worker was still in its first search when the window ended.
    assert_eq!(record.stages.search_ms, None);
    assert_eq!(record.stages.rank_ms, None);
    let value = record.to_json();
    assert_eq!(value["cut"], true);
    assert_eq!(value["deadline_ms"], 60);
}

#[test]
fn a_complete_brief_should_log_each_stage_it_ran_and_no_cut() {
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.pause = Duration::from_millis(30);
    let record = briefed(PROSE, fake).record;
    assert!(!record.cut);
    assert_eq!(record.deadline_ms, Some(1000));
    let search = record.stages.search_ms.expect("the probes searched");
    assert!((30..1000).contains(&search), "{search}");
    assert!(record.stages.rank_ms.is_some_and(|ms| ms < 1000));
    assert!(record.stages.render_ms.is_some_and(|ms| ms < 1000));
    assert!(
        search <= record.elapsed_ms,
        "{search} > {}",
        record.elapsed_ms
    );
}

#[test]
fn a_declined_prompt_should_log_no_window_and_no_stage() {
    let record = declined_record(Declined::NotAboutCode, "thanks", None);
    assert!(!record.cut);
    assert_eq!(record.deadline_ms, None);
    assert_eq!(record.stages, Stages::default());
}

#[test]
fn work_stages_should_split_the_worker_time_at_the_search_mark() {
    let started = Instant::now();
    let at = |ms| started + Duration::from_millis(ms);
    // Searched at 40, worker done at 100, collected at 120.
    assert_eq!(
        work_stages(started, Some(at(40)), Some(at(100)), at(120), true),
        (Some(40), Some(60))
    );
    // The window cut the worker: ranking ran until the brief was collected.
    assert_eq!(
        work_stages(started, Some(at(40)), None, at(120), true),
        (Some(40), Some(80))
    );
    // A worker mark after collection cannot stretch the stage.
    assert_eq!(
        work_stages(started, Some(at(40)), Some(at(200)), at(120), true),
        (Some(40), Some(80))
    );
    assert_eq!(
        work_stages(started, Some(at(40)), Some(at(100)), at(120), false),
        (Some(40), None)
    );
    assert_eq!(
        work_stages(started, None, Some(at(100)), at(120), true),
        (None, None)
    );
}

#[test]
fn millis_should_count_whole_milliseconds_and_saturate() {
    assert_eq!(millis(Duration::from_micros(2_999)), 2);
    assert_eq!(millis(Duration::MAX), u64::MAX);
}

#[test]
fn reciprocal_rank_should_count_ranks_from_one() {
    assert!((reciprocal_rank(0) - 1.0 / 61.0).abs() < 1e-12);
    assert!((reciprocal_rank(1) - 1.0 / 62.0).abs() < 1e-12);
    assert!((reciprocal_rank(7) - 1.0 / 68.0).abs() < 1e-12);
}

#[test]
fn a_weak_prompt_with_only_a_meaning_lead_should_show_it_without_a_confidence_line() {
    let mut fake = Fake::new();
    fake.meaning = Ok(vec![lead(
        "crates/pixel-daemon/src/api.rs",
        40,
        "fn op_status",
    )]);
    let finished = briefed("how does the login flow work", fake);
    let text = finished.text.unwrap();
    assert!(
        text.contains("\nfiles: crates/pixel-daemon/src/api.rs:40 — fn op_status\n"),
        "{text}"
    );
    assert!(!text.contains("confidence"), "{text}");
    assert!(text.contains("\ncoverage: 1/2 ops answered\n"), "{text}");
    assert_eq!(finished.record.gate, "unjudged");
    assert_eq!(finished.record.score, None);
}

#[test]
fn a_decision_should_name_where_its_evidence_came_from() {
    let finished = briefed(PROSE, Fake::new());
    assert_eq!(finished.record.route, Some("unknown"));
    let declined = declined_record(Declined::NotAboutCode, "hello there", None);
    assert_eq!(declined.route, None);
}

/// A daemon source file the fake can be asked for lines of.
fn daemon_source() -> Vec<String> {
    let mut lines: Vec<String> = (1..=278)
        .map(|n| format!("let filler_{n} = {n};"))
        .collect();
    lines.extend(
        [
            "/// Wait until the daemon answers its socket.",
            "pub fn watch_ready(root: &Path) -> bool {",
            "    let mut waited = 0;",
            "    while waited < READY_TRIES {",
            "        waited += 1;",
            "    }",
            "    waited < READY_TRIES",
            "}",
        ]
        .map(String::from),
    );
    lines
}

/// A weak plain-language prompt with retrieval results and source available.
fn confident_fake() -> Fake {
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.meaning = Ok(vec![lead(DAEMON, 280, "pub fn watch_ready")]);
    fake.sources = vec![(DAEMON.to_string(), daemon_source())];
    fake
}

const WEAK: &str = "how does the daemon handle changes made during startup";

#[test]
fn the_receipt_should_state_the_searches_that_ran_with_their_counts() {
    let finished = briefed(WEAK, confident_fake());
    let text = finished.text.unwrap();
    assert!(
        text.contains(
            "searched: content+symbols+paths for daemon, changes, startup (3 terms, 1000 files) · meaning search returned 1 chunks\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("result: the matches below are the best across both searches;"),
        "{text}"
    );
    assert!(finished.record.receipt);
}

#[test]
fn the_receipt_should_never_name_a_meaning_search_that_did_not_answer() {
    let mut fake = confident_fake();
    fake.meaning = Err("cold".into());
    let finished = briefed(WEAK, fake);
    let text = finished.text.unwrap();
    assert!(
        text.contains("searched: content+symbols+paths for"),
        "{text}"
    );
    assert!(!text.contains("meaning search"), "{text}");
    assert!(text.contains("across the search;"), "{text}");
}

#[test]
fn a_brief_without_a_relevance_answer_should_carry_no_receipt() {
    let mut fake = confident_fake();
    fake.relevance = Err("cold".into());
    let finished = briefed(WEAK, fake);
    let text = finished.text.unwrap_or_default();
    assert!(!text.contains("searched:"), "{text}");
    assert!(!finished.record.receipt);
}

#[test]
fn lexical_score_should_not_suppress_receipt_or_excerpt() {
    let finished = briefed_with(WEAK, confident_fake(), model_off);
    let text = finished.text.unwrap();
    assert!(text.contains("searched:"), "{text}");
    assert!(text.contains("\nanswer "), "{text}");
    assert!(finished.record.receipt && finished.record.answer);
    assert_eq!(finished.record.tier, None);
    assert_eq!(finished.record.model, None);
}

#[test]
fn a_strong_prompt_should_carry_neither_receipt_nor_excerpt() {
    let mut fake = confident_fake();
    fake.symbols = Ok(vec![symbol(DAEMON, "watch_ready")]);
    let finished = briefed("what does `watch_ready` do", fake);
    let text = finished.text.unwrap();
    assert!(
        !text.contains("searched:") && !text.contains("\nanswer "),
        "{text}"
    );
    assert!(text.len() <= BRIEF_BYTES);
}

#[test]
fn the_excerpt_should_hold_the_signature_the_doc_line_and_the_body_within_the_cap() {
    let finished = briefed(WEAK, confident_fake());
    let text = finished.text.unwrap();
    assert!(text.contains("\nanswer "), "{text}");
    assert!(
        text.contains("\nread: crates/pixel-daemon/src/daemon.rs:279-286 — watch_ready\n"),
        "{text}"
    );
    assert!(text.ends_with(READ_LINE), "{text}");
    assert!(!text.contains("confidence:"), "{text}");
    assert!(text.len() <= PROSE_BRIEF_BYTES, "{} bytes", text.len());
    assert!(finished.record.answer);
    assert!(finished.record.excerpt_bytes > 0);
    assert_eq!(finished.record.bytes, text.len());
}

#[test]
fn a_source_that_cannot_be_read_should_leave_the_brief_without_an_excerpt() {
    let mut fake = confident_fake();
    fake.sources.clear();
    let finished = briefed(WEAK, fake);
    let text = finished.text.unwrap();
    assert!(!text.contains("\nanswer "), "{text}");
    assert!(text.contains("searched:"), "{text}");
    assert!(!finished.record.answer);
    assert_eq!(finished.record.excerpt_bytes, 0);
}

#[test]
fn a_credential_shaped_file_should_never_be_excerpted() {
    let mut fake = confident_fake();
    fake.relevance = Ok(RelevanceAnswer {
        lines: vec![rhit("deploy/.env", 2, "TOKEN=x")],
        ..on_topic_answer()
    });
    fake.meaning = Ok(Vec::new());
    fake.sources = vec![(
        "deploy/.env".to_string(),
        vec!["TOKEN=x".into(), "B=2".into()],
    )];
    let finished = briefed(WEAK, fake);
    let text = finished.text.unwrap_or_default();
    assert!(
        !text.contains("\nanswer ") && !text.contains("TOKEN"),
        "{text}"
    );
}

/// [`answer_evidence`] on `brief` against `fake`.
fn answered(brief: Brief, fake: &Fake) -> Brief {
    let state = Mutex::new(brief);
    answer_evidence(fake, &state, Instant::now() + SECOND);
    state.into_inner().unwrap()
}

#[test]
fn a_flow_brief_should_list_the_hops_with_their_sites() {
    let mut fake = Fake::new();
    fake.symbols = Ok(vec![symbol("src/a.rs", "render_page")]);
    fake.source = Some("pub fn render_page() {".into());
    let mut brief = high_brief();
    brief.kind = Some(QuestionKind::Flow);
    brief.flow_hops = vec![
        "render_page".into(),
        "middleware".into(),
        "send_response".into(),
    ];
    let brief = answered(brief, &fake);
    assert_eq!(brief.excerpts.len(), 1);
    assert_eq!(brief.excerpts[0].label, "flow");
    assert_eq!(
        brief.excerpts[0].lines[0],
        "src/a.rs:3 render_page — pub fn render_page() {"
    );
    assert_eq!(brief.excerpts[0].lines.len(), 3);
    // The extra lookups are outside the coverage count.
    assert_eq!((brief.ops, brief.answered), (3, 3));
}

#[test]
fn a_flow_brief_whose_hops_do_not_resolve_should_fall_back_to_the_generic_excerpt() {
    let mut fake = Fake::new();
    fake.symbols = Ok(Vec::new());
    fake.sources = vec![(DAEMON.to_string(), daemon_source())];
    let mut brief = high_brief();
    brief.kind = Some(QuestionKind::Flow);
    brief.flow_hops = vec!["a".into(), "b".into()];
    brief.files = vec![rhit(DAEMON, 280, "pub fn watch_ready")];
    let brief = answered(brief, &fake);
    assert_eq!(brief.excerpts.len(), 1);
    assert_eq!(brief.excerpts[0].label, format!("{DAEMON}:280"));
}

#[test]
fn a_tests_brief_should_name_the_tests_with_their_first_assert() {
    let mut fake = Fake::new();
    fake.sources = vec![(
        "tests/ready.rs".to_string(),
        [
            "#[test]",
            "fn ready_after_startup() {",
            "    let up = watch_ready(root);",
            "    assert!(up);",
            "}",
        ]
        .map(String::from)
        .to_vec(),
    )];
    let mut brief = high_brief();
    brief.kind = Some(QuestionKind::Tests);
    brief.defined = vec![symbol("src/a.rs", "watch_ready")];
    brief.tests = vec!["tests/ready.rs".into()];
    let brief = answered(brief, &fake);
    assert_eq!(brief.excerpts.len(), 1);
    assert_eq!(brief.excerpts[0].label, "tests");
    assert_eq!(
        brief.excerpts[0].lines,
        ["tests/ready.rs:2 ready_after_startup — assert!(up);"]
    );
}

#[test]
fn a_config_brief_should_show_the_setting_line_with_its_default_and_where_it_is_read() {
    let mut fake = Fake::new();
    fake.sources = vec![(
        "src/cfg.rs".to_string(),
        vec![
            "/// Tries before the daemon is called late.".into(),
            "pub const READY_TRIES: u32 = 40;".into(),
        ],
    )];
    let mut brief = high_brief();
    brief.kind = Some(QuestionKind::Config);
    brief.receipt = Receipt::new(
        &RelevanceInput {
            keywords: vec![keyword("tries", 2.0)],
            ..on_topic_answer().input
        },
        None,
    );
    brief.files = vec![
        rhit("src/cfg.rs", 2, "pub const READY_TRIES: u32 = 40;"),
        rhit("src/run.rs", 9, "let n = std::env::var(READY_TRIES).ok();"),
        rhit("src/other.rs", 1, "pub const UNRELATED_LIMIT: u32 = 1;"),
    ];
    let brief = answered(brief, &fake);
    assert_eq!(brief.excerpts.len(), 2);
    assert_eq!(brief.excerpts[0].label, "config");
    assert_eq!(
        brief.excerpts[0].lines,
        [
            "src/cfg.rs:2 — pub const READY_TRIES: u32 = 40;",
            "src/run.rs:9 — let n = std::env::var(READY_TRIES).ok();"
        ]
    );
    // The generic excerpt of the best file follows the kind's block.
    assert_eq!(brief.excerpts[1].label, "src/cfg.rs:2");
    assert_eq!(
        brief.excerpts[1].lines[0],
        "1| /// Tries before the daemon is called late."
    );
}

#[test]
fn answer_evidence_should_skip_a_rationale_a_maybe_a_strong_prompt_and_a_gap() {
    let mut fake = Fake::new();
    fake.sources = vec![(DAEMON.to_string(), daemon_source())];
    let ready = || {
        let mut brief = high_brief();
        brief.files = vec![rhit(DAEMON, 280, "pub fn watch_ready")];
        brief.excerpts.clear();
        brief
    };
    assert_eq!(answered(ready(), &fake).excerpts.len(), 1);
    let mut rationale = ready();
    rationale.kind = Some(QuestionKind::Rationale);
    let mut strong = ready();
    strong.signal = Some(Signal::Strong);
    let mut cut = ready();
    cut.cut = true;
    for (name, brief) in [("rationale", rationale), ("strong", strong), ("cut", cut)] {
        assert!(answered(brief, &fake).excerpts.is_empty(), "{name}");
    }
}

#[test]
fn the_generic_excerpt_should_prefer_code_over_tests_that_quote_the_question() {
    for test in [
        "tests/a.rs",
        "src/a_test.rs",
        "web/a.test.ts",
        "src/tests.rs",
        "test_a.py",
    ] {
        assert!(answer::is_test_path(test), "{test}");
    }
    for code in ["src/a.rs", "src/latest.rs", "docs/contest.md"] {
        assert!(!answer::is_test_path(code), "{code}");
    }
    let mut fake = Fake::new();
    fake.sources = vec![
        ("tests/a.rs".to_string(), vec!["fn quoted() {}".into()]),
        ("src/b.rs".to_string(), vec!["fn b() {}".into()]),
        ("src/c.rs".to_string(), vec!["fn c() {}".into()]),
    ];
    let mut brief = high_brief();
    brief.files = vec![
        rhit("tests/a.rs", 1, "x"),
        rhit("src/b.rs", 1, "x"),
        rhit("src/c.rs", 1, "x"),
    ];
    let labels: Vec<String> = answered(brief, &fake)
        .excerpts
        .into_iter()
        .map(|e| e.label)
        .collect();
    assert_eq!(labels, ["src/b.rs:1", "src/c.rs:1"]);
}

#[test]
fn the_generic_excerpt_should_cover_two_distinct_files_and_skip_the_packed_definition() {
    let mut fake = Fake::new();
    fake.sources = vec![
        (DAEMON.to_string(), daemon_source()),
        (
            "b.rs".to_string(),
            vec!["fn b() {".into(), "    go();".into(), "}".into()],
        ),
        ("c.rs".to_string(), vec!["fn c() {}".into()]),
    ];
    let mut brief = high_brief();
    brief.files = vec![
        rhit(DAEMON, 280, "x"),
        rhit(DAEMON, 281, "x"),
        rhit("b.rs", 1, "x"),
        rhit("c.rs", 1, "x"),
    ];
    let kept = answered(brief.clone(), &fake);
    let labels: Vec<&str> = kept.excerpts.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(
        labels,
        [format!("{DAEMON}:280").as_str(), "b.rs:1", "c.rs:1"]
    );
    brief.defined = vec![symbol(DAEMON, "watch_ready")];
    brief.def_body = Some("fn watch_ready() {}".into());
    let packed = answered(brief, &fake);
    let labels: Vec<&str> = packed.excerpts.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["b.rs:1", "c.rs:1"]);
}

#[test]
fn a_rationale_brief_should_carry_the_receipt_and_no_excerpt() {
    let mut fake = confident_fake();
    fake.status = Ok(StatusProbe {
        facts_fresh: Some(true),
    });
    fake.history = Ok((vec![history("abc1234", "retry on startup")], Vec::new()));
    let finished = briefed("why does the daemon retry during startup", fake);
    let text = finished.text.unwrap();
    assert!(text.contains("searched:"), "{text}");
    assert!(!text.contains("\nanswer "), "{text}");
}

fn high_brief() -> Brief {
    let hit = rhit("src/a.rs", 3, "fn a");
    Brief {
        signal: Some(Signal::Prose),
        receipt: Receipt::new(&on_topic_answer().input, Some(2)),
        files: vec![hit.clone(); 6],
        excerpts: (0..2)
            .map(|n| Excerpt {
                label: format!("src/f{n}.rs:3"),
                path: format!("src/f{n}.rs"),
                from_meaning: true,
                meaning_rank: Some(n),
                density: 3,
                lines: (0..10)
                    .map(|line| format!("{line}| {}", "x".repeat(105)))
                    .collect(),
            })
            .collect(),
        ops: 3,
        answered: 3,
        finished: true,
        ..Brief::default()
    }
}

#[test]
fn an_oversized_brief_should_drop_the_second_excerpt_first_and_keep_the_receipt() {
    let mut brief = high_brief();
    brief.caps = vec!["c".repeat(110); 10];
    let (text, stats) = fit(&brief).unwrap();
    assert!(text.len() <= PROSE_BRIEF_BYTES, "{}", text.len());
    assert!(
        text.contains("searched:") && text.contains("result:"),
        "{text}"
    );
    assert!(stats.receipt && stats.answer, "{stats:?}");
    let count = |label: &str| {
        text.lines()
            .skip_while(|line| !line.starts_with(&format!("answer {label}")))
            .skip(1)
            .take_while(|line| line.starts_with("  "))
            .count()
    };
    let (first, second) = (count("src/f0.rs:3"), count("src/f1.rs:3"));
    // Whatever had to go went from the second block before the first.
    assert!(second < 10, "{text}");
    assert!(second == 0 || first == 10, "{text}");
}

#[test]
fn a_tight_cap_should_empty_the_excerpts_before_it_touches_the_receipt() {
    let mut brief = high_brief();
    brief.files = (0..4)
        .map(|n| rhit(&format!("crates/long/{n}.rs"), 1, &"t".repeat(110)))
        .collect();
    brief.def_body = Some("b".repeat(480));
    brief.defined = vec![symbol("src/handle.ts", "handle")];
    brief.callers = (0..10)
        .map(|n| caller("p.rs", &format!("via{n}"), n))
        .collect();
    let (text, stats) = fit(&brief).unwrap();
    assert!(stats.receipt, "{text}");
    assert!(text.contains("result: the matches below"), "{text}");
    if !stats.answer {
        assert_eq!(stats.excerpt_bytes, 0);
        assert!(!text.contains("\nanswer "), "{text}");
    }
}

#[test]
fn a_cut_packet_should_keep_bounded_source_excerpts_without_a_receipt() {
    let mut brief = high_brief();
    brief.cut = true;
    let (text, stats) = fit(&brief).unwrap();
    assert!(
        !text.contains("searched:") && text.contains("\nanswer "),
        "{text}"
    );
    assert!(stats.answer);
    assert!(!stats.receipt);
    assert!(stats.excerpt_bytes > 0);
}

fn lead_at(path: &str, line: u32, symbol: Option<&str>) -> MeaningHit {
    MeaningHit {
        symbol: symbol.map(String::from),
        ..lead(path, line, "chunk")
    }
}

fn two_file_fake() -> Fake {
    let mut fake = Fake::new();
    fake.sources = vec![
        (DAEMON.to_string(), daemon_source()),
        (
            "crates/pixel/src/other.rs".to_string(),
            vec!["fn other() {".into(), "    go();".into(), "}".into()],
        ),
    ];
    fake
}

#[test]
fn the_excerpt_should_come_from_the_meaning_chunk_not_the_lexical_line() {
    let mut brief = high_brief();
    brief.leads = vec![lead_at(DAEMON, 280, None)];
    brief.files = vec![
        rhit(DAEMON, 3, "filler"),
        rhit("crates/pixel/src/other.rs", 1, "go"),
    ];
    let brief = answered(brief, &two_file_fake());
    assert_eq!(brief.excerpts[0].label, format!("{DAEMON}:280"));
    assert!(brief.excerpts[0].from_meaning);
    let other = brief
        .excerpts
        .iter()
        .find(|excerpt| excerpt.path.ends_with("other.rs"))
        .expect("the second file speaks too");
    assert_eq!(other.label, "crates/pixel/src/other.rs:1");
    assert!(!other.from_meaning);
}

#[test]
fn the_file_ranked_first_should_speak_first_even_against_a_better_scored_chunk() {
    let mut brief = high_brief();
    brief.leads = vec![lead_at(DAEMON, 280, None)];
    brief.files = vec![
        rhit("crates/pixel/src/other.rs", 1, "go"),
        rhit(DAEMON, 280, "x"),
    ];
    let brief = answered(brief, &two_file_fake());
    assert_eq!(brief.excerpts[0].label, "crates/pixel/src/other.rs:1");
}

#[test]
fn excerpts_should_skip_tests_docs_and_test_modules_unless_the_question_asks() {
    let mut fake = two_file_fake();
    let mut inside: Vec<String> = vec![
        "fn real() {}".into(),
        "#[cfg(test)]".into(),
        "mod tests {".into(),
    ];
    inside.extend((0..15).map(|n| format!("    fn t{n}() {{}}")));
    fake.sources
        .push(("src/mod_with_tests.rs".to_string(), inside));
    for path in [
        "tests/a.rs",
        "docs/guide.md",
        "CHANGELOG.md",
        "src/lib_test.rs",
    ] {
        fake.sources
            .push((path.to_string(), vec!["fn x() {}".into()]));
    }
    let mut brief = high_brief();
    brief.leads = vec![
        lead_at("tests/a.rs", 1, None),
        lead_at("docs/guide.md", 1, None),
        lead_at("CHANGELOG.md", 1, None),
        lead_at("src/lib_test.rs", 1, None),
        lead_at("crates/pixel/src/other.rs", 17, Some("render_should_wrap")),
        lead_at("src/mod_with_tests.rs", 10, None),
        lead_at(DAEMON, 280, Some("watch_ready")),
    ];
    brief.files.clear();
    let kept = answered(brief.clone(), &fake);
    let paths: Vec<&str> = kept.excerpts.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, [DAEMON]);
    // A question about tests or docs may cut from them.
    brief.kind = Some(QuestionKind::Tests);
    brief.defined = Vec::new();
    let tests = answered(brief, &fake);
    assert!(
        tests.excerpts.iter().any(|e| e.path == "tests/a.rs"),
        "{:?}",
        tests.excerpts
    );
}

#[test]
fn a_partial_brief_should_name_its_gap_and_still_end_on_the_instruction() {
    let mut brief = full_brief();
    brief.unresolved = vec!["find-symbol x: 2 candidates".into()];
    brief.next = Some("pixel find-code 'x'".into());
    brief.reads = vec![ReadRange {
        path: "src/a.rs".into(),
        first: 1,
        last: 3,
        symbol: None,
    }];
    let (text, _) = fit(&brief).unwrap();
    assert!(text.contains("packet partial"), "{text}");
    assert!(text.contains("\nnext: pixel find-code 'x'\n"), "{text}");
    assert!(text.ends_with(READ_LINE), "{text}");
    assert!(!text.contains("answer from them"), "{text}");
}

#[test]
fn a_flow_or_tests_question_should_borrow_the_symbol_of_the_best_non_test_chunk() {
    let mut brief = high_brief();
    brief.leads = vec![
        lead_at("tests/a.rs", 3, Some("a_test_symbol")),
        lead_at(DAEMON, 280, Some("watch_ready")),
    ];
    let state = Mutex::new(brief);
    let plan = |kind, text: &str| Plan {
        anchors: Anchors::from_text(text),
        change_intent: false,
        kind,
        typed: text.to_string(),
        concept: None,
        steps: Steps::default(),
    };
    let seeded_flow = seeded(plan(QuestionKind::Flow, "what calls the watchdog"), &state);
    assert_eq!(
        seeded_flow.anchors.symbol_name().as_deref(),
        Some("watch_ready")
    );
    // Another kind, or a question that names its own symbol, keeps its anchors.
    let lookup = seeded(plan(QuestionKind::Lookup, "what is the watchdog"), &state);
    assert!(lookup.anchors.names().next().is_none());
    let named = seeded(plan(QuestionKind::Flow, "who calls `retry_loop`"), &state);
    assert_eq!(named.anchors.names().collect::<Vec<_>>(), ["retry_loop"]);
    let cold = seeded(
        plan(QuestionKind::Tests, "which tests cover the watchdog"),
        &state,
    );
    assert_eq!(cold.anchors.symbol_name().as_deref(), Some("watch_ready"));
}

#[test]
fn a_one_endpoint_flow_brief_should_list_the_callers_as_its_hops() {
    let mut fake = Fake::new();
    fake.source = Some("fn run() { watch_ready() }".into());
    let mut brief = high_brief();
    brief.kind = Some(QuestionKind::Flow);
    brief.callers = vec![
        caller("src/run.rs", "run", 12),
        caller("src/boot.rs", "boot", 7),
    ];
    let brief = answered(brief, &fake);
    assert_eq!(brief.excerpts[0].label, "callers");
    assert_eq!(
        brief.excerpts[0].lines,
        [
            "src/run.rs:12 run — fn run() { watch_ready() }",
            "src/boot.rs:7 boot — fn run() { watch_ready() }"
        ]
    );
}

fn files_line(text: &str) -> String {
    text.lines()
        .find(|line| line.starts_with("files: "))
        .unwrap_or_default()
        .to_string()
}

fn ranked_fake() -> Fake {
    let mut fake = Fake::new();
    fake.relevance = Ok(RelevanceAnswer {
        lines: vec![
            rhit("tests/a.rs", 1, "x"),
            rhit("docs/x.md", 1, "x"),
            rhit("src/b.rs", 1, "x"),
            rhit("eval/run.py", 1, "x"),
            rhit("src/c.rs", 1, "x"),
        ],
        ..on_topic_answer()
    });
    fake.meaning = Err("cold".into());
    fake
}

#[test]
fn the_file_list_should_rank_tests_docs_and_eval_after_the_code_without_dropping_them() {
    let finished = briefed(WEAK, ranked_fake());
    let files = files_line(&finished.text.unwrap());
    let at = |path: &str| {
        files
            .find(path)
            .unwrap_or_else(|| panic!("{path} in {files}"))
    };
    assert!(at("src/b.rs") < at("src/c.rs"), "{files}");
    for later in ["tests/a.rs", "docs/x.md", "eval/run.py"] {
        assert!(at("src/c.rs") < at(later), "{files}");
    }
    // The demoted keep their own order.
    assert!(at("tests/a.rs") < at("docs/x.md") && at("docs/x.md") < at("eval/run.py"));
}

#[test]
fn a_question_about_tests_or_docs_should_keep_those_files_in_place() {
    let finished = briefed(
        "which tests cover how the daemon handles startup changes",
        ranked_fake(),
    );
    let files = files_line(&finished.text.unwrap());
    assert!(files.find("tests/a.rs") < files.find("src/b.rs"), "{files}");
    // Docs stay demoted when only tests were asked for.
    assert!(files.find("src/c.rs") < files.find("docs/x.md"), "{files}");
    let finished = briefed(
        "what do the docs say about how the daemon handles startup changes",
        ranked_fake(),
    );
    let files = files_line(&finished.text.unwrap());
    assert!(files.find("docs/x.md") < files.find("src/b.rs"), "{files}");
    assert!(files.find("src/c.rs") < files.find("tests/a.rs"), "{files}");
}

#[test]
fn the_best_chunk_should_be_the_one_the_meaning_search_and_the_keywords_both_favour() {
    let mut source: Vec<String> = (1..=90).map(|n| format!("let filler_{n} = {n};")).collect();
    for (line, name) in [(10, "a_chunk"), (40, "b_chunk"), (70, "c_chunk")] {
        source[line - 1] = format!("fn {name}() {{");
    }
    source[44] = "    daemon.notice(changes_during_startup);".into();
    source[74] = "    daemon.notice(changes);".into();
    let mut fake = Fake::new();
    fake.sources = vec![(DAEMON.to_string(), source)];
    let mut brief = high_brief();
    brief.leads = [(10, 20), (40, 50), (70, 80)]
        .iter()
        .map(|&(start, end)| MeaningHit {
            end_line: end,
            ..lead_at(DAEMON, start, None)
        })
        .collect();
    brief.files = vec![rhit(DAEMON, 10, "x")];
    brief.excerpts.clear();
    let brief = answered(brief, &fake);
    let labels: Vec<&str> = brief.excerpts.iter().map(|e| e.label.as_str()).collect();
    // Meaning ranks a, b, c; keyword density ranks b, c, a: b wins, and
    // the file of the best chunk may show all three.
    assert_eq!(
        labels,
        [
            format!("{DAEMON}:40"),
            format!("{DAEMON}:10"),
            format!("{DAEMON}:70")
        ]
    );
    let first = brief.excerpts[0].lines.join("\n");
    assert!(first.contains("changes_during_startup"), "{first}");
    assert!(
        brief
            .excerpts
            .iter()
            .all(|e| e.lines.len() <= 2 + answer::CHUNK_BODY_LINES)
    );
}

#[test]
fn only_the_file_of_the_best_chunk_may_show_more_than_one_chunk() {
    let mut fake = two_file_fake();
    fake.sources[1].1 = (1..=30).map(|n| format!("fn other_{n}() {{}}")).collect();
    let mut brief = high_brief();
    brief.leads = vec![
        lead_at(DAEMON, 280, None),
        lead_at("crates/pixel/src/other.rs", 2, None),
        lead_at("crates/pixel/src/other.rs", 20, None),
    ];
    brief.files = vec![
        rhit(DAEMON, 280, "x"),
        rhit("crates/pixel/src/other.rs", 2, "x"),
    ];
    brief.excerpts.clear();
    let brief = answered(brief, &fake);
    let other = brief
        .excerpts
        .iter()
        .filter(|e| e.path.ends_with("other.rs"))
        .count();
    assert_eq!(other, 1, "{:?}", brief.excerpts);
}

fn function_file() -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (name, body) in [
        ("alpha", "unrelated work"),
        ("beta", "daemon startup changes"),
        ("gamma", "other things"),
    ] {
        lines.push(format!("/// Does {body}."));
        lines.push(format!("pub fn {name}() {{"));
        for n in 0..10 {
            lines.push(format!("    step_{name}_{n}(); // {body}"));
        }
        lines.push("}".into());
        lines.push(String::new());
    }
    lines
}

#[test]
fn the_range_should_be_the_function_whose_lines_hold_the_most_probed_keywords() {
    let mut fake = Fake::new();
    fake.sources = vec![
        (DAEMON.to_string(), function_file()),
        ("crates/pixel/src/other.rs".to_string(), function_file()),
        ("docs/guide.md".to_string(), function_file()),
    ];
    let mut brief = high_brief();
    brief.files = vec![
        rhit("docs/guide.md", 1, "x"),
        rhit(DAEMON, 1, "x"),
        rhit(DAEMON, 20, "x"),
        rhit("crates/pixel/src/other.rs", 1, "x"),
    ];
    let terms: Vec<String> = ["daemon", "startup", "changes"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let reads = read_evidence(&fake, &brief, &terms, Instant::now() + SECOND);
    // The guide is demoted; one range per file; each the `beta` function.
    let paths: Vec<&str> = reads.iter().map(|read| read.path.as_str()).collect();
    assert_eq!(paths, [DAEMON, "crates/pixel/src/other.rs"]);
    for read in &reads {
        assert_eq!(read.symbol.as_deref(), Some("beta"), "{read:?}");
        assert!(read.first <= 15 && read.last >= 25, "{read:?}");
        assert!(read.last - read.first < 60);
    }
    // A question about docs keeps the guide.
    brief.wants_docs = true;
    let reads = read_evidence(&fake, &brief, &terms, Instant::now() + SECOND);
    assert_eq!(reads[0].path, "docs/guide.md");
}

#[test]
fn a_strong_prompt_should_read_its_definition_then_its_callers() {
    let mut fake = Fake::new();
    fake.sources = vec![(
        "src/caller.rs".to_string(),
        vec![
            "fn caller() {".into(),
            "    watch_ready();".into(),
            "}".into(),
        ],
    )];
    let mut brief = Brief {
        signal: Some(Signal::Strong),
        finished: true,
        answered: 1,
        ..Brief::default()
    };
    brief.defined = vec![SymbolHit {
        start_line: 280,
        end_line: 400,
        ..symbol(DAEMON, "watch_ready")
    }];
    brief.callers = vec![caller("src/caller.rs", "caller", 2)];
    let plan = Plan::from_prompt("who calls `watch_ready`").unwrap();
    let state = Mutex::new(brief);
    strong_reads(&plan, &fake, &state, Instant::now() + SECOND);
    let brief = state.into_inner().unwrap();
    let text = fit(&brief).unwrap().0;
    assert!(
        text.contains(&format!("\nread: {DAEMON}:280-339 — watch_ready\n")),
        "{text}"
    );
    assert!(
        text.contains("\nalso: src/caller.rs:1-3 — caller\n"),
        "{text}"
    );
    assert!(text.ends_with(READ_LINE), "{text}");
    // No resolved definition, nothing to read.
    let none = Mutex::new(Brief {
        signal: Some(Signal::Strong),
        ..Brief::default()
    });
    strong_reads(&plan, &fake, &none, Instant::now() + SECOND);
    assert!(none.into_inner().unwrap().reads.is_empty());
}

#[test]
fn every_brief_that_ends_on_ranges_should_say_to_search_as_usual_if_they_fail() {
    assert_eq!(
        READ_LINE,
        "Read these ranges (in parallel, one turn) before any search. If they don't answer, search as usual."
    );
    let range = || ReadRange {
        path: "a.rs".into(),
        first: 1,
        last: 9,
        symbol: None,
    };
    let mut high = full_brief();
    high.reads = vec![range()];
    let mut strong = Brief {
        signal: Some(Signal::Strong),
        finished: true,
        answered: 1,
        reads: vec![range()],
        ..Brief::default()
    };
    strong.defined = vec![symbol("a.rs", "f")];
    for (name, brief) in [("prose", high), ("strong", strong)] {
        let (text, _) = fit(&brief).unwrap();
        assert!(text.ends_with(READ_LINE), "{name}: {text}");
        assert!(
            text.contains("read: a.rs:1-9") || text.contains("read: "),
            "{name}"
        );
    }
}

#[test]
fn a_code_shaped_prompt_with_one_short_definition_can_show_it_as_the_answer() {
    let mut fake = Fake::new();
    let body: Vec<String> = (0..5).map(|n| format!("    step_{n}();")).collect();
    fake.sources = vec![("src/f.rs".to_string(), {
        let mut lines = vec!["fn before() {}".to_string()];
        lines.push("pub fn f() {".into());
        lines.extend(body);
        lines.push("}".into());
        lines
    })];
    let mut brief = Brief {
        signal: Some(Signal::Strong),
        finished: true,
        answered: 1,
        unambiguous_def: true,
        ..Brief::default()
    };
    brief.defined = vec![SymbolHit {
        start_line: 2,
        end_line: 8,
        ..symbol("src/f.rs", "f")
    }];
    let state = Mutex::new(brief.clone());
    strong_answer(&fake, &state, &brief, Instant::now() + SECOND);
    let shown = state.into_inner().unwrap();
    assert!(shown.zero());
    let text = fit(&shown).unwrap().0;
    assert!(text.contains("\nanswer src/f.rs:2:\n"), "{text}");
    assert!(text.contains("  2| pub fn f() {"), "{text}");
    assert!(text.ends_with(ZERO_LINE), "{text}");
    assert!(!text.contains("read: "), "{text}");
    // Ambiguous, too long, or unreadable: nothing is shown.
    let mut ambiguous = brief.clone();
    ambiguous.unambiguous_def = false;
    let mut long = brief.clone();
    long.defined[0].end_line = 80;
    for case in [ambiguous, long] {
        let state = Mutex::new(case.clone());
        strong_answer(&fake, &state, &case, Instant::now() + SECOND);
        assert!(!state.into_inner().unwrap().zero());
    }
}

// ---- the region pack -------------------------------------------------

const PACK_PROMPT: &str = "What does handleError do and who uses it?";

fn pack_source() -> Fake {
    let mut fake = Fake::new();
    fake.rows = Some(Ok(rfound(vec![
        rhit(
            "src/ui.ts",
            1,
            "export function handleError(e: Error): string {",
        ),
        rhit("src/page.ts", 3, "  return handleError(e as Error);"),
        rhit("docs/errors.md", 2, "handleError is documented here"),
    ])));
    fake.files = Ok(found(vec![hit("src/ui.ts", 1), hit("src/page.ts", 3)]));
    fake.symbols = Ok(vec![SymbolHit {
        uid: "src/ui.ts#handleError#function".into(),
        name: "handleError".into(),
        kind: "function".into(),
        path: "src/ui.ts".into(),
        start_line: 1,
        end_line: 3,
    }]);
    fake.sources = vec![
        (
            "src/ui.ts".into(),
            vec![
                "export function handleError(e: Error): string {".into(),
                "  return \"boom \" + e.message;".into(),
                "}".into(),
            ],
        ),
        (
            "src/page.ts".into(),
            vec![
                "export function Page() {".into(),
                "  try { return 1; } catch (e) {".into(),
                "  return handleError(e as Error);".into(),
                "  }".into(),
                "}".into(),
            ],
        ),
    ];
    fake
}

fn packed(fake: Fake, parts: usize) -> Finished {
    try_start_with(
        PACK_PROMPT,
        SECOND,
        move |_| Box::new(fake),
        model_high,
        Around {
            pack: region_pack::Limit::of(parts),
            ..Around::default()
        },
    )
    .expect("the prompt starts a brief")
    .finish_with_record()
}

#[test]
fn a_pack_should_replace_the_packet_of_a_prompt_that_names_a_symbol() {
    let finished = packed(pack_source(), 1);
    let text = finished.text.expect("a brief");
    assert!(
        text.starts_with(
            "[PIXEL:BRIEF]\nregion pack — searched handleError in the symbol graph and text index"
        ),
        "{text}"
    );
    assert!(text.contains("### src/ui.ts:1-3 — handleError definition (function)\n1| export function handleError(e: Error): string {"), "{text}");
    assert!(
        text.contains("### src/page.ts:1-5 — handleError usage in Page\n"),
        "{text}"
    );
    assert!(
        text.contains("3|   return handleError(e as Error);"),
        "{text}"
    );
    assert!(text.ends_with(region_pack::DIRECTIVE), "{text}");
    for gone in [
        "\nfiles:",
        "\nread:",
        "\ncoverage:",
        "\nkind:",
        "docs/errors.md",
    ] {
        assert!(!text.contains(gone), "{gone}: {text}");
    }
    let record = finished.record;
    assert_eq!(record.bytes, text.len());
    assert_eq!(
        (
            record.pack_bytes,
            record.pack_parts,
            record.pack_targets,
            record.fallback
        ),
        (text.len(), 1, 1, false)
    );
}

#[test]
fn the_kill_switch_should_leave_the_packet_as_it_was() {
    let finished = packed(pack_source(), 0);
    let text = finished.text.expect("a brief");
    assert!(text.contains("\nfiles: "), "{text}");
    assert!(
        text.contains("\nread: src/ui.ts:1-3 — handleError"),
        "{text}"
    );
    assert!(!text.contains("region pack"), "{text}");
    let record = finished.record;
    assert_eq!(
        (
            record.pack_bytes,
            record.pack_parts,
            record.pack_targets,
            record.fallback
        ),
        (0, 0, 0, false)
    );
}

#[test]
fn a_stale_graph_should_fall_back_to_the_text_search_for_the_definition() {
    let mut fake = pack_source();
    fake.symbols = Err("the graph is stale".into());
    let finished = packed(fake, 1);
    let text = finished.text.expect("a brief");
    assert!(
        text.contains("text index (graph stale: text fallback)"),
        "{text}"
    );
    assert!(
        text.contains("### src/ui.ts:1-3 — handleError definition (function)"),
        "{text}"
    );
    assert!(text.ends_with(region_pack::DIRECTIVE), "{text}");
    assert!(!text.contains("find-symbol"), "{text}");
    assert!(!text.contains("packet partial"), "{text}");
    assert!(finished.record.fallback);
}

#[test]
fn a_pack_with_nothing_to_show_should_leave_the_packet_in_place() {
    let mut fake = pack_source();
    fake.symbols = Ok(Vec::new());
    fake.rows = Some(Ok(rfound(vec![rhit(
        "docs/errors.md",
        2,
        "handleError is documented here",
    )])));
    fake.files = Ok(found(vec![hit("docs/errors.md", 2)]));
    let finished = packed(fake, 1);
    let text = finished.text.expect("a brief");
    assert!(text.contains("\nfiles: docs/errors.md:2"), "{text}");
    assert!(!text.contains("region pack"), "{text}");
    assert_eq!(finished.record.pack_bytes, 0);
}

#[test]
fn a_pack_should_not_spend_the_operation_budget() {
    let with = packed(pack_source(), 1).record;
    let without = packed(pack_source(), 0).record;
    assert_eq!((with.ops, with.answered), (without.ops, without.answered));
}

#[test]
fn the_pack_stage_should_publish_into_the_brief_and_do_nothing_when_off_or_late() {
    let plan = Plan::from_prompt(PACK_PROMPT).unwrap();
    let brief = |parts| Brief {
        signal: Some(Signal::Strong),
        pack_limit: region_pack::Limit::of(parts),
        answered: 1,
        searched: true,
        ..Brief::default()
    };
    let repo = pack_source();
    let on = Mutex::new(brief(1));
    pack_evidence(&plan, &repo, &on, Instant::now() + SECOND);
    let mut shown = on.into_inner().unwrap();
    assert!(shown.pack.is_some());
    let text = fit(&shown).unwrap().0;
    assert!(text.starts_with("[PIXEL:BRIEF]\nregion pack"), "{text}");
    shown.staleness.mark();
    let text = fit(&shown).unwrap().0;
    assert!(
        text.starts_with(&format!("{BRIEF_TAG}\n{STALENESS_LINE}\n")),
        "{text}"
    );
    assert!(text.ends_with(region_pack::LEAD), "{text}");
    shown.pack_limit.chars = 1;
    let fallback = fit(&shown).expect("the brief falls back when its pack does not fit");
    assert!(!fallback.0.contains("region pack"), "{}", fallback.0);
    let off = Mutex::new(brief(0));
    pack_evidence(&plan, &repo, &off, Instant::now() + SECOND);
    assert!(off.into_inner().unwrap().pack.is_none());
    let late = Mutex::new(brief(1));
    let past = Instant::now().checked_sub(SECOND).unwrap();
    pack_evidence(&plan, &repo, &late, past);
    assert!(late.into_inner().unwrap().pack.is_none());
}

// ---- the kinds that name no symbol ------------------------------------------

const TOUR: &str =
    "I'm new to this repo. give me a quick tour of how it's organised and what each crate is for";
const REVIEW: &str = "review the latest commit and tell me if anything looks risky";
const WALK: &str = "walk me through what happens when someone drops an image onto the canvas";

#[test]
fn the_plan_should_read_a_tour_a_review_and_a_walkthrough_from_the_words() {
    for (prompt, kind) in [
        (TOUR, QuestionKind::Overview),
        (REVIEW, QuestionKind::Review),
        (WALK, QuestionKind::Flow),
    ] {
        assert_eq!(Plan::from_typed(prompt).kind, kind, "{prompt}");
    }
}

/// A fake repository that has a map, a README and nothing else.
struct Mapped(Fake);

impl Evidence for Mapped {
    fn files_with(&self, anchor: &str, deadline: Instant) -> Result<Found, String> {
        self.0.files_with(anchor, deadline)
    }
    fn concept(&self, phrase: &str, deadline: Instant) -> Result<Found, String> {
        self.0.concept(phrase, deadline)
    }
    fn symbols(&self, name: &str, deadline: Instant) -> Result<Vec<SymbolHit>, String> {
        self.0.symbols(name, deadline)
    }
    fn callers(&self, target: &str, deadline: Instant) -> Result<Vec<CallerHit>, String> {
        self.0.callers(target, deadline)
    }
    fn relevance(&self, typed: &str, deadline: Instant) -> Result<RelevanceAnswer, String> {
        self.0.relevance(typed, deadline)
    }
    fn meaning(&self, q: &str, limit: usize, deadline: Instant) -> Result<Vec<MeaningHit>, String> {
        self.0.meaning(q, limit, deadline)
    }
    fn repo_map(&self, _: Instant) -> Result<RepoMap, String> {
        Ok(RepoMap {
            files: vec![kinds_file("src/lib.rs")],
            symbols: 1,
            tops: Vec::new(),
            mains: Vec::new(),
        })
    }
    fn lines_at(
        &self,
        path: &str,
        start: u64,
        end: u64,
        deadline: Instant,
    ) -> Result<Vec<String>, String> {
        self.0.lines_at(path, start, end, deadline)
    }
}

fn kinds_file(path: &str) -> kinds::MapFile {
    kinds::MapFile {
        path: path.into(),
        lang: "rust".into(),
        symbols: 1,
    }
}

fn around_pack() -> Around {
    Around {
        pack: region_pack::Limit::of(3),
        ..Around::default()
    }
}

#[test]
fn a_tour_should_brief_through_a_gate_that_reads_it_as_off_topic() {
    let mut fake = Fake::new();
    fake.relevance = Ok(off_topic_answer());
    fake.sources = vec![(
        "README.md".to_string(),
        vec!["# PIXEL".into(), "A tool.".into()],
    )];
    let finished = try_start_with(
        TOUR,
        SECOND,
        move |_| Box::new(Mapped(fake)),
        model_off,
        around_pack(),
    )
    .unwrap()
    .finish_with_record();
    let text = finished.text.expect("the tour gets a brief");
    assert!(
        text.contains("### repo map\n1 indexed files, 1 symbols (rust 1)"),
        "{text}"
    );
    assert!(
        text.contains("### README.md:1-2 — readme\n1| # PIXEL"),
        "{text}"
    );
    assert!(text.ends_with(region_pack::DIRECTIVE), "{text}");
    let record = finished.record;
    assert_eq!(
        (
            record.gate,
            record.reason.as_deref(),
            record.tier,
            record.kind
        ),
        ("open", None, None, Some("overview"))
    );
    assert!(record.pack_located >= 2, "{}", record.pack_located);
    // Relevance tier no longer controls admission for other kinds either.
    let mut fake = Fake::new();
    fake.relevance = Ok(off_topic_answer());
    let closed = try_start_with(
        PROSE,
        SECOND,
        move |_| Box::new(Mapped(fake)),
        model_off,
        around_pack(),
    )
    .unwrap()
    .finish_with_record();
    assert!(closed.text.is_some());
    assert_eq!(closed.record.gate, "open");
}

#[test]
fn a_tour_without_a_pack_or_without_anything_to_read_should_render_nothing() {
    let fake = || {
        let mut fake = Fake::new();
        fake.relevance = Ok(off_topic_answer());
        fake
    };
    // The pack switched off: the packet has nothing of a tour.
    let off = try_start_with(
        TOUR,
        SECOND,
        move |_| Box::new(Mapped(fake())),
        model_off,
        Around::default(),
    )
    .unwrap()
    .finish_with_record();
    assert!(off.text.is_some());
    // Nothing to read: the fake has no map, no document, no manifest.
    let nothing = try_start_with(
        TOUR,
        SECOND,
        move |_| Box::new(fake()),
        model_off,
        around_pack(),
    )
    .unwrap()
    .finish_with_record();
    assert!(nothing.text.is_some());
    assert_eq!(nothing.record.gate, "open");
}

#[test]
fn the_brief_command_never_runs_past_pis_cutoff_whatever_the_hook_setting() {
    let ms = |window: Duration| u64::try_from(window.as_millis()).unwrap();
    assert_eq!(ms(window_for(Some(3_000), false)), 3_000);
    assert_eq!(ms(window_for(Some(3_000), true)), WINDOW_COMMAND_MAX_MS);
    assert_eq!(ms(window_for(Some(400), true)), 400);
}

#[test]
fn a_window_defaults_per_caller_and_stays_inside_the_bounds() {
    let ms = |window: Duration| u64::try_from(window.as_millis()).unwrap();
    assert_eq!(ms(window_for(None, false)), WINDOW_HOOK_DEFAULT_MS);
    assert_eq!(ms(window_for(None, true)), WINDOW_COMMAND_DEFAULT_MS);
    assert_eq!(ms(window_for(Some(1), false)), 1);
    assert_eq!(ms(window_for(Some(60_000), false)), WINDOW_MAX_MS);
    assert_eq!(ms(window_for(Some(1), true)), 1);
}

/// The session's files, as a hook with a transcript would collect them.
fn around_session(recent: &[&str], dirty: &[&str]) -> Around {
    let paths = SessionPaths {
        recent: recent.iter().map(ToString::to_string).collect(),
        dirty: dirty.iter().map(ToString::to_string).collect(),
    };
    Around {
        context: Some(Box::new(move |_| paths)),
        ..Around::default()
    }
}

#[test]
fn the_session_files_should_boost_an_on_topic_prompt_without_changing_its_gate() {
    let mut fake = Fake::new();
    fake.relevance = Ok(on_topic_answer());
    fake.meaning = Ok(vec![
        lead(DAEMON, 280, "fn watch_ready() {"),
        lead("crates/pixel-daemon/src/api.rs", 40, "fn op_status"),
        lead("crates/pixel-daemon/src/lib.rs", 3, "mod api;"),
    ]);
    let finished = try_start_with(
        PROSE,
        SECOND,
        move |_| Box::new(fake),
        model_high,
        around_session(&["crates/pixel-daemon/src/lib.rs"], &[]),
    )
    .unwrap()
    .finish_with_record();
    assert_eq!(finished.record.gate, "open");
    let text = finished.text.expect("an on-topic prompt is briefed");
    let files = text
        .lines()
        .find(|line| line.starts_with("files:"))
        .unwrap();
    let lib = files.find("crates/pixel-daemon/src/lib.rs").unwrap();
    let api = files.find("crates/pixel-daemon/src/api.rs").unwrap();
    assert!(
        lib < api,
        "the file the session just read ranks above a lead it did not: {files}"
    );
}

/// A pack for `PACK_PROMPT` in session `s` of `root`, with what the
/// session was sent already.
fn packed_in(root: &Path, seen: Seen) -> Finished {
    try_start_with(
        PACK_PROMPT,
        SECOND,
        move |_| Box::new(pack_source()),
        model_high,
        Around {
            pack: region_pack::Limit::of(1),
            seen,
            sent: Some((root.to_path_buf(), "s".to_string())),
            ..Around::default()
        },
    )
    .expect("the prompt starts a brief")
    .finish_with_record()
}

#[test]
fn a_pack_should_be_remembered_for_the_session_and_not_sent_to_it_again() {
    let root = scratch_dir("sent-once");
    std::fs::create_dir_all(root.join(".pixel")).unwrap();
    let now = || now_ms();
    let first = packed_in(&root, Seen::default());
    assert!(first.record.pack_bytes > 0, "{:?}", first.text);
    let seen = sent::load(&root, "s", now());
    assert!(seen.overlaps("src/ui.ts", 1, 3), "the definition was sent");
    assert!(seen.overlaps("src/page.ts", 3, 3), "the usage was sent");
    // The next prompt of the session finds the same code and sends none of it.
    let second = packed_in(&root, seen);
    assert_eq!(second.record.pack_bytes, 0, "{:?}", second.text);
    assert!(
        !second.text.unwrap_or_default().contains("\n### "),
        "no block of source is repeated"
    );
    // A session the host started over is sent the pack again.
    sent::reset(&root, "s");
    let third = packed_in(&root, sent::load(&root, "s", now()));
    assert_eq!(third.record.pack_bytes, first.record.pack_bytes);
}

#[test]
fn a_brief_with_no_pack_should_remember_nothing() {
    let root = scratch_dir("sent-none");
    std::fs::create_dir_all(root.join(".pixel")).unwrap();
    let finished = try_start_with(
        PACK_PROMPT,
        SECOND,
        move |_| Box::new(pack_source()),
        model_high,
        Around {
            pack: region_pack::Limit::of(0),
            sent: Some((root.clone(), "s".to_string())),
            ..Around::default()
        },
    )
    .unwrap()
    .finish_with_record();
    assert!(finished.text.is_some());
    assert!(sent::load(&root, "s", now_ms()).is_empty());
}

fn probe_host(
    root: &Path,
    name: &str,
    extra: &str,
    delay: &str,
    output: &str,
) -> pixel_plugin::Host {
    use std::os::unix::fs::PermissionsExt;

    let plugin = root.join(".pixel/plugins").join(name);
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        plugin.join("pixel-plugin.toml"),
        format!("name = \"{name}\"\nversion = \"0.1.0\"\napi = 1\ndescription = \"evidence\"\nrun = \"probe\"\ncapabilities = [\"brief-probe\"]\n[brief-probe]\nowner = \"test\"\nbudget_ms = 1000\n{extra}"),
    )
    .unwrap();
    let program = plugin.join("probe");
    std::fs::write(
        &program,
        format!("#!/bin/sh\ncat >/dev/null\nsleep {delay}\nprintf '{output}'\n"),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    pixel_plugin::Host {
        home: root.to_path_buf(),
        repo_root: None,
        path: std::ffi::OsString::new(),
        github_base: None,
    }
}

fn probed(
    brief: Brief,
    root: &Path,
    host: pixel_plugin::Host,
    window: Duration,
    setup_delay: Duration,
) -> Finished {
    let (send, done) = mpsc::channel();
    send.send(()).unwrap();
    let started = Instant::now();
    let pending = Pending {
        state: Arc::new(Mutex::new(brief)),
        done,
        deadline: started + window,
        started,
        task: "what does this crate expose".into(),
        signal: Signal::Prose,
        intent: None,
        sent: None,
        probes: Some(ProbeSetup {
            host,
            root: root.to_path_buf(),
            cap: 1_000,
            max_bytes: None,
        }),
    };
    std::thread::sleep(setup_delay);
    pending.finish_with_record()
}

#[test]
fn finishing_should_run_configured_evidence_probes_on_an_open_brief() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let output = "### source.rs:1-1 — evidence\n1| pub struct Library;\n";
    let host = probe_host(&root, "recovery", "when_prompt = \"crate\"\n", "0", output);
    let finished = probed(Brief::default(), &root, host, SECOND, Duration::ZERO);
    assert_eq!(
        finished.text.as_deref(),
        Some("[PIXEL:BRIEF]\n### source.rs:1-1 — evidence\n1| pub struct Library;\n")
    );
    assert_eq!(finished.record.probes.len(), 1);
    assert_eq!(finished.record.probes[0].outcome, "ok");
    assert!(finished.record.stages.probe_ms.is_some());
}

#[test]
fn a_slow_probe_cannot_extend_the_shared_deadline_and_keeps_the_rendered_brief() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let host = probe_host(
        &root,
        "slow",
        "when_prompt = \"crate\"\n",
        "1",
        "### source.rs:1-1 — evidence\n1| pub struct Library;\n",
    );
    let window = Duration::from_millis(500);
    let started = Instant::now();
    let finished = probed(
        full_brief(),
        &root,
        host,
        window,
        Duration::from_millis(430),
    );
    assert!(
        started.elapsed() < window + Duration::from_millis(250),
        "probe exceeded shared expiry by too much: {:?}",
        started.elapsed()
    );
    let text = finished
        .text
        .expect("base brief remains available after probe timeout");
    assert!(text.contains("src/handleError.ts"), "{text}");
    assert_eq!(finished.record.probes[0].outcome, "timeout");
    assert_eq!(finished.record.deadline_ms, Some(500));
}

#[test]
fn spelling_terms_should_drop_the_words_that_only_say_what_is_asked() {
    let terms: Vec<String> = ["cover", "Covers", "validation", "purchase", "behaviour"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(spelling_terms(&terms), ["validation", "purchase"]);
    assert!(spelling_terms(&[]).is_empty());
}
