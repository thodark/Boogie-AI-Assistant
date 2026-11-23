use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::config::{ApiConfig, active_config_dir};
use crate::util::{http_client, truncate_chars};

pub const DEFAULT_MEMORY_MODEL: &str = "gemini-2.5-flash-lite";
const MAX_STORED_SESSIONS: usize = 25;
const MAX_SESSION_SUMMARY_CHARS: usize = 1600;
const PROMPT_MEMORY_MAX_ITEMS: usize = 30;
const PROMPT_MEMORY_MAX_CHARS: usize = 2400;
const PROMPT_ALSO_REMEMBERED_MAX_CHARS: usize = 700;
const MIN_TURNS_FOR_AI_SUMMARY: usize = 4;
const MIN_USER_CHARS_FOR_AI_SUMMARY: usize = 80;
const AI_CONSOLIDATION_COOLDOWN_SECS: u64 = 600; // 10 min cooldown protects free-tier quota
const AI_CONSOLIDATION_EXIT_COOLDOWN_SECS: u64 = 45;

static MEMORY_FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static LAST_AI_CONSOLIDATION: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

fn file_lock() -> &'static Mutex<()> {
    MEMORY_FILE_LOCK.get_or_init(|| Mutex::new(()))
}

fn default_importance() -> u8 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub value: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default = "default_importance")]
    pub importance: u8,
    #[serde(default)]
    pub access_count: u32,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Default for MemoryEntry {
    fn default() -> Self {
        Self {
            value: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            importance: default_importance(),
            access_count: 0,
            tags: Vec::new(),
        }
    }
}

pub type MemoryCategory = HashMap<String, MemoryEntry>;
pub type Memory = HashMap<String, MemoryCategory>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub date: String,
    pub summary: String,
    pub language: String,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub ai_summarized: bool,
}

#[derive(Debug, Clone)]
pub struct MemoryRow {
    pub category: String,
    pub key: String,
    pub entry: MemoryEntry,
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

fn memory_path() -> PathBuf {
    active_config_dir().join("memory.json")
}

fn long_term_path() -> PathBuf {
    active_config_dir().join("long_term.json")
}

/// Folds Vietnamese diacritics & common Latin accents into ASCII lowercase for tolerant search.
pub fn fold_vietnamese(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let mapped = match ch {
            'à' | 'á' | 'ả' | 'ã' | 'ạ' | 'ă' | 'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ầ' | 'ấ'
            | 'ẩ' | 'ẫ' | 'ậ' | 'À' | 'Á' | 'Ả' | 'Ã' | 'Ạ' | 'Ă' | 'Ằ' | 'Ắ' | 'Ẳ' | 'Ẵ' | 'Ặ'
            | 'Â' | 'Ầ' | 'Ấ' | 'Ẩ' | 'Ẫ' | 'Ậ' => 'a',
            'đ' | 'Đ' => 'd',
            'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' | 'ê' | 'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' | 'È' | 'É' | 'Ẻ'
            | 'Ẽ' | 'Ẹ' | 'Ê' | 'Ề' | 'Ế' | 'Ể' | 'Ễ' | 'Ệ' => 'e',
            'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' | 'Ì' | 'Í' | 'Ỉ' | 'Ĩ' | 'Ị' => 'i',
            'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' | 'ô' | 'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' | 'ơ' | 'ờ' | 'ớ'
            | 'ở' | 'ỡ' | 'ợ' | 'Ò' | 'Ó' | 'Ỏ' | 'Õ' | 'Ọ' | 'Ô' | 'Ồ' | 'Ố' | 'Ổ' | 'Ỗ' | 'Ộ'
            | 'Ơ' | 'Ờ' | 'Ớ' | 'Ở' | 'Ỡ' | 'Ợ' => 'o',
            'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' | 'ư' | 'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' | 'Ù' | 'Ú' | 'Ủ'
            | 'Ũ' | 'Ụ' | 'Ư' | 'Ừ' | 'Ứ' | 'Ử' | 'Ữ' | 'Ự' => 'u',
            'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' | 'Ỳ' | 'Ý' | 'Ỷ' | 'Ỹ' | 'Ỵ' => 'y',
            other => {
                for lc in other.to_lowercase() {
                    out.push(lc);
                }
                continue;
            }
        };
        out.push(mapped);
    }
    out
}

/// Normalizes category/key names into consistent `snake_case` ASCII slugs to avoid duplicate keys.
pub fn normalize_slug(raw: &str) -> String {
    let folded = fold_vietnamese(raw.trim());
    let mut slug = String::with_capacity(folded.len());
    let mut prev_sep = false;
    for ch in folded.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            prev_sep = false;
        } else if !prev_sep && !slug.is_empty() {
            slug.push('_');
            prev_sep = true;
        }
    }
    slug.trim_matches('_').to_string()
}

fn clean_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tag in tags {
        let t = tag.trim();
        if t.is_empty() {
            continue;
        }
        let key = fold_vietnamese(t);
        if seen.insert(key) {
            out.push(truncate_chars(t, 32).to_string());
            if out.len() >= 8 {
                break;
            }
        }
    }
    out
}

fn tokenize(text: &str) -> Vec<String> {
    let folded = fold_vietnamese(text);
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|tok| tok.chars().count() >= 2)
        .map(ToString::to_string)
        .collect()
}

/// Atomically writes `content` to `path` via a temp file + OS replace so crashes never corrupt JSON.
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    let tmp_name = format!(
        "{}.tmp.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("memory"),
        std::process::id()
    );
    let tmp_path = path.with_file_name(tmp_name);
    std::fs::write(&tmp_path, content).map_err(|e| format!("write tmp: {e}"))?;

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        use windows::core::PCWSTR;

        let src: Vec<u16> = tmp_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let dst: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let ok = unsafe {
            MoveFileExW(
                PCWSTR(src.as_ptr()),
                PCWSTR(dst.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if let Err(e) = ok {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("atomic replace failed: {e}"));
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        std::fs::rename(&tmp_path, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            format!("atomic rename failed: {e}")
        })
    }
}

pub fn load_memory() -> Memory {
    let _g = file_lock().lock().ok();
    load_memory_unlocked(&memory_path())
}

fn load_memory_unlocked(path: &Path) -> Memory {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Memory::new(),
    }
}

fn save_memory_unlocked(path: &Path, memory: &Memory) -> Result<(), String> {
    let text = serde_json::to_string_pretty(memory).map_err(|e| format!("json: {e}"))?;
    atomic_write(path, &text)
}

pub fn upsert_fact(
    category: &str,
    key: &str,
    value: &str,
    importance: Option<u8>,
    tags: Vec<String>,
) -> Result<(String, String), String> {
    let cat = {
        let s = normalize_slug(category);
        if s.is_empty() {
            "general".to_string()
        } else {
            s
        }
    };
    let k = normalize_slug(key);
    let val = value.trim();
    if k.is_empty() || val.is_empty() {
        return Err("key and value are required".to_string());
    }

    let _g = file_lock().lock().ok();
    let path = memory_path();
    let mut mem = load_memory_unlocked(&path);
    upsert_into_map(&mut mem, &cat, &k, val, importance, tags);
    save_memory_unlocked(&path, &mem)?;
    Ok((cat, k))
}

fn upsert_into_map(
    mem: &mut Memory,
    cat: &str,
    key: &str,
    value: &str,
    importance: Option<u8>,
    tags: Vec<String>,
) {
    let now = now_stamp();
    let cleaned_tags = clean_tags(tags);
    let category_map = mem.entry(cat.to_string()).or_default();

    // Check if an older unnormalized key matches the same normalized slug to avoid duplicates
    let existing_raw_key = category_map
        .keys()
        .find(|existing| normalize_slug(existing) == key && existing.as_str() != key)
        .cloned();
    let migrated = existing_raw_key.and_then(|old_k| category_map.remove(&old_k));

    let entry = category_map
        .entry(key.to_string())
        .or_insert_with(|| migrated.unwrap_or_default());

    entry.value = truncate_chars(value.trim(), 500).to_string();
    if entry.created_at.is_empty() {
        entry.created_at = now.clone();
    }
    entry.updated_at = now;
    if let Some(imp) = importance {
        entry.importance = imp.clamp(1, 5);
    } else if entry.importance == 0 {
        entry.importance = default_importance();
    }
    if !cleaned_tags.is_empty() {
        let mut merged = entry.tags.clone();
        merged.extend(cleaned_tags);
        entry.tags = clean_tags(merged);
    }
}

/// Removes a memory entry by exact `category`/`key`, or best match via `query`.
pub fn forget_memory(
    category: Option<&str>,
    key: Option<&str>,
    query: Option<&str>,
) -> Result<String, String> {
    let _g = file_lock().lock().ok();
    let path = memory_path();
    let mut mem = load_memory_unlocked(&path);
    let result = forget_from_map(&mut mem, category, key, query)?;
    save_memory_unlocked(&path, &mem)?;
    Ok(result)
}

fn forget_from_map(
    mem: &mut Memory,
    category: Option<&str>,
    key: Option<&str>,
    query: Option<&str>,
) -> Result<String, String> {
    if let (Some(c), Some(k)) = (
        category.filter(|s| !s.trim().is_empty()),
        key.filter(|s| !s.trim().is_empty()),
    ) {
        let norm_c = normalize_slug(c);
        let norm_k = normalize_slug(k);
        let matched_cat = mem
            .keys()
            .find(|existing| existing.as_str() == c || normalize_slug(existing) == norm_c)
            .cloned();
        if let Some(cat_name) = matched_cat
            && let Some(cat_map) = mem.get_mut(&cat_name)
        {
            let matched_key = cat_map
                .keys()
                .find(|existing| existing.as_str() == k || normalize_slug(existing) == norm_k)
                .cloned();
            if let Some(key_name) = matched_key {
                let removed = cat_map.remove(&key_name);
                if cat_map.is_empty() {
                    mem.remove(&cat_name);
                }
                let val = removed.map(|e| e.value).unwrap_or_default();
                return Ok(format!("Forgot {cat_name}/{key_name} ({val})"));
            }
        }
    }

    // Try key match across any category
    if let Some(k) = key.filter(|s| !s.trim().is_empty()) {
        let norm_k = normalize_slug(k);
        let mut found: Option<(String, String)> = None;
        for (cat_name, cat_map) in mem.iter() {
            if let Some(key_name) = cat_map
                .keys()
                .find(|existing| existing.as_str() == k || normalize_slug(existing) == norm_k)
            {
                found = Some((cat_name.clone(), key_name.clone()));
                break;
            }
        }
        if let Some((cat_name, key_name)) = found
            && let Some(cat_map) = mem.get_mut(&cat_name)
        {
            let removed = cat_map.remove(&key_name);
            if cat_map.is_empty() {
                mem.remove(&cat_name);
            }
            let val = removed.map(|e| e.value).unwrap_or_default();
            return Ok(format!("Forgot {cat_name}/{key_name} ({val})"));
        }
    }

    // Fallback: best semantic/token match using query
    let q = query
        .or(key)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Provide category+key or a query to forget.".to_string())?;

    let ranked = rank_facts(mem, q);
    if let Some((score, cat_name, key_name, _)) = ranked.into_iter().next()
        && score >= 2.0
        && let Some(cat_map) = mem.get_mut(&cat_name)
    {
        let removed = cat_map.remove(&key_name);
        if cat_map.is_empty() {
            mem.remove(&cat_name);
        }
        let val = removed.map(|e| e.value).unwrap_or_default();
        return Ok(format!("Forgot {cat_name}/{key_name} ({val})"));
    }

    Err(format!("No matching memory found to forget for '{q}'."))
}

pub fn clear_all_memory() -> Result<(), String> {
    let _g = file_lock().lock().ok();
    save_memory_unlocked(&memory_path(), &Memory::new())?;
    let empty_sessions: Vec<SessionSummary> = Vec::new();
    let text = serde_json::to_string_pretty(&empty_sessions).map_err(|e| format!("json: {e}"))?;
    atomic_write(&long_term_path(), &text)
}

pub fn list_memory_rows() -> Vec<MemoryRow> {
    let mem = load_memory();
    sorted_rows(&mem)
}

fn priority_key(
    cat: &str,
    key: &str,
    entry: &MemoryEntry,
) -> (u8, u8, String, u32, String, String) {
    let is_identity = u8::from(matches!(cat, "identity" | "user"));
    let imp = if entry.importance == 0 {
        default_importance()
    } else {
        entry.importance
    };
    (
        is_identity,
        imp,
        entry.updated_at.clone(),
        entry.access_count,
        cat.to_string(),
        key.to_string(),
    )
}

fn sorted_rows(memory: &Memory) -> Vec<MemoryRow> {
    let mut rows = Vec::new();
    for (cat, entries) in memory {
        for (key, entry) in entries {
            rows.push(MemoryRow {
                category: cat.clone(),
                key: key.clone(),
                entry: entry.clone(),
            });
        }
    }
    rows.sort_by(|a, b| {
        let ka = priority_key(&a.category, &a.key, &a.entry);
        let kb = priority_key(&b.category, &b.key, &b.entry);
        kb.0.cmp(&ka.0)
            .then_with(|| kb.1.cmp(&ka.1))
            .then_with(|| kb.2.cmp(&ka.2))
            .then_with(|| kb.3.cmp(&ka.3))
            .then_with(|| ka.4.cmp(&kb.4))
            .then_with(|| ka.5.cmp(&kb.5))
    });
    rows
}

/// Deterministic, budgeted prompt memory block with real `[ALSO REMEMBERED]` index.
pub fn format_memory_for_prompt(memory: &Memory) -> String {
    if memory.is_empty() {
        return String::new();
    }
    let rows = sorted_rows(memory);
    let mut shown = String::from("[MEMORY]\n");
    let mut overflow: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut count = 0usize;

    for row in rows {
        let line = if row.entry.updated_at.is_empty() {
            format!("- {}/{}: {}\n", row.category, row.key, row.entry.value)
        } else {
            let date_only = row
                .entry
                .updated_at
                .split_whitespace()
                .next()
                .unwrap_or(&row.entry.updated_at);
            format!(
                "- {}/{}: {} ({date_only})\n",
                row.category, row.key, row.entry.value
            )
        };

        if count < PROMPT_MEMORY_MAX_ITEMS && shown.len() + line.len() <= PROMPT_MEMORY_MAX_CHARS {
            shown.push_str(&line);
            count += 1;
        } else {
            overflow.entry(row.category).or_default().push(row.key);
        }
    }

    if !overflow.is_empty() {
        shown.push_str("\n[ALSO REMEMBERED]\n");
        let mut used = 0usize;
        for (cat, mut keys) in overflow {
            keys.sort();
            let line = format!("- {cat}: {}\n", keys.join(", "));
            if used + line.len() > PROMPT_ALSO_REMEMBERED_MAX_CHARS {
                shown.push_str("- (more stored — call recall_memory to search)\n");
                break;
            }
            shown.push_str(&line);
            used += line.len();
        }
    }

    shown
}

fn score_document(query_folded: &str, query_tokens: &[String], doc_text: &str) -> f32 {
    if query_folded.is_empty() {
        return 1.0;
    }
    let doc_folded = fold_vietnamese(doc_text);
    let doc_tokens = tokenize(doc_text);
    let doc_set: HashSet<&str> = doc_tokens.iter().map(String::as_str).collect();

    let mut score = 0.0_f32;
    if doc_folded.contains(query_folded) {
        score += 6.0;
    }

    let mut matched_tokens = 0usize;
    for qt in query_tokens {
        if doc_set.contains(qt.as_str()) {
            score += 3.0;
            matched_tokens += 1;
        } else if doc_tokens
            .iter()
            .any(|dt| dt.starts_with(qt.as_str()) || (qt.len() >= 4 && dt.contains(qt.as_str())))
        {
            score += 1.6;
            matched_tokens += 1;
        }
    }

    if !query_tokens.is_empty() && matched_tokens == query_tokens.len() {
        score += 2.5; // All query words present
    }
    score
}

fn rank_facts(memory: &Memory, query: &str) -> Vec<(f32, String, String, MemoryEntry)> {
    let q_folded = fold_vietnamese(query.trim());
    let q_tokens = tokenize(query);
    let mut scored = Vec::new();

    for (cat, entries) in memory {
        for (key, entry) in entries {
            let doc = format!(
                "{cat} {} {key} {} {} {}",
                cat.replace('_', " "),
                key.replace('_', " "),
                entry.value,
                entry.tags.join(" ")
            );
            let mut score = score_document(&q_folded, &q_tokens, &doc);
            if score > 0.0 {
                let imp = entry.importance.clamp(1, 5) as f32;
                score += imp * 0.25 + (entry.access_count.min(20) as f32) * 0.05;
                scored.push((score, cat.clone(), key.clone(), entry.clone()));
            }
        }
    }

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.3.updated_at.cmp(&a.3.updated_at))
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    scored
}

/// Smart search across BOTH `memory.json` (facts) and `long_term.json` (past sessions),
/// with Vietnamese diacritic-folding, multi-word token matching, and access tracking.
pub fn search_memory(query: &str, limit: usize) -> String {
    let limit = limit.clamp(1, 50);
    let q = query.trim();

    let (mut mem, sessions) = {
        let _g = file_lock().lock().ok();
        (
            load_memory_unlocked(&memory_path()),
            load_all_sessions_unlocked(&long_term_path()),
        )
    };

    if mem.is_empty() && sessions.is_empty() {
        return "No memories stored yet.".to_string();
    }

    if q.is_empty() {
        let mut lines = Vec::new();
        for row in sorted_rows(&mem).into_iter().take(limit) {
            lines.push(format!(
                "- {}/{}: {}",
                row.category, row.key, row.entry.value
            ));
        }
        if lines.len() < limit && !sessions.is_empty() {
            let rem = (limit - lines.len()).min(3);
            lines.push("\n[RECENT SESSIONS]".to_string());
            for s in sessions.iter().rev().take(rem) {
                lines.push(format!(
                    "- ({}): {}",
                    s.date,
                    truncate_chars(&s.summary, 220)
                ));
            }
        }
        return lines.join("\n");
    }

    let ranked_facts = rank_facts(&mem, q);
    let q_folded = fold_vietnamese(q);
    let q_tokens = tokenize(q);

    let mut ranked_sessions: Vec<(f32, &SessionSummary)> = sessions
        .iter()
        .filter_map(|s| {
            let doc = format!("{} {} {}", s.date, s.topics.join(" "), s.summary);
            let sc = score_document(&q_folded, &q_tokens, &doc);
            (sc > 0.0).then_some((sc, s))
        })
        .collect();
    ranked_sessions.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.date.cmp(&a.1.date))
    });

    if ranked_facts.is_empty() && ranked_sessions.is_empty() {
        return format!("No memories matching '{q}'.");
    }

    let mut out = Vec::new();
    let mut touched = Vec::new();
    let fact_budget = if ranked_sessions.is_empty() {
        limit
    } else {
        limit.saturating_sub(ranked_sessions.len().min(3)).max(1)
    };

    for (_, cat, key, entry) in ranked_facts.into_iter().take(fact_budget) {
        let meta = if entry.updated_at.is_empty() {
            String::new()
        } else {
            format!(" [{}]", entry.updated_at)
        };
        out.push(format!("- {cat}/{key}: {}{meta}", entry.value));
        touched.push((cat, key));
    }

    if !ranked_sessions.is_empty() {
        out.push("\n[PAST SESSIONS]".to_string());
        for (_, s) in ranked_sessions.into_iter().take(3) {
            let topic_str = if s.topics.is_empty() {
                String::new()
            } else {
                format!(" [{}]", s.topics.join(", "))
            };
            out.push(format!(
                "- ({}){topic_str}: {}",
                s.date,
                truncate_chars(&s.summary, 320)
            ));
        }
    }

    // Increment recall access_count cleanly
    if !touched.is_empty() {
        let _g = file_lock().lock().ok();
        let path = memory_path();
        mem = load_memory_unlocked(&path);
        for (cat, key) in touched {
            if let Some(entry) = mem.get_mut(&cat).and_then(|m| m.get_mut(&key)) {
                entry.access_count = entry.access_count.saturating_add(1);
            }
        }
        let _ = save_memory_unlocked(&path, &mem);
    }

    out.join("\n")
}

// --- Episodic Sessions & Transcript Turn Merging ---

/// Merges streaming transcript chunks from the same speaker into complete readable turns.
pub fn push_transcript_turn(log: &mut Vec<String>, speaker: &str, raw_chunk: &str) {
    let chunk = raw_chunk.trim();
    if chunk.is_empty() {
        return;
    }
    let prefix = format!("{speaker}: ");
    if let Some(last) = log.last_mut()
        && last.starts_with(&prefix)
    {
        let starts_punct = chunk
            .chars()
            .next()
            .is_some_and(|c| matches!(c, '.' | ',' | '!' | '?' | ':' | ';' | '\'' | '’' | ')'));
        if !starts_punct && !last.ends_with(' ') && !last.ends_with('\n') {
            last.push(' ');
        }
        last.push_str(chunk);
        return;
    }
    log.push(format!("{prefix}{chunk}"));
    if log.len() > 200 {
        let drop_n = log.len() - 200;
        log.drain(..drop_n);
    }
}

fn load_all_sessions_unlocked(path: &Path) -> Vec<SessionSummary> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn load_recent_sessions(limit: usize) -> Vec<SessionSummary> {
    let _g = file_lock().lock().ok();
    let entries = load_all_sessions_unlocked(&long_term_path());
    entries
        .into_iter()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn delete_session_by_index(rev_index_from_newest: usize) -> Result<(), String> {
    let _g = file_lock().lock().ok();
    let path = long_term_path();
    let mut entries = load_all_sessions_unlocked(&path);
    if rev_index_from_newest >= entries.len() {
        return Err("Session index out of range".to_string());
    }
    let real_idx = entries.len() - 1 - rev_index_from_newest;
    entries.remove(real_idx);
    let text = serde_json::to_string_pretty(&entries).map_err(|e| format!("json: {e}"))?;
    atomic_write(&path, &text)
}

pub fn save_session_summary_rich(
    summary: &str,
    language: &str,
    topics: Vec<String>,
    ai_summarized: bool,
) {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return;
    }
    let _g = file_lock().lock().ok();
    let path = long_term_path();
    let mut entries = load_all_sessions_unlocked(&path);
    entries.push(SessionSummary {
        date: now_stamp(),
        summary: truncate_chars(trimmed, MAX_SESSION_SUMMARY_CHARS).to_string(),
        language: language.to_string(),
        topics: clean_tags(topics),
        ai_summarized,
    });
    if entries.len() > MAX_STORED_SESSIONS {
        let drop_n = entries.len() - MAX_STORED_SESSIONS;
        entries.drain(..drop_n);
    }
    if let Ok(text) = serde_json::to_string_pretty(&entries)
        && let Err(e) = atomic_write(&path, &text)
    {
        tracing::error!("failed to save long_term.json: {e}");
    }
}

/// Formats a clean local fallback summary from merged turns when AI consolidation is disabled/skipped.
pub fn build_local_fallback_summary(turns: &[String]) -> String {
    if turns.is_empty() {
        return String::new();
    }
    let recent: Vec<String> = turns
        .iter()
        .rev()
        .take(14)
        .rev()
        .map(|line| truncate_chars(line.trim(), 180).to_string())
        .collect();
    recent.join("\n")
}

// --- Level-2 Guarded AI Auto-Consolidation ---

#[derive(Debug, Deserialize)]
struct ConsolidatedFact {
    #[serde(default)]
    category: String,
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: String,
    #[serde(default = "default_importance")]
    importance: u8,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ForgetTarget {
    #[serde(default)]
    category: String,
    #[serde(default)]
    key: String,
}

#[derive(Debug, Deserialize)]
struct ConsolidationPayload {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    facts: Vec<ConsolidatedFact>,
    #[serde(default)]
    forget: Vec<ForgetTarget>,
}

fn can_run_ai_consolidation(turns: &[String], is_exit: bool) -> bool {
    if turns.len() < MIN_TURNS_FOR_AI_SUMMARY {
        return false;
    }
    let user_chars: usize = turns
        .iter()
        .filter_map(|t| t.strip_prefix("user: "))
        .map(|s| s.trim().chars().count())
        .sum();
    if user_chars < MIN_USER_CHARS_FOR_AI_SUMMARY {
        return false;
    }
    let min_gap = if is_exit {
        AI_CONSOLIDATION_EXIT_COOLDOWN_SECS
    } else {
        AI_CONSOLIDATION_COOLDOWN_SECS
    };
    let lock = LAST_AI_CONSOLIDATION.get_or_init(|| Mutex::new(None));
    let Ok(guard) = lock.lock() else {
        return false;
    };
    !matches!(*guard, Some(prev) if prev.elapsed().as_secs() < min_gap)
}

fn mark_ai_consolidation_attempted() {
    let lock = LAST_AI_CONSOLIDATION.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(Instant::now());
    }
}

/// Consolidates unsummarized session turns into `long_term.json` and extracts durable facts
/// into `memory.json` via lightweight Gemini REST (`gemini-2.5-flash-lite` by default),
/// strictly rate-limited with silent local fallback so Live API quota is never harmed.
pub async fn consolidate_session(cfg: &ApiConfig, turns: &[String], language: &str, is_exit: bool) {
    if turns.is_empty() {
        return;
    }

    let try_ai = cfg.auto_memory_consolidation()
        && !cfg.api_key().is_empty()
        && can_run_ai_consolidation(turns, is_exit);

    if try_ai {
        mark_ai_consolidation_attempted();
        let timeout_secs = if is_exit { 4 } else { 10 };
        match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            call_gemini_consolidator(cfg, turns, language),
        )
        .await
        {
            Ok(Ok(payload)) => {
                apply_consolidation_payload(payload, turns, language);
                return;
            }
            Ok(Err(e)) => {
                tracing::warn!("AI memory consolidation skipped (falling back to local): {e}");
            }
            Err(_) => {
                tracing::warn!(
                    "AI memory consolidation timed out after {timeout_secs}s (falling back to local)"
                );
            }
        }
    }

    let fallback = build_local_fallback_summary(turns);
    if !fallback.is_empty() {
        save_session_summary_rich(&fallback, language, Vec::new(), false);
    }
}

fn apply_consolidation_payload(payload: ConsolidationPayload, turns: &[String], language: &str) {
    let summary_text = if payload.summary.trim().is_empty() {
        build_local_fallback_summary(turns)
    } else {
        payload.summary.trim().to_string()
    };
    save_session_summary_rich(&summary_text, language, payload.topics, true);

    if payload.facts.is_empty() && payload.forget.is_empty() {
        return;
    }

    let _g = file_lock().lock().ok();
    let path = memory_path();
    let mut mem = load_memory_unlocked(&path);
    let mut added = 0usize;
    let mut removed = 0usize;

    for f in payload.forget {
        if forget_from_map(
            &mut mem,
            (!f.category.trim().is_empty()).then_some(f.category.as_str()),
            (!f.key.trim().is_empty()).then_some(f.key.as_str()),
            None,
        )
        .is_ok()
        {
            removed += 1;
        }
    }

    for fact in payload.facts.into_iter().take(8) {
        let cat = {
            let s = normalize_slug(&fact.category);
            if s.is_empty() {
                "general".to_string()
            } else {
                s
            }
        };
        let key = normalize_slug(&fact.key);
        if key.is_empty() || fact.value.trim().is_empty() {
            continue;
        }
        upsert_into_map(
            &mut mem,
            &cat,
            &key,
            &fact.value,
            Some(fact.importance.clamp(1, 5)),
            fact.tags,
        );
        added += 1;
    }

    if let Err(e) = save_memory_unlocked(&path, &mem) {
        tracing::error!("failed saving consolidated facts: {e}");
    } else {
        tracing::info!(
            "AI memory consolidation saved summary (+{added} facts, -{removed} outdated)"
        );
    }
}

async fn call_gemini_consolidator(
    cfg: &ApiConfig,
    turns: &[String],
    language: &str,
) -> Result<ConsolidationPayload, String> {
    let model = cfg.memory_model().trim_start_matches("models/");
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={}",
        cfg.api_key()
    );

    let existing = load_memory();
    let existing_keys: Vec<String> = sorted_rows(&existing)
        .into_iter()
        .take(45)
        .map(|r| {
            format!(
                "{}/{}: {}",
                r.category,
                r.key,
                truncate_chars(&r.entry.value, 80)
            )
        })
        .collect();

    let transcript: String = turns
        .iter()
        .rev()
        .take(40)
        .rev()
        .map(|line| truncate_chars(line, 260))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "You are a quiet background memory consolidator for a personal voice assistant.\n\
         Analyze the conversation transcript below and return ONLY valid JSON matching this schema:\n\
         {{\n\
           \"summary\": \"2-4 concise sentences in {language} capturing what was discussed, any personal/emotional context, and open threads. Do NOT invent anything.\",\n\
           \"topics\": [\"2 to 5 short keyword tags\"],\n\
           \"facts\": [\n\
             {{\"category\": \"identity|preferences|projects|people|habits|goals|life\", \"key\": \"snake_case_key\", \"value\": \"short durable fact\", \"importance\": 1-5, \"tags\": [\"optional\"]}}\n\
           ],\n\
           \"forget\": [\n\
             {{\"category\": \"existing_category\", \"key\": \"existing_key\"}}\n\
           ]\n\
         }}\n\
         Rules:\n\
         - Only extract DURABLE, genuinely useful facts about the user (identity, preferences, projects, relationships, routines, goals, recurring context). Skip one-off commands like 'turn volume down' or 'open chrome'.\n\
         - Reuse existing category/key names when updating an existing fact instead of creating duplicates.\n\
         - Put outdated keys in \"forget\" ONLY if the user explicitly corrected or cancelled an existing memory.\n\
         - Return empty arrays for \"facts\" / \"forget\" if nothing durable came up.\n\n\
         Existing stored memories:\n{}\n\n\
         Conversation transcript:\n{}",
        if existing_keys.is_empty() {
            "(none)".to_string()
        } else {
            existing_keys.join("\n")
        },
        transcript
    );

    let body = serde_json::json!({
        "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
        "generationConfig": {
            "temperature": 0.2,
            "maxOutputTokens": 700,
            "responseMimeType": "application/json"
        }
    });

    let resp = http_client()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| format!("json decode: {e}"))?;
    let raw_text = json
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing candidate text".to_string())?;

    let cleaned = raw_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    serde_json::from_str::<ConsolidationPayload>(cleaned)
        .map_err(|e| format!("consolidation json parse: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_vietnamese_diacritics_and_normalizes_slugs() {
        assert_eq!(
            fold_vietnamese("Dự Án Tiếng Việt Đỉnh"),
            "du an tieng viet dinh"
        );
        assert_eq!(normalize_slug("Màu Yêu Thích!"), "mau_yeu_thich");
        assert_eq!(normalize_slug("  Favorite Color "), "favorite_color");
    }

    #[test]
    fn smart_search_matches_accentless_multiword_queries() {
        let mut mem = Memory::new();
        upsert_into_map(
            &mut mem,
            "projects",
            "boogie_ai",
            "Đang phát triển trợ lý giọng nói bằng ngôn ngữ Rust trên Windows",
            Some(5),
            vec!["rust".to_string(), "ai".to_string()],
        );
        upsert_into_map(
            &mut mem,
            "preferences",
            "drink",
            "Thích uống cà phê sữa đá buổi sáng",
            Some(3),
            vec![],
        );

        let hits = rank_facts(&mem, "du an rust");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].1, "projects");
        assert_eq!(hits[0].2, "boogie_ai");

        let drink_hits = rank_facts(&mem, "ca phe sua");
        assert!(!drink_hits.is_empty());
        assert_eq!(drink_hits[0].2, "drink");
    }

    #[test]
    fn budgeted_prompt_populates_also_remembered_for_overflow() {
        let mut mem = Memory::new();
        for i in 0..40 {
            upsert_into_map(
                &mut mem,
                "notes",
                &format!("item_{i:02}"),
                &format!(
                    "This is stored fact number {i} with enough detail to exceed prompt budget cleanly"
                ),
                Some(if i < 5 { 5 } else { 2 }),
                vec![],
            );
        }
        let formatted = format_memory_for_prompt(&mem);
        assert!(formatted.contains("[MEMORY]"));
        assert!(formatted.contains("[ALSO REMEMBERED]"));
        // High-importance items must appear inside the primary [MEMORY] section
        let primary_part = formatted.split("[ALSO REMEMBERED]").next().unwrap_or("");
        assert!(primary_part.contains("notes/item_00"));
        assert!(primary_part.contains("notes/item_04"));
    }

    #[test]
    fn forget_removes_exact_and_fuzzy_memories() {
        let mut mem = Memory::new();
        upsert_into_map(
            &mut mem,
            "preferences",
            "mau_yeu_thich",
            "Xanh dương",
            Some(3),
            vec![],
        );
        upsert_into_map(
            &mut mem,
            "habits",
            "ngu_muon",
            "Thường thức khuya tới 2 giờ sáng",
            Some(3),
            vec![],
        );

        let msg =
            forget_from_map(&mut mem, Some("preferences"), Some("Màu Yêu Thích"), None).unwrap();
        assert!(msg.contains("mau_yeu_thich"));
        assert!(!mem.contains_key("preferences"));

        let msg2 = forget_from_map(&mut mem, None, None, Some("thuc khuya")).unwrap();
        assert!(msg2.contains("ngu_muon"));
        assert!(mem.is_empty());
    }

    #[test]
    fn merges_streaming_turns_by_speaker() {
        let mut log = Vec::new();
        push_transcript_turn(&mut log, "user", "Hôm nay");
        push_transcript_turn(&mut log, "user", "hơi mệt");
        push_transcript_turn(&mut log, "user", "quá.");
        push_transcript_turn(&mut log, "assistant", "Nghỉ một chút");
        push_transcript_turn(&mut log, "assistant", "đi bạn.");
        assert_eq!(log.len(), 2);
        assert_eq!(log[0], "user: Hôm nay hơi mệt quá.");
        assert_eq!(log[1], "assistant: Nghỉ một chút đi bạn.");
    }

    #[test]
    fn legacy_memory_json_deserializes_with_defaults() {
        let legacy = r#"{"identity":{"language":{"value":"Vietnamese"}}}"#;
        let mem: Memory = serde_json::from_str(legacy).unwrap();
        let entry = &mem["identity"]["language"];
        assert_eq!(entry.value, "Vietnamese");
        assert_eq!(entry.importance, 3);
        assert_eq!(entry.access_count, 0);
    }
}
