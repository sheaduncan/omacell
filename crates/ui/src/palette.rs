//! Command palette: fuzzy ranking, recents, `?` AI prefix.

use omacell_bus::CommandJson;

/// Natural-language plan hook (implemented in WP-23).
pub trait AiPlanProvider {
    /// Return a human hint or plan summary. `None` means the feature is absent.
    fn plan(&self, prompt: &str) -> Option<String>;
}

/// One ranked palette row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteHit {
    /// Command id.
    pub id: String,
    /// Documentation.
    pub doc: String,
    /// Current chord, if any.
    pub keys: Vec<String>,
    /// Rank (lower is better).
    pub rank: i32,
}

/// Palette model.
#[derive(Clone, Debug, Default)]
pub struct Palette {
    /// Open?
    pub open: bool,
    /// Query text.
    pub query: String,
    /// Recent command ids, newest first.
    pub recents: Vec<String>,
    /// Ranked hits for the current query.
    pub hits: Vec<PaletteHit>,
    /// Inline argument prompt (schema-driven).
    pub prompt: Option<String>,
    /// Multi-line AI plan preview.
    pub preview: Option<String>,
}

impl Palette {
    /// Open the palette.
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.prompt = None;
        self.preview = None;
    }

    /// Close.
    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.hits.clear();
        self.prompt = None;
        self.preview = None;
    }

    /// Record a successful command for recents.
    pub fn remember(&mut self, id: &str) {
        self.recents.retain(|x| x != id);
        self.recents.insert(0, id.to_string());
        self.recents.truncate(32);
    }

    /// Fuzzy-rank `commands` for `query`. Recents sort first on an empty query.
    pub fn rank(&mut self, commands: &[CommandJson], query: &str) {
        self.rank_with_ai(commands, query, None);
    }

    /// Rank with an optional natural-language planning provider.
    pub fn rank_with_ai(
        &mut self,
        commands: &[CommandJson],
        query: &str,
        ai: Option<&dyn AiPlanProvider>,
    ) {
        self.query = query.to_string();
        if let Some(rest) = query.strip_prefix('?') {
            self.hits.clear();
            self.preview = None;
            let request = rest.trim();
            self.prompt = Some(if request.is_empty() {
                "Type a sentence after ? for an AI plan".into()
            } else {
                ai.and_then(|provider| provider.plan(request))
                    .unwrap_or_else(|| "Press Enter to propose an AI plan".into())
            });
            return;
        }
        self.prompt = None;
        self.preview = None;
        let q = query.to_ascii_lowercase();
        let mut hits: Vec<PaletteHit> = commands
            .iter()
            .filter_map(|c| {
                let rank = fuzzy_rank(&c.id, &c.doc, &q)?;
                Some(PaletteHit {
                    id: c.id.clone(),
                    doc: c.doc.clone(),
                    keys: c.default_keys.clone(),
                    rank,
                })
            })
            .collect();
        if q.is_empty() {
            hits.sort_by(|a, b| {
                let ai = self
                    .recents
                    .iter()
                    .position(|id| id == &a.id)
                    .unwrap_or(usize::MAX);
                let bi = self
                    .recents
                    .iter()
                    .position(|id| id == &b.id)
                    .unwrap_or(usize::MAX);
                ai.cmp(&bi).then_with(|| a.id.cmp(&b.id))
            });
        } else {
            hits.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.id.cmp(&b.id)));
        }
        self.hits = hits;
    }

    /// Build an inline prompt from a command's JSON Schema required fields.
    pub fn prompt_for(&mut self, command: &CommandJson) {
        self.preview = None;
        let required = command
            .arg_schema
            .get("required")
            .and_then(serde_json::Value::as_array);
        let properties = command
            .arg_schema
            .get("properties")
            .and_then(serde_json::Value::as_object);
        let mut fields = required
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(|name| {
                let kind = properties
                    .and_then(|props| props.get(name))
                    .and_then(|schema| schema.get("type"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("value");
                format!("{name}: {kind}")
            })
            .collect::<Vec<_>>();
        fields.sort();
        self.prompt = (!fields.is_empty()).then(|| fields.join(", "));
    }
}

fn fuzzy_rank(id: &str, doc: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(1000);
    }
    let id_l = id.to_ascii_lowercase();
    let doc_l = doc.to_ascii_lowercase();
    if let Some(pos) = id_l.find(query) {
        return Some(pos as i32);
    }
    if subsequence(&id_l, query) {
        return Some(100 + id.len() as i32);
    }
    if doc_l.contains(query) {
        return Some(300);
    }
    None
}

fn subsequence(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars();
    for c in needle.chars() {
        loop {
            match it.next() {
                Some(h) if h == c => break,
                Some(_) => {}
                None => return false,
            }
        }
    }
    true
}
