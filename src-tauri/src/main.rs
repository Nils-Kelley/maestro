// Maestro — a project's harness, one keypress away. The Rust side reads and writes the files; the page never touches the disk
// and there is no port: every call below arrives over the window's own IPC.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

fn rd(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

/// Where a new maestro.config.json goes when there is none yet: beside the program, or in %APPDATA%\Maestro when that folder
/// cannot be written (an install under Program Files).
fn app_data() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Maestro"))
}
/// maestro.config.json beside the program, in a folder above it (target/release → the project folder), or in %APPDATA%\Maestro.
fn config_path() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![];
    if let Ok(exe) = std::env::current_exe() {
        let mut d = exe.parent().map(|p| p.to_path_buf());
        while let Some(p) = d {
            dirs.push(p.clone());
            d = p.parent().map(|x| x.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    if let Some(d) = app_data() {
        dirs.push(d);
    }
    dirs.iter().map(|d| d.join("maestro.config.json")).find(|f| f.exists())
}
fn config() -> Value {
    let mut c = config_path().and_then(|f| serde_json::from_str::<Value>(&rd(&f)).ok()).filter(|c| c.is_object()).unwrap_or(json!({}));
    if !c["projects"].is_array() { c["projects"] = json!([]); }
    c
}
/// Write the config back (the list of projects changed). The file as it was is kept beside it as .bak.
fn save_config(c: &Value) -> Result<(), String> {
    let text = format!("{}\n", serde_json::to_string_pretty(c).map_err(|e| e.to_string())?);
    if let Some(f) = config_path() {
        let _ = fs::copy(&f, f.with_extension("json.bak"));
        return fs::write(&f, text).map_err(|e| e.to_string());
    }
    // the first project on this machine: the file does not exist yet
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("maestro.config.json")));
    if let Some(f) = &beside {
        if fs::write(f, &text).is_ok() { return Ok(()); }
    }
    let dir = app_data().ok_or("there is nowhere to keep maestro.config.json: the app's folder cannot be written and %APPDATA% is not set")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join("maestro.config.json"), text).map_err(|e| e.to_string())
}

struct Project {
    name: String,
    root: PathBuf,
    claude_home: PathBuf,
    memory: PathBuf,
    wsl_root: String,
    distro: String,
    kit: String,
    audits: Vec<PathBuf>,
    /// The agent runs on Windows itself (no WSL): sessions are started directly and end with the app.
    native: bool,
    /// The folder that holds the harness: `.claude`, or `.agents` in a project that keeps it there.
    dot: PathBuf,
    /// The instructions file: CLAUDE.md, or AGENTS.md where that is the one the project has.
    instructions: PathBuf,
}
/// Which folder holds a project's harness. `.claude` unless the project keeps its rules, skills or log in `.agents` and has none
/// of them in `.claude` (a `.claude` that only holds settings does not count).
fn dot_of(root: &Path) -> PathBuf {
    let (claude, agents) = (root.join(".claude"), root.join(".agents"));
    let used = |d: &Path| ["rules", "skills", "log"].iter().any(|k| d.join(k).is_dir());
    if agents.is_dir() && !used(&claude) && (used(&agents) || !claude.is_dir()) { agents } else { claude }
}
/// CLAUDE.md if it is there; else AGENTS.md if that is; else whichever goes with the harness folder.
fn instructions_of(root: &Path, dot: &Path) -> PathBuf {
    let (c, a) = (root.join("CLAUDE.md"), root.join("AGENTS.md"));
    let agents = dot.file_name().map(|n| n == ".agents").unwrap_or(false);
    if agents && a.exists() { a } else if c.exists() { c } else if a.exists() || agents { a } else { c }
}
fn rel(dot: &Path) -> String {
    dot.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| ".claude".into())
}
/// Which project of maestro.config.json is open.
/// Each window shows a project of its own; a window not in the map shows the first one.
struct Cur(Mutex<std::collections::HashMap<String, usize>>);
fn cur_of(cur: &State<Cur>, window: &tauri::WebviewWindow) -> usize {
    cur.0.lock().ok().and_then(|m| m.get(window.label()).copied()).unwrap_or(0)
}
fn project(cur: &State<Cur>, window: &tauri::WebviewWindow) -> Result<Project, String> {
    let i = cur_of(cur, window);
    project_at(i)
}
/// The project at one place in the list (a session belongs to its project, whichever one the window is showing).
fn project_at(i: usize) -> Result<Project, String> {
    let c = config();
    let p = c["projects"].get(i).or_else(|| c["projects"].get(0)).ok_or("maestro.config.json names no project")?;
    let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
    let root = PathBuf::from(s("root"));
    let dot = dot_of(&root);
    Ok(Project { name: s("name"), native: s("runs") == "windows" || s("wslRoot").is_empty(), instructions: instructions_of(&root, &dot), dot, root, claude_home: PathBuf::from(s("claudeHome")), memory: PathBuf::from(s("memoryDir")), wsl_root: s("wslRoot"), kit: s("kit"), audits: p["audits"].as_array().cloned().unwrap_or_default().iter().filter_map(|a| a.as_str().map(PathBuf::from)).collect(), distro: if s("distro").is_empty() { "Ubuntu".into() } else { s("distro") } })
}

/// Run one kit command inside WSL, in the project's folder, and return what it printed. The kit (`ctx`) stays the one place
/// that knows how a ledger row closes; this app never re-implements it. `--exec` starts the kit's launcher with the arguments
/// as given: through a shell, wsl.exe re-splits the line and a quoted script arrives in pieces.
fn wsl(p: &Project, args: &[&str]) -> Result<String, String> {
    if p.wsl_root.is_empty() || p.kit.is_empty() { return Err("this needs a kit: a script named as \"kit\" for this project in maestro.config.json (see docs/kit.md)".into()); }
    let mut c = std::process::Command::new("wsl.exe");
    c.args(["-d", &p.distro, "--cd", &p.wsl_root, "--exec", "bash", &p.kit]);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // no console window
    }
    let out = c.output().map_err(|e| format!("could not start WSL: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.trim().is_empty() && !out.status.success() { return Err(String::from_utf8_lossy(&out.stderr).trim().to_string()); }
    Ok(text)
}
fn quiet(mut c: std::process::Command) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

/// A file name the page may ask for: one path segment, letters digits . _ -, ending in .md.
fn safe(f: &str) -> Result<&str, String> {
    let ok = !f.is_empty() && f.ends_with(".md") && !f.contains("..") && f.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    if ok { Ok(f) } else { Err(format!("not a file this app opens: {f}")) }
}

fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let z = secs.div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Keep the first copy of a file per day before it is changed. Nothing is deleted.
fn backup(root: &Path, kind: &str, file: &Path) {
    let dir = dot_of(root).join("archive").join(format!("{kind}-ui-{}", today()));
    if let Some(name) = file.file_name() {
        let to = dir.join(name);
        if file.exists() && !to.exists() {
            let _ = fs::create_dir_all(&dir);
            let _ = fs::copy(file, to);
        }
    }
}

// ---- a rules file as data: frontmatter · text before the first heading · sections of laws. parse → serialize is byte-exact ----
struct Sec {
    head: String,
    intro: String,
    laws: Vec<String>,
}
struct Doc {
    crlf: bool,
    fm: String,
    pre: String,
    secs: Vec<Sec>,
}
fn parse_rules(raw: &str) -> Doc {
    let crlf = raw.contains("\r\n");
    let text = if crlf { raw.replace("\r\n", "\n") } else { raw.to_string() };
    let mut fm = String::new();
    let mut rest: &str = &text;
    if text.starts_with("---\n") {
        if let Some(end) = text[4..].find("\n---\n") {
            let cut = 4 + end + 5;
            fm = text[..cut].to_string();
            rest = &text[cut..];
        }
    }
    let mut doc = Doc { crlf, fm, pre: String::new(), secs: vec![] };
    let mut in_law = false;
    for ln in rest.split_inclusive('\n') {
        if ln.starts_with("## ") {
            doc.secs.push(Sec { head: ln.to_string(), intro: String::new(), laws: vec![] });
            in_law = false;
            continue;
        }
        match doc.secs.last_mut() {
            None => doc.pre.push_str(ln),
            Some(sec) => {
                if ln.starts_with("- ") {
                    sec.laws.push(ln.to_string());
                    in_law = true;
                } else if in_law {
                    if let Some(l) = sec.laws.last_mut() {
                        l.push_str(ln);
                    }
                } else {
                    sec.intro.push_str(ln);
                }
            }
        }
    }
    doc
}
fn serialize_rules(d: &Doc) -> String {
    let mut t = String::new();
    t.push_str(&d.fm);
    t.push_str(&d.pre);
    for s in &d.secs {
        t.push_str(&s.head);
        t.push_str(&s.intro);
        for l in &s.laws {
            t.push_str(l);
        }
    }
    if d.crlf { t.replace('\n', "\r\n") } else { t }
}
fn body_of(l: &str) -> &str {
    l.trim_end_matches('\n')
}
/// The blank lines that follow a rule (they close its section).
fn tail_of(l: &str) -> String {
    let n = l.len() - l.trim_end_matches('\n').len();
    if n > 1 { "\n".repeat(n - 1) } else { String::new() }
}
/// Take a rule out; the blank lines that closed the section stay with the section.
fn remove_law(sec: &mut Sec, i: usize) -> String {
    let law = sec.laws.remove(i);
    let (body, tail) = (body_of(&law).to_string(), tail_of(&law));
    if i == sec.laws.len() && !tail.is_empty() {
        if let Some(prev) = sec.laws.last_mut() {
            *prev = format!("{}\n{}", body_of(prev), tail);
        } else {
            let t = sec.intro.trim_end_matches('\n').to_string();
            sec.intro = if t.trim().is_empty() { tail } else { format!("{t}\n{tail}") };
        }
    }
    body
}
/// Put a rule at the end of a section, in front of the blank lines that close it.
fn append_law(sec: &mut Sec, body: &str) {
    if let Some(last) = sec.laws.last_mut() {
        let tail = tail_of(last);
        *last = format!("{}\n", body_of(last));
        sec.laws.push(format!("{body}\n{tail}"));
    } else {
        let tail = tail_of(&sec.intro);
        if sec.intro.trim().is_empty() { sec.intro.clear(); } else { sec.intro = format!("{}\n", sec.intro.trim_end_matches('\n')); }
        sec.laws.push(format!("{body}\n{tail}"));
    }
}
/// Add a rule to a document: into section `to` or into a new section at the end.
fn place(d: &mut Doc, to: Option<usize>, new_section: &Option<String>, body: &str) -> Result<(), String> {
    if let Some(name) = new_section.as_ref().map(|n| n.replace('\n', " ").trim().to_string()).filter(|n| !n.is_empty()) {
        match d.secs.last_mut() {
            Some(last) => match last.laws.last_mut() {
                Some(l) => *l = format!("{}\n\n", body_of(l)),
                None => last.intro = format!("{}\n\n", last.intro.trim_end_matches('\n')),
            },
            None => d.pre = format!("{}\n\n", d.pre.trim_end_matches('\n')),
        }
        d.secs.push(Sec { head: format!("## {name}\n"), intro: String::new(), laws: vec![] });
        let n = d.secs.len() - 1;
        append_law(&mut d.secs[n], body);
        return Ok(());
    }
    let sec = to.and_then(|t| d.secs.get_mut(t)).ok_or("no such section in the target file")?;
    append_law(sec, body);
    Ok(())
}
/// The rule at (s, i), checked against what the page was showing.
fn checked<'a>(d: &'a mut Doc, s: usize, i: usize, was: &str) -> Result<&'a mut Sec, String> {
    let sec = d.secs.get_mut(s).ok_or("that rule is no longer there: the file changed, reopen it")?;
    let law = sec.laws.get(i).ok_or("that rule is no longer there: the file changed, reopen it")?;
    if law.trim_end() != was.trim_end() { return Err("the file changed since you opened this rule: reopen it".into()); }
    Ok(sec)
}
fn globs_of(fm: &str) -> Vec<String> {
    fm.lines().filter_map(|l| l.trim().strip_prefix("- ")).map(|g| g.trim().trim_matches('"').trim_matches('\'').to_string()).filter(|g| !g.is_empty()).collect()
}

fn md_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir).map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "md").unwrap_or(false)).collect()).unwrap_or_default();
    v.sort();
    v
}
fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}
fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\''))) { t[1..t.len() - 1].replace("\\\"", "\"") } else { t.to_string() }
}

/// Everything the page shows, read fresh from the files.
#[tauri::command]
fn load(cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    if config()["projects"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
        // nothing is set up yet: the page shows its welcome screen over this empty read
        return Ok(json!({
            "empty": true, "project": { "name": "", "root": "", "dir": ".claude", "instructions": "CLAUDE.md", "native": false, "kit": false },
            "limit": 120000, "sizes": { "projClaude": 0, "userClaude": 0, "memoryIndex": 0 }, "rules": [], "memories": [], "log": [],
            "ledger": { "total": 0, "status": {}, "repos": {} }, "sessions": { "drafts": 0, "live": 0, "abandoned": 0, "recent": [], "past": [] },
            "registry": [], "skills": [], "hooks": [], "audits": [], "probes": null, "hot": [], "hotkey": config()["hotkey"],
        }));
    }
    let p = project(&cur, &window)?;
    let claude = p.dot.clone();

    let rules: Vec<Value> = md_files(&claude.join("rules"))
        .iter()
        .map(|f| {
            let raw = rd(f);
            let d = parse_rules(&raw);
            json!({ "f": name_of(f), "chars": raw.chars().count(), "globs": globs_of(&d.fm),
                "sections": d.secs.iter().map(|s| json!({ "head": s.head[3..].trim(), "laws": s.laws.iter().map(|l| l.trim_end()).collect::<Vec<_>>() })).collect::<Vec<_>>() })
        })
        .collect();

    let memories: Vec<Value> = md_files(&p.memory)
        .iter()
        .filter(|f| name_of(f) != "MEMORY.md")
        .map(|f| {
            let raw = rd(f).replace("\r\n", "\n");
            let (mut name, mut desc, mut kind) = (String::new(), String::new(), String::new());
            let mut body = raw.as_str();
            if raw.starts_with("---\n") {
                if let Some(end) = raw[4..].find("\n---") {
                    for ln in raw[4..4 + end].lines() {
                        let t = ln.trim();
                        if let Some(v) = t.strip_prefix("name:") { name = unquote(v); }
                        if let Some(v) = t.strip_prefix("description:") { desc = unquote(v); }
                        if let Some(v) = t.strip_prefix("type:") { kind = unquote(v); }
                    }
                    body = raw[4 + end + 4..].trim_start_matches('\n');
                }
            }
            let modified = fs::metadata(f).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            json!({ "f": name_of(f), "name": if name.is_empty() { name_of(f).replace(".md", "") } else { name }, "desc": desc, "type": kind, "body": body, "chars": raw.chars().count(), "modified": modified })
        })
        .collect();

    // the work log: an index if a kit keeps one (INDEX.json), else the dated files themselves
    let mut log: Vec<Value> = serde_json::from_str::<Value>(&rd(&claude.join("log").join("INDEX.json")))
        .ok()
        .and_then(|v| v["entries"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|e| json!({ "date": e["date"], "title": e["title"], "slug": e["slug"], "pending": e["pending"], "tags": e["tags"] }))
        .collect();
    if log.is_empty() {
        log = log_from_files(&claude.join("log"));
    }

    let mut status = serde_json::Map::new();
    let mut total = 0;
    for ln in rd(&claude.join("state").join("deploy.md")).lines() {
        if !ln.starts_with("| L") { continue; }
        let cells: Vec<&str> = ln.split('|').map(|c| c.trim()).collect();
        if cells.len() < 10 { continue; }
        total += 1;
        let word = cells[8].split(|c: char| c == ' ' || c == '(').next().unwrap_or("").to_string();
        let n = status.get(&word).and_then(|v| v.as_i64()).unwrap_or(0);
        status.insert(word, json!(n + 1));
    }
    let heads: Value = serde_json::from_str(&rd(&claude.join("state").join("prod-heads.json"))).unwrap_or(json!({}));

    let (mut live, mut abandoned, mut drafts) = (0, 0, 0);
    if let Ok(dir) = fs::read_dir(claude.join("state").join("sessions")) {
        for e in dir.filter_map(|e| e.ok()) {
            if e.path().extension().map(|x| x == "md").unwrap_or(false) {
                drafts += 1;
                let head: String = rd(&e.path()).lines().take(3).collect::<Vec<_>>().join("\n");
                if head.contains("status: live") { live += 1; }
                if head.contains("abandoned") { abandoned += 1; }
            }
        }
    }
    let mut recent: Vec<Value> = vec![];
    if let Ok(dir) = fs::read_dir(claude.join("state").join("sessions")) {
        for e in dir.filter_map(|e| e.ok()) {
            let path = e.path();
            if !path.extension().map(|x| x == "md").unwrap_or(false) { continue; }
            let text = rd(&path);
            let line = |key: &str| text.lines().find_map(|l| l.find(key).map(|at| l[at + key.len()..].trim().to_string())).unwrap_or_default();
            let st = text.lines().find(|l| l.starts_with("status:")).unwrap_or("").to_string();
            let state = if st.trim() == "status: live" { "live" } else if st.contains("abandoned") { "abandoned" } else { "ended" };
            let up = line("last update: ").split_whitespace().next().unwrap_or("").to_string();
            let ask = text.lines().skip_while(|l| !l.starts_with("## What the user asked")).nth(1).unwrap_or("").trim_start_matches("- ").to_string();
            recent.push(json!({ "id": name_of(&path).replace(".md", ""), "state": state, "up": up, "ask": redact(&ask), "files": text.lines().filter(|l| l.starts_with("- ") && l.contains(" ×")).count() }));
        }
    }
    recent.sort_by(|a, b| b["up"].as_str().unwrap_or("").cmp(a["up"].as_str().unwrap_or("")));
    recent.truncate(25);
    let registry: Vec<Value> = serde_json::from_str::<Value>(&rd(&p.root.join("scripts").join("probes.json")))
        .ok()
        .and_then(|v| v["probes"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|x| json!({ "name": x["name"], "area": x["area"], "last": x["last_pass"], "guards": x["paths"].as_array().map(|a| a.len()).unwrap_or(0) }))
        .collect();
    let mut skills: Vec<Value> = vec![];
    for (scope, dir) in [("you", p.claude_home.join("skills")), ("project", claude.join("skills"))] {
        let mut names: Vec<PathBuf> = fs::read_dir(&dir).map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).filter(|d| d.join("SKILL.md").exists()).collect()).unwrap_or_default();
        names.sort();
        for d in names {
            let raw = rd(&d.join("SKILL.md")).replace("\r\n", "\n");
            let (fm, body) = split_front(&raw);
            let desc = fm.lines().find_map(|l| l.trim().strip_prefix("description:")).map(unquote).unwrap_or_default();
            skills.push(json!({ "name": name_of(&d), "scope": scope, "desc": desc, "body": body, "chars": raw.chars().count() }));
        }
    }
    let mut hooks: Vec<Value> = vec![];
    for (scope, file) in [("you", p.claude_home.join("settings.json")), ("project", p.root.join(".claude").join("settings.json"))] {
        let j: Value = serde_json::from_str(&rd(&file)).unwrap_or(json!({}));
        if let Some(events) = j["hooks"].as_object() {
            for (ev, groups) in events {
                for g in groups.as_array().cloned().unwrap_or_default() {
                    for h in g["hooks"].as_array().cloned().unwrap_or_default() {
                        let runs = if h["type"] == "agent" { format!("an agent: {}", h["prompt"].as_str().unwrap_or("").chars().take(120).collect::<String>()) } else { h["command"].as_str().unwrap_or("").to_string() };
                        hooks.push(json!({ "scope": scope, "when": ev, "on": g["matcher"].as_str().unwrap_or("always"), "runs": runs }));
                    }
                }
            }
        }
    }
    let mut audits: Vec<Value> = audit_files(&p)
        .iter()
        .map(|(label, f)| {
            let raw = rd(f);
            let name = name_of(f);
            let title = raw.lines().find_map(|l| l.strip_prefix("# ")).map(|t| t.trim().to_string()).unwrap_or_else(|| name.clone());
            let modified = fs::metadata(f).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
            let date = date_in(&name).or_else(|| date_in(label)).unwrap_or_else(|| day_of(modified));
            let lead: String = raw.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("---") && !l.starts_with('|')).take(3).collect::<Vec<_>>().join(" ").chars().take(320).collect();
            json!({ "kind": "doc", "title": title, "file": name, "date": date, "where": label, "path": f.to_string_lossy(), "chars": raw.chars().count(), "lead": lead })
        })
        .collect();
    for e in &log {
        let t = e["title"].as_str().unwrap_or("");
        if t.to_lowercase().contains("audit") {
            audits.push(json!({ "kind": "log", "title": t, "date": e["date"], "where": "work log", "slug": e["slug"], "lead": "" }));
        }
    }
    audits.sort_by(|a, b| b["date"].as_str().unwrap_or("").cmp(a["date"].as_str().unwrap_or("")));
    let probes: Value = serde_json::from_str(&rd(&claude.join("state").join("probes-audit.json"))).unwrap_or(Value::Null);
    // the files whose rule load the home screen measures, and the limit it is held against: a kit's ctx.json or the project's own maestro.json
    let ctx: Value = serde_json::from_str(&rd(&claude.join("ctx.json"))).unwrap_or(json!({}));
    let own: Value = serde_json::from_str(&rd(&claude.join("maestro.json"))).unwrap_or(json!({}));
    let hot = if own["hot"].is_array() { own["hot"].clone() } else { ctx["size"]["hot"].clone() };
    let past = if recent.is_empty() { past_conversations(&p) } else { vec![] };

    Ok(json!({
        "project": { "name": p.name, "root": p.root.to_string_lossy(), "dir": rel(&p.dot), "instructions": name_of(&p.instructions), "native": p.native, "kit": !p.kit.is_empty(),
            "bare": !p.instructions.exists() && md_files(&claude.join("rules")).is_empty() && !claude.join("skills").is_dir() },
        "limit": own["limit"].as_u64().unwrap_or(120000),
        "sizes": { "projClaude": rd(&p.instructions).chars().count(), "userClaude": rd(&p.claude_home.join("CLAUDE.md")).chars().count(), "memoryIndex": rd(&p.memory.join("MEMORY.md")).chars().count() },
        "rules": rules, "memories": memories, "log": log,
        "ledger": { "total": total, "status": status, "repos": heads["repos"] },
        "sessions": { "drafts": drafts, "live": live, "abandoned": abandoned, "recent": recent, "past": past },
        "registry": registry, "skills": skills, "hooks": hooks, "audits": audits,
        "probes": probes, "hot": hot, "hotkey": config()["hotkey"],
    }))
}

/// The log read from its files: every `YYYY-MM-DD-slug.md`, titled by its first `# ` line. Newest first.
fn log_from_files(dir: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = md_files(dir)
        .iter()
        .filter_map(|f| {
            let name = name_of(f);
            let date = date_in(&name).filter(|d| name.starts_with(d.as_str()))?;
            let head: String = { use std::io::Read; let mut b = String::new(); let _ = fs::File::open(f).ok()?.take(4096).read_to_string(&mut b); b };
            let slug = name.trim_end_matches(".md").to_string();
            let title = head.lines().find_map(|l| l.strip_prefix("# ")).map(|t| t.trim().to_string()).unwrap_or_else(|| slug[date.len()..].trim_start_matches('-').replace('-', " "));
            Some(json!({ "date": date, "title": title, "slug": slug, "pending": false, "tags": [] }))
        })
        .collect();
    out.sort_by(|a, b| b["slug"].as_str().unwrap_or("").cmp(a["slug"].as_str().unwrap_or("")));
    out
}
/// Earlier conversations, read from the transcripts Claude Code keeps beside the project's memory (one .jsonl per conversation):
/// the id, when it was last written, and the first thing that was asked. Used where no kit keeps session drafts.
fn past_conversations(p: &Project) -> Vec<Value> {
    use std::io::Read;
    let Some(dir) = p.memory.parent() else { return vec![] };
    let mut files: Vec<(u64, PathBuf)> = fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).filter(|f| f.extension().map(|x| x == "jsonl").unwrap_or(false)).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|f| (fs::metadata(&f).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0), f))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = vec![];
    for (secs, f) in files.into_iter().take(40) {
        let mut buf = Vec::new();
        if fs::File::open(&f).map(|h| h.take(300_000).read_to_end(&mut buf)).is_err() { continue; }
        let text = String::from_utf8_lossy(&buf);
        let ask = text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["type"] == "user" && v["isMeta"] != true && v["isSidechain"] != true).find_map(|v| {
            let c = &v["message"]["content"];
            let said = c.as_str().map(|t| t.to_string()).or_else(|| c.as_array().and_then(|a| a.iter().find_map(|part| if part["type"] == "text" { part["text"].as_str().map(|t| t.to_string()) } else { None })))?;
            let said = said.trim();
            if said.is_empty() || said.starts_with('<') || said.starts_with("Caveat:") { return None; } // what the program wrote, not the person
            Some(redact(said)).filter(|t| !t.is_empty())
        });
        if let Some(ask) = ask {
            out.push(json!({ "id": name_of(&f).trim_end_matches(".jsonl"), "state": "ended", "up": secs, "ask": ask, "files": 0 }));
            if out.len() == 12 { break; }
        }
    }
    out
}
/// The first YYYY-MM-DD inside a name, if it holds one.
fn date_in(name: &str) -> Option<String> {
    let b = name.as_bytes();
    (0..b.len().saturating_sub(9)).find(|&i| {
        let d = |k: usize| b[i + k].is_ascii_digit();
        b[i] == b'2' && b[i + 1] == b'0' && d(2) && d(3) && b[i + 4] == b'-' && d(5) && d(6) && b[i + 7] == b'-' && d(8) && d(9)
    }).map(|i| name[i..i + 10].to_string())
}
fn day_of(secs: i64) -> String {
    let z = secs.div_euclid(86400) + 719468;
    let (era, doe) = (z.div_euclid(146097), z.rem_euclid(146097));
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    format!("{:04}-{m:02}-{d:02}", yoe + era * 400 + if m <= 2 { 1 } else { 0 })
}
/// Where audits are kept: any file with "audit" in its name under the project's reference, planning and docs folders, and every
/// .md / .txt of the extra folders the config names for this project (`audits`). Each comes with the label of where it sits.
fn audit_files(p: &Project) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = vec![];
    let text = |f: &PathBuf| f.extension().map(|x| x == "md" || x == "txt").unwrap_or(false);
    for (label, dir) in [("reference", p.dot.join("reference")), (".planning", p.root.join(".planning")), ("docs", p.root.join("docs"))] {
        if let Ok(rd) = fs::read_dir(&dir) {
            for f in rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|f| f.is_file() && text(f) && name_of(f).to_lowercase().contains("audit")) {
                out.push((label.to_string(), f));
            }
        }
    }
    for dir in &p.audits {
        let label = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if let Ok(rd) = fs::read_dir(dir) {
            for f in rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|f| f.is_file() && text(f)) {
                out.push((label.clone(), f));
            }
        }
    }
    out
}
/// One audit document's text. Only a file the list above holds is opened.
#[tauri::command]
fn read_audit(path: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    let p = project(&cur, &window)?;
    let want = PathBuf::from(&path);
    if !audit_files(&p).iter().any(|(_, f)| *f == want) { return Err("that file is not one of this project's audits".into()); }
    Ok(rd(&want))
}

/// A file's `---` header and the text after it.
fn split_front(raw: &str) -> (&str, &str) {
    if raw.starts_with("---\n") {
        if let Some(end) = raw[4..].find("\n---") {
            let after = &raw[4 + end + 4..];
            return (&raw[4..4 + end], after.trim_start_matches('\n'));
        }
    }
    ("", raw)
}
/// A name that becomes a file or folder: lower-case letters, digits and dashes.
fn slug(name: &str) -> Result<String, String> {
    let n = name.trim().to_lowercase().replace(' ', "-");
    let ok = !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !n.starts_with('-');
    if ok { Ok(n) } else { Err("a name is lower-case letters, digits and dashes (for example release-checklist)".into()) }
}
fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ").trim().to_string()
}
fn skill_dir(p: &Project, scope: &str) -> Result<PathBuf, String> {
    match scope {
        "you" => Ok(p.claude_home.join("skills")),
        "project" => Ok(p.dot.join("skills")),
        _ => Err("a skill belongs to you or to the project".into()),
    }
}
fn keep_copy(dir: &Path, label: &str, file: &Path) {
    let to_dir = dir.join(format!("{label}-{}", today()));
    let to = to_dir.join(format!("{}-{}", file.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), name_of(file)));
    if file.exists() && !to.exists() {
        let _ = fs::create_dir_all(&to_dir);
        let _ = fs::copy(file, to);
    }
}

/// A skill's SKILL.md whole, for editing.
#[tauri::command]
fn read_skill(scope: String, name: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    let p = project(&cur, &window)?;
    Ok(rd(&skill_dir(&p, &scope)?.join(slug(&name)?).join("SKILL.md")))
}
#[tauri::command]
fn save_skill(scope: String, name: String, text: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    let p = project(&cur, &window)?;
    let file = skill_dir(&p, &scope)?.join(slug(&name)?).join("SKILL.md");
    if !file.exists() { return Err(format!("no skill named {name}")); }
    if !text.starts_with("---") { return Err("a skill starts with its --- header (name and description): keep it".into()); }
    keep_copy(&p.claude_home.join("backups"), "skills-ui", &file);
    fs::write(&file, text).map_err(|e| e.to_string())
}
/// A new skill: a folder with a SKILL.md that says when to use it and what to do.
#[tauri::command]
fn new_skill(scope: String, name: String, description: String, body: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    let p = project(&cur, &window)?;
    let n = slug(&name)?;
    let desc = one_line(&description);
    if desc.is_empty() { return Err("say when the agent should use it: the description is how a skill gets picked".into()); }
    let dir = skill_dir(&p, &scope)?.join(&n);
    if dir.exists() { return Err(format!("a skill named {n} already exists")); }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let steps = if body.trim().is_empty() { "Write the steps here.".to_string() } else { body.replace("\r\n", "\n").trim().to_string() };
    fs::write(dir.join("SKILL.md"), format!("---\nname: {n}\ndescription: {desc}\n---\n\n# /{n}\n\n{steps}\n")).map_err(|e| e.to_string())?;
    Ok(n)
}
/// A new memory: one fact in its own file, and a one-line pointer in MEMORY.md (the index the agent reads every session).
#[tauri::command]
fn new_memory(name: String, kind: String, description: String, body: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    let p = project(&cur, &window)?;
    let n = slug(&name)?;
    if !["user", "feedback", "project", "reference"].contains(&kind.as_str()) { return Err("a memory is one of: user, feedback, project, reference".into()); }
    let desc = one_line(&description);
    if desc.is_empty() { return Err("give it a one-line summary: that is what decides when it is recalled".into()); }
    if body.trim().is_empty() { return Err("write the fact itself".into()); }
    let file = p.memory.join(format!("{n}.md"));
    if file.exists() { return Err(format!("a memory named {n} already exists: edit that one")); }
    fs::create_dir_all(&p.memory).map_err(|e| e.to_string())?;
    fs::write(&file, format!("---\nname: {n}\ndescription: {}\nmetadata:\n  type: {kind}\n---\n\n{}\n", serde_json::to_string(&desc).unwrap_or_default(), body.replace("\r\n", "\n").trim())).map_err(|e| e.to_string())?;
    let index = p.memory.join("MEMORY.md");
    let cur_index = rd(&index);
    let mut words = n.replace('-', " ");
    if let Some(first) = words.get_mut(0..1) { first.make_ascii_uppercase(); }
    let hook: String = desc.chars().take(150).collect();
    let pointer = format!("- [{words}]({n}.md) — {hook}\n");
    let next = if cur_index.trim().is_empty() { format!("# Memory Index\n{pointer}") } else {
        // right under the title, where the newest pointers sit
        match cur_index.find('\n') { Some(at) if cur_index.starts_with('#') => format!("{}{}{}", &cur_index[..=at], pointer, &cur_index[at + 1..]), _ => format!("{pointer}{cur_index}") }
    };
    backup(&p.root, "memory", &index);
    fs::write(&index, next).map_err(|e| e.to_string())?;
    Ok(format!("{n}.md"))
}

/// One log entry's text.
#[tauri::command]
fn read_log(slug: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    let p = project(&cur, &window)?;
    let f = format!("{slug}.md");
    Ok(rd(&p.dot.join("log").join(safe(&f)?)))
}

/// A memory file whole (frontmatter and all), for editing.
#[tauri::command]
fn read_memory(f: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    let p = project(&cur, &window)?;
    Ok(rd(&p.memory.join(safe(&f)?)))
}

#[tauri::command]
fn save_memory(f: String, text: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    let p = project(&cur, &window)?;
    let file = p.memory.join(safe(&f)?);
    if !file.exists() { return Err(format!("no memory named {f}")); }
    if !text.starts_with("---") { return Err("a memory starts with its --- header: keep it".into()); }
    backup(&p.root, "memory", &file);
    fs::write(&file, text).map_err(|e| e.to_string())
}

/// Replace one rule's text. `s` and `i` are the section and the rule's place in it; the rule must still read `was` (the file may have changed).
#[tauri::command]
fn save_law(f: String, s: usize, i: usize, was: String, text: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    let p = project(&cur, &window)?;
    let file = p.dot.join("rules").join(safe(&f)?);
    let raw = fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let mut d = parse_rules(&raw);
    let body = text.replace("\r\n", "\n").trim_end().to_string();
    if !body.starts_with("- ") { return Err("a rule starts with \"- \"".into()); }
    let law = d.secs.get_mut(s).and_then(|sec| sec.laws.get_mut(i)).ok_or("that rule is no longer there: the file changed, reopen it")?;
    if law.trim_end() != was.trim_end() { return Err("the file changed since you opened this rule: reopen it".into()); }
    let tail: String = law[law.trim_end().len()..].to_string();
    *law = format!("{body}{tail}");
    backup(&p.root, "rules", &file);
    fs::write(&file, serialize_rules(&d)).map_err(|e| e.to_string())
}

/// A session's first request may hold a pasted login: drop what follows "password:" and the like, and name#digits tokens.
fn redact(s: &str) -> String {
    // pasted blocks arrive wrapped in tags: drop everything between < and >
    let mut plain = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => { in_tag = false; plain.push(' '); }
            _ if !in_tag => plain.push(ch),
            _ => {}
        }
    }
    const KEYS: [&str; 6] = ["username:", "password:", "pin:", "id:", "pw:", "pass:"];
    let mut out: Vec<&str> = vec![];
    let mut skip = false;
    for tok in plain.split_whitespace() {
        if skip { skip = false; continue; }
        let low = tok.to_lowercase();
        if KEYS.contains(&low.as_str()) { skip = true; continue; }
        if KEYS.iter().any(|k| low.starts_with(k)) { continue; } // "ID:42"
        if let Some(at) = tok.find('#') {
            if at > 0 && tok[at + 1..].chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) { continue; }
        }
        out.push(tok);
    }
    out.join(" ").chars().take(110).collect()
}

/// Move one rule to another rules file (or another section of its own), at the end of a section or into a new one.
#[tauri::command]
fn move_law(f: String, s: usize, i: usize, was: String, to: String, to_section: Option<usize>, new_section: Option<String>, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    let p = project(&cur, &window)?;
    let dir = p.dot.join("rules");
    let (from, dest) = (dir.join(safe(&f)?), dir.join(safe(&to)?));
    let mut src = parse_rules(&fs::read_to_string(&from).map_err(|e| e.to_string())?);
    let named = new_section.as_ref().map(|n| !n.trim().is_empty()).unwrap_or(false);
    if f == to && !named && to_section == Some(s) { return Err("it is already in that section".into()); }
    let body = remove_law(checked(&mut src, s, i, &was)?, i);
    if f == to {
        place(&mut src, to_section, &new_section, &body)?;
        backup(&p.root, "rules", &from);
        return fs::write(&from, serialize_rules(&src)).map_err(|e| e.to_string());
    }
    let mut dst = parse_rules(&fs::read_to_string(&dest).map_err(|e| e.to_string())?);
    place(&mut dst, to_section, &new_section, &body)?;
    backup(&p.root, "rules", &from);
    backup(&p.root, "rules", &dest);
    fs::write(&dest, serialize_rules(&dst)).map_err(|e| e.to_string())?; // the rule lands before it leaves: a failure can double it, never lose it
    fs::write(&from, serialize_rules(&src)).map_err(|e| e.to_string())
}

/// Take a rule out of its file; it is kept, with where it came from, in the harness folder's archive/retired-laws.md.
#[tauri::command]
fn retire_law(f: String, s: usize, i: usize, was: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    let p = project(&cur, &window)?;
    let file = p.dot.join("rules").join(safe(&f)?);
    let mut d = parse_rules(&fs::read_to_string(&file).map_err(|e| e.to_string())?);
    let sec = checked(&mut d, s, i, &was)?;
    let head = sec.head[3..].trim().to_string();
    let body = remove_law(sec, i);
    let arch = p.dot.join("archive");
    fs::create_dir_all(&arch).map_err(|e| e.to_string())?;
    let keep = arch.join("retired-laws.md");
    let mut all = rd(&keep);
    if all.is_empty() { all.push_str("# Retired rules — each with the file and section it came from\n"); }
    all.push_str(&format!("\n## {} · {} · {}\n{}\n", today(), f, head, body));
    fs::write(&keep, all).map_err(|e| e.to_string())?;
    backup(&p.root, "rules", &file);
    fs::write(&file, serialize_rules(&d)).map_err(|e| e.to_string())
}

/// The open ledger rows and why each is open (the kit works it out, with git, inside WSL).
#[tauri::command(async)]
fn ledger(cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    let out = wsl(&p, &["ui", "--ledger-json"])?;
    let at = out.find('{').ok_or("the kit printed no ledger")?;
    serde_json::from_str(&out[at..]).map_err(|e| format!("could not read the kit's ledger: {e}"))
}
/// Record that production runs this commit of this repo; the kit then closes every row the commit contains.
#[tauri::command(async)]
fn mark_live(repo: String, sha: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    let p = project(&cur, &window)?;
    if repo.is_empty() || !repo.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { return Err("that is not a repo name".into()); }
    if sha.len() < 7 || sha.len() > 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) { return Err("that is not a commit id (7 to 40 hex characters)".into()); }
    wsl(&p, &["deploy", "--mark", &repo, &sha, "--by", "Maestro"])
}
#[tauri::command(async)]
fn close_rows(cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    wsl(&project(&cur, &window)?, &["deploy", "--close"])
}
/// Run the probe audit (about twenty seconds) and return what it printed.
#[tauri::command(async)]
fn audit(cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    wsl(&project(&cur, &window)?, &["probes", "audit"])
}

/// Mark a live session draft as ended.
#[tauri::command]
fn end_session(id: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    let p = project(&cur, &window)?;
    if id.len() < 8 || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') { return Err("not a session id".into()); }
    let file = p.dot.join("state").join("sessions").join(format!("{id}.md"));
    let text = fs::read_to_string(&file).map_err(|e| e.to_string())?;
    if !text.lines().any(|l| l.trim_end() == "status: live") { return Err("that draft is not live".into()); }
    let crlf = text.contains("\r\n");
    let out: Vec<String> = text.replace("\r\n", "\n").split('\n').map(|l| if l == "status: live" { format!("status: ended {} (marked in Maestro)", today()) } else { l.to_string() }).collect();
    let joined = out.join("\n");
    fs::write(&file, if crlf { joined.replace('\n', "\r\n") } else { joined }).map_err(|e| e.to_string())
}

/// The WSL distros on this machine, the default one first (`wsl.exe -l -q` prints UTF-16). Empty where WSL is not installed.
fn wsl_distros() -> Vec<String> {
    let mut c = std::process::Command::new("wsl.exe");
    c.args(["-l", "-q"]);
    let Ok(out) = quiet(c).output() else { return vec![] };
    if !out.status.success() { return vec![]; }
    let wide: Vec<u16> = out.stdout.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    String::from_utf16_lossy(&wide).lines().map(|l| l.trim().trim_matches('\u{feff}').to_string()).filter(|l| !l.is_empty() && !l.starts_with("docker-desktop")).collect()
}
/// The user's home inside a distro, the Linux way (/home/me). This starts the distro if it was not running.
fn wsl_home(distro: &str) -> Option<String> {
    let mut c = std::process::Command::new("wsl.exe");
    c.args(["-d", distro, "--exec", "printenv", "HOME"]);
    let out = quiet(c).output().ok()?;
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if t.starts_with('/') { Some(t) } else { None }
}
/// \\wsl.localhost\Ubuntu\home\me → /home/me
fn linux_of(unc: &str) -> Option<String> {
    let low = unc.to_lowercase();
    if !(low.starts_with("\\\\wsl.localhost\\") || low.starts_with("\\\\wsl$\\")) { return None; }
    let rest: Vec<&str> = unc.trim_start_matches('\\').split('\\').collect(); // host, distro, path…
    if rest.len() < 3 { return None; }
    Some(format!("/{}", rest[2..].join("/")))
}
/// Claude Code's own folder when it runs on Windows itself: %USERPROFILE%\.claude
fn windows_claude_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".claude"))
}
/// Claude names a project's folder after its path with every sign that is not a letter or a digit turned into a dash.
fn dashed(path: &str) -> String {
    path.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' }).collect()
}
struct Folder {
    /// the folder as Windows opens it
    root: String,
    /// the same folder as WSL sees it ("" when it cannot: no such mount)
    wsl_root: String,
    /// the distro the path itself names, if it names one
    distro: Option<String>,
    /// the folder is on WSL's own disk, so only an agent inside WSL can work in it
    inside: bool,
}
/// Read a folder typed the Windows way (D:\projects\x), the WSL way (/mnt/d/projects/x, or /home/me/x for a folder inside WSL)
/// or as a \\wsl.localhost\<distro>\… path. `distro` is the one to assume for a bare /home/… path.
fn read_folder(folder: &str, distro: &str) -> Result<Folder, String> {
    let t = folder.trim().trim_matches('"').trim_end_matches(['\\', '/']).to_string();
    if t.is_empty() { return Err("type or pick the project's folder".into()); }
    let low = t.to_lowercase();
    if low.starts_with("\\\\wsl.localhost\\") || low.starts_with("\\\\wsl$\\") {
        let rest: Vec<&str> = t.trim_start_matches('\\').split('\\').collect();
        if rest.len() < 3 { return Err("that WSL path names no folder".into()); }
        return Ok(Folder { wsl_root: format!("/{}", rest[2..].join("/")), distro: Some(rest[1].to_string()), root: t, inside: true });
    }
    if t.starts_with("/mnt/") && t.len() > 6 && t.as_bytes()[6] == b'/' && t.as_bytes()[5].is_ascii_alphabetic() {
        return Ok(Folder { root: format!("{}:\\{}", t[5..6].to_uppercase(), t[7..].replace('/', "\\")), wsl_root: t, distro: None, inside: false });
    }
    if t.starts_with('/') {
        if distro.is_empty() { return Err("that is a path inside WSL, and no WSL distro was found on this machine".into()); }
        return Ok(Folder { root: format!("\\\\wsl.localhost\\{distro}{}", t.replace('/', "\\")), wsl_root: t, distro: None, inside: true });
    }
    if t.len() > 2 && t.as_bytes()[1] == b':' && t.as_bytes()[0].is_ascii_alphabetic() {
        let root = t.replace('/', "\\");
        return Ok(Folder { wsl_root: format!("/mnt/{}{}", t[..1].to_lowercase(), root[2..].replace('\\', "/")), root, distro: None, inside: false });
    }
    Err("give the whole path, for example D:\\projects\\my-site or /mnt/d/projects/my-site".into())
}
/// Work out a project's entry from its folder alone: where the folder is, whether its agent runs inside WSL or on Windows itself,
/// and where Claude keeps that project's memory. `runs` ("wsl" or "windows") is the user's own answer; without it the app looks
/// at which side has already worked in the folder, then at what the other projects of the list do, then at what is installed.
fn derive(folder: &str, runs: Option<&str>) -> Result<Value, String> {
    let c = config();
    let list = c["projects"].as_array().cloned().unwrap_or_default();
    let is_native = |p: &Value| p["runs"] == "windows" || p["wslRoot"].as_str().unwrap_or("").is_empty();
    let known = list.iter().find(|p| !is_native(p)).and_then(|p| p["distro"].as_str()).filter(|d| !d.is_empty()).map(|d| d.to_string());
    let mut distro = known.clone().or_else(|| wsl_distros().into_iter().next()).unwrap_or_default();
    let f = read_folder(folder, &distro)?;
    if let Some(d) = &f.distro { distro = d.clone(); }
    let root_path = PathBuf::from(&f.root);
    if !root_path.is_dir() { return Err(format!("there is no folder at {}", f.root)); }

    // Claude's folder on each side, and whether that side has a record of this project
    let win_home = windows_claude_home();
    let win_slug = dashed(&f.root);
    let win_has = !f.inside && win_home.as_ref().map(|h| h.join("projects").join(&win_slug).is_dir()).unwrap_or(false);
    let same = list.iter().find(|p| !is_native(p) && p["distro"].as_str() == Some(distro.as_str()));
    let wsl_claude: Option<String> = if distro.is_empty() { None } else {
        same.and_then(|p| p["claudeHome"].as_str()).filter(|h| linux_of(h).is_some()).map(|h| h.to_string())
            .or_else(|| wsl_home(&distro).map(|h| format!("\\\\wsl.localhost\\{distro}{}\\.claude", h.replace('/', "\\"))))
    };
    let wsl_slug = wsl_claude.as_ref().map(|h| {
        let projects = PathBuf::from(h).join("projects");
        let (plain, strict) = (f.wsl_root.replace('/', "-"), dashed(&f.wsl_root));
        [strict.clone(), plain].into_iter().find(|n| projects.join(n).is_dir()).unwrap_or(strict)
    });
    let wsl_has = match (&wsl_claude, &wsl_slug) { (Some(h), Some(n)) => PathBuf::from(h).join("projects").join(n).is_dir(), _ => false };
    let can_wsl = wsl_claude.is_some() && !f.wsl_root.is_empty();
    let can_win = !f.inside && win_home.is_some();
    let native = if f.inside { false } else {
        match runs {
            Some("windows") => true,
            Some("wsl") => false,
            _ if win_has != wsl_has => win_has,
            _ if !can_wsl => true,
            _ if !can_win => false,
            _ => match list.first() { Some(p) => is_native(p), None => !wsl_claude.as_ref().map(|h| Path::new(h).is_dir()).unwrap_or(false) },
        }
    };
    if native && !can_win { return Err("this folder is on WSL's own disk: only an agent inside WSL can work in it".into()); }
    if !native && !can_wsl { return Err("WSL was not found (or its home folder could not be read): add the project as one that runs on Windows".into()); }

    let name = root_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let (claude_home, memory, entry);
    if native {
        claude_home = win_home.unwrap_or_default();
        memory = claude_home.join("projects").join(&win_slug).join("memory");
        entry = json!({ "name": name, "root": f.root, "runs": "windows", "wslRoot": "", "distro": "", "claudeHome": claude_home.to_string_lossy(), "memoryDir": memory.to_string_lossy(), "kit": "" });
    } else {
        claude_home = PathBuf::from(wsl_claude.unwrap_or_default());
        memory = claude_home.join("projects").join(wsl_slug.unwrap_or_default()).join("memory");
        let kit = same.and_then(|p| p["kit"].as_str()).unwrap_or("").to_string();
        entry = json!({ "name": name, "root": f.root, "runs": "wsl", "wslRoot": f.wsl_root, "distro": distro, "claudeHome": claude_home.to_string_lossy(), "memoryDir": memory.to_string_lossy(), "kit": kit });
    }
    let dot = dot_of(&root_path);
    let instructions = instructions_of(&root_path, &dot);
    let count = |d: &Path| md_files(d).len();
    Ok(json!({
        "entry": entry,
        "choices": { "wsl": can_wsl, "windows": can_win, "distro": distro },
        "found": {
            "dir": rel(&dot), "claude": dot.is_dir(), "kit": dot.join("ctx.json").exists(),
            "instructions": instructions.exists(), "instructionsName": name_of(&instructions),
            "rules": count(&dot.join("rules")), "log": md_files(&dot.join("log")).iter().filter(|f| date_in(&name_of(f)).is_some()).count(),
            "skills": fs::read_dir(dot.join("skills")).map(|r| r.filter_map(|e| e.ok()).filter(|e| e.path().join("SKILL.md").exists()).count()).unwrap_or(0),
            "memory": memory.is_dir(), "memories": md_files(&memory).iter().filter(|f| name_of(f) != "MEMORY.md").count(),
            "worked": if native { win_has } else { wsl_has },
        }
    }))
}
/// What this machine has, for the welcome screen: WSL distros, and whether Claude Code has a folder on the Windows side.
#[tauri::command(async)]
fn machine() -> Value {
    json!({ "distros": wsl_distros(), "windowsClaude": windows_claude_home().map(|h| h.is_dir()).unwrap_or(false), "windowsClaudeExe": find_claude().is_some() })
}
/// What adding this folder would give: the entry, and what is in the folder.
#[tauri::command(async)]
fn inspect_project(folder: String, runs: Option<String>) -> Result<Value, String> {
    let mut v = derive(&folder, runs.as_deref())?;
    let root = v["entry"]["root"].as_str().unwrap_or("").to_lowercase();
    v["already"] = json!(config()["projects"].as_array().map(|a| a.iter().any(|p| p["root"].as_str().unwrap_or("").to_lowercase() == root)).unwrap_or(false));
    Ok(v)
}
/// Add a folder to the list of projects and open it.
#[tauri::command]
fn add_project(folder: String, name: String, runs: Option<String>, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<usize, String> {
    let mut entry = derive(&folder, runs.as_deref())?["entry"].clone();
    let n = one_line(&name);
    if !n.is_empty() { entry["name"] = json!(n); }
    let mut c = config();
    let list = c["projects"].as_array_mut().ok_or("maestro.config.json has no projects list")?;
    let (root, shown) = (entry["root"].as_str().unwrap_or("").to_lowercase(), entry["name"].as_str().unwrap_or("").to_lowercase());
    if list.iter().any(|p| p["root"].as_str().unwrap_or("").to_lowercase() == root) { return Err("that folder is already in the list".into()); }
    if list.iter().any(|p| p["name"].as_str().unwrap_or("").to_lowercase() == shown) { return Err("another project already has that name: give this one a different name".into()); }
    list.push(entry);
    let at = list.len() - 1;
    if c["hotkey"].is_null() { c["hotkey"] = json!("Ctrl+Shift+Space"); }
    save_config(&c)?;
    cur.0.lock().map_err(|e| e.to_string())?.insert(window.label().to_string(), at);
    Ok(at)
}

// ---- the starter harness: for a project that has none. Only files that are not there are written; nothing is ever replaced. ----
const STARTER: [(&str, &str); 6] = [
    ("{{instructions}}", include_str!("../../starter/INSTRUCTIONS.md")),
    ("{{dir}}/rules/harness.md", include_str!("../../starter/rules/harness.md")),
    ("{{dir}}/skills/log/SKILL.md", include_str!("../../starter/skills/log/SKILL.md")),
    ("{{dir}}/skills/rule/SKILL.md", include_str!("../../starter/skills/rule/SKILL.md")),
    ("{{dir}}/skills/recall/SKILL.md", include_str!("../../starter/skills/recall/SKILL.md")),
    ("{{dir}}/log/{{date}}-harness-set-up.md", include_str!("../../starter/log/harness-set-up.md")),
];
/// The layout a project already leans to: "agents" (AGENTS.md + .agents/) or "claude" (CLAUDE.md + .claude/).
fn layout_of(root: &Path) -> &'static str {
    let dot = dot_of(root);
    if rel(&dot) == ".agents" || (root.join("AGENTS.md").exists() && !root.join("CLAUDE.md").exists() && !used_dir(&root.join(".claude"))) { "agents" } else { "claude" }
}
fn used_dir(d: &Path) -> bool {
    ["rules", "skills", "log"].iter().any(|k| d.join(k).is_dir())
}
/// The starter's files for one project: (path inside the project, text).
fn starter_files(root: &Path, name: &str, layout: &str) -> Result<Vec<(String, String)>, String> {
    let (dir, instructions) = match layout { "claude" => (".claude", "CLAUDE.md"), "agents" => (".agents", "AGENTS.md"), _ => return Err("a layout is claude or agents".into()) };
    let day = today();
    let fill = |t: &str| t.replace("\r\n", "\n").replace("{{dir}}", dir).replace("{{instructions}}", instructions).replace("{{date}}", &day).replace("{{project}}", name);
    let mut out: Vec<(String, String)> = STARTER.iter().map(|(path, text)| (fill(path), fill(text))).collect();
    // Claude Code reads CLAUDE.md: in the agents layout a one-line CLAUDE.md brings AGENTS.md in
    if layout == "agents" && !root.join("CLAUDE.md").exists() { out.push(("CLAUDE.md".into(), "@AGENTS.md\n".into())); }
    Ok(out)
}
fn at(root: &Path, path: &str) -> PathBuf {
    path.split('/').fold(root.to_path_buf(), |d, part| d.join(part))
}
fn write_starter(root: &Path, name: &str, layout: &str) -> Result<Value, String> {
    let (mut written, mut kept) = (vec![], vec![]);
    for (path, text) in starter_files(root, name, layout)? {
        let file = at(root, &path);
        // a first log entry is only for a log that has none
        let has_log = path.contains("/log/") && file.parent().map(|d| !md_files(d).is_empty()).unwrap_or(false);
        if file.exists() || has_log { kept.push(path); continue; }
        if let Some(d) = file.parent() { fs::create_dir_all(d).map_err(|e| e.to_string())?; }
        fs::write(&file, text).map_err(|e| format!("{path}: {e}"))?;
        written.push(path);
    }
    Ok(json!({ "written": written, "kept": kept }))
}
/// What setting up the starter harness in the open project would write, and what is already there and stays.
#[tauri::command]
fn starter_plan(layout: Option<String>, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    let layout = layout.unwrap_or_else(|| layout_of(&p.root).to_string());
    let files: Vec<Value> = starter_files(&p.root, &p.name, &layout)?.iter().map(|(path, text)| {
        let file = at(&p.root, path);
        let has_log = path.contains("/log/") && file.parent().map(|d| !md_files(d).is_empty()).unwrap_or(false);
        json!({ "path": path, "exists": file.exists() || has_log, "text": text })
    }).collect();
    Ok(json!({ "layout": layout, "root": p.root.to_string_lossy(), "files": files }))
}
#[tauri::command]
fn starter_apply(layout: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    write_starter(&p.root, &p.name, &layout)
}
/// Mark a project as a favourite (favourites come first wherever projects are listed), or take the mark off.
#[tauri::command]
fn set_favorite(i: usize, on: bool) -> Result<(), String> {
    let mut c = config();
    let list = c["projects"].as_array_mut().ok_or("maestro.config.json has no projects list")?;
    let p = list.get_mut(i).ok_or("no such project")?;
    p["favorite"] = json!(on);
    save_config(&c)
}
/// Take a project off the list. Its files are not touched.
#[tauri::command]
fn remove_project(i: usize, cur: State<Cur>, _window: tauri::WebviewWindow) -> Result<(), String> {
    let mut c = config();
    let list = c["projects"].as_array_mut().ok_or("maestro.config.json has no projects list")?;
    if i >= list.len() { return Err("no such project".into()); }
    if list.len() == 1 { return Err("the last project stays: add another one first".into()); }
    list.remove(i);
    save_config(&c)?;
    // every window's place in the list moves with it
    for at in cur.0.lock().map_err(|e| e.to_string())?.values_mut() { if *at == i { *at = 0; } else if *at > i { *at -= 1; } }
    Ok(())
}
/// The system's own folder picker.
#[tauri::command(async)]
fn pick_folder(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog().file().set_parent(&window).set_title("Pick the project's folder").blocking_pick_folder().map(|p| p.to_string())
}

// ---- git: read only. The app shows what changed and what was committed; it never stages, commits or pushes. ----
/// A path the kit wrote the WSL way (/mnt/d/x) as Windows wants it (D:\\x).
fn win_path(pth: &str) -> PathBuf {
    let b = pth.as_bytes();
    if pth.starts_with("/mnt/") && b.len() > 6 && b[6] == b'/' {
        return PathBuf::from(format!("{}:\\{}", pth[5..6].to_uppercase(), pth[7..].replace('/', "\\")));
    }
    PathBuf::from(pth)
}
/// The repos of a project: the ones its deploy ledger names (prod-heads.json), else the project folder itself.
fn repo_list(p: &Project) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = vec![];
    let heads: Value = serde_json::from_str(&rd(&p.dot.join("state").join("prod-heads.json"))).unwrap_or(json!({}));
    if let Some(repos) = heads["repos"].as_object() {
        for (name, r) in repos {
            let path = win_path(r["path"].as_str().unwrap_or(""));
            if path.is_dir() { out.push((name.clone(), path)); }
        }
    }
    if !out.iter().any(|(_, d)| *d == p.root) && (p.root.join(".git").exists() || out.is_empty()) {
        out.insert(0, (p.name.clone(), p.root.clone()));
    }
    // the project's own repo first
    out.sort_by_key(|(_, d)| if *d == p.root { 0 } else { 1 });
    out
}
fn repo_dir(p: &Project, repo: &str) -> Result<PathBuf, String> {
    repo_list(p).into_iter().find(|(n, _)| n == repo).map(|(_, d)| d).ok_or_else(|| format!("{repo} is not one of this project's repos"))
}
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut c = std::process::Command::new("git");
    c.arg("-C").arg(dir).args(["-c", "core.quotepath=false", "-c", "color.ui=false"]).args(args);
    let out = quiet(c).output().map_err(|e| format!("git could not be started ({e}): is Git for Windows installed?"))?;
    if !out.status.success() { return Err(String::from_utf8_lossy(&out.stderr).trim().to_string()); }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}
fn hexish(s: &str) -> Result<&str, String> {
    if s.len() >= 7 && s.len() <= 40 && s.chars().all(|c| c.is_ascii_hexdigit()) { Ok(s) } else { Err("that is not a commit id".into()) }
}
fn inside(path: &str) -> Result<&str, String> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains(':') || path.split(['/', '\\']).any(|seg| seg == "..") { Err("that path is not inside the repo".into()) } else { Ok(path) }
}
fn cap(text: String, max: usize) -> Value {
    if text.len() <= max { return json!({ "text": text, "cut": false }); }
    let mut at = max;
    while !text.is_char_boundary(at) { at -= 1; }
    json!({ "text": text[..at], "cut": true })
}

#[tauri::command]
fn git_repos(cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    Ok(json!(repo_list(&p).iter().map(|(n, d)| json!({ "name": n, "path": d.to_string_lossy() })).collect::<Vec<_>>()))
}
/// The branch, how far it is from its upstream, and every changed file with its added and removed lines.
#[tauri::command(async)]
fn git_status(repo: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    let dir = repo_dir(&p, &repo)?;
    let started = std::time::Instant::now();
    let out = git(&dir, &["status", "--porcelain=v2", "--branch", "--untracked-files=normal"])?;
    let (mut branch, mut upstream, mut ahead, mut behind) = (String::new(), String::new(), 0i64, 0i64);
    let mut files: Vec<Value> = vec![];
    for ln in out.lines() {
        if let Some(v) = ln.strip_prefix("# branch.head ") { branch = v.to_string(); }
        else if let Some(v) = ln.strip_prefix("# branch.upstream ") { upstream = v.to_string(); }
        else if let Some(v) = ln.strip_prefix("# branch.ab ") { let mut it = v.split(' '); ahead = it.next().unwrap_or("+0").trim_start_matches('+').parse().unwrap_or(0); behind = it.next().unwrap_or("-0").trim_start_matches('-').parse().unwrap_or(0); }
        else if ln.starts_with("1 ") || ln.starts_with("2 ") {
            let renamed = ln.starts_with("2 ");
            let parts: Vec<&str> = ln.splitn(if renamed { 10 } else { 9 }, ' ').collect();
            if parts.len() < 9 { continue; }
            let xy = parts[1].as_bytes();
            let path = parts[parts.len() - 1].split('\t').next().unwrap_or("").to_string();
            files.push(json!({ "path": path, "staged": (xy[0] as char).to_string(), "work": (xy[1] as char).to_string() }));
        } else if let Some(v) = ln.strip_prefix("? ") { files.push(json!({ "path": v, "staged": ".", "work": "?" })); }
        else if let Some(v) = ln.strip_prefix("u ") { files.push(json!({ "path": v.rsplit(' ').next().unwrap_or(""), "staged": "U", "work": "U" })); }
    }
    let total = files.len();
    // line counts, both sides; skipped when the list is huge (a vendored folder checked in and edited)
    if total <= 400 {
        let mut counts: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
        for args in [&["diff", "--numstat"][..], &["diff", "--cached", "--numstat"][..]] {
            for ln in git(&dir, args).unwrap_or_default().lines() {
                let mut it = ln.split('\t');
                let (a, d, f) = (it.next().unwrap_or("0").parse().unwrap_or(0), it.next().unwrap_or("0").parse().unwrap_or(0), it.next().unwrap_or(""));
                let e = counts.entry(f.to_string()).or_insert((0, 0));
                e.0 += a; e.1 += d;
            }
        }
        for f in files.iter_mut() { if let Some((a, d)) = counts.get(f["path"].as_str().unwrap_or("")) { f["add"] = json!(a); f["del"] = json!(d); } }
    }
    files.truncate(600);
    Ok(json!({ "repo": repo, "branch": branch, "upstream": upstream, "ahead": ahead, "behind": behind, "files": files, "total": total, "ms": started.elapsed().as_millis() as u64 }))
}
#[tauri::command(async)]
fn git_log(repo: String, skip: usize, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    let dir = repo_dir(&p, &repo)?;
    let out = git(&dir, &["log", "-n", "80", &format!("--skip={skip}"), "--date=format:%Y-%m-%d %H:%M", "--pretty=format:%H%x1f%h%x1f%an%x1f%ad%x1f%ar%x1f%s%x1f%D%x1e"])?;
    let unpushed: std::collections::HashSet<String> = git(&dir, &["log", "--pretty=format:%H", "@{upstream}..HEAD"]).unwrap_or_default().lines().map(|l| l.to_string()).collect();
    Ok(json!(out.split('\u{1e}').filter_map(|rec| {
        let f: Vec<&str> = rec.trim_start_matches('\n').split('\u{1f}').collect();
        if f.len() < 7 { return None; }
        Some(json!({ "sha": f[0], "short": f[1], "who": f[2], "at": f[3], "ago": f[4], "subject": f[5], "refs": f[6], "unpushed": unpushed.contains(f[0]) }))
    }).collect::<Vec<_>>()))
}
/// One commit: its message and the files it touched, with added and removed lines.
#[tauri::command(async)]
fn git_commit(repo: String, sha: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    let dir = repo_dir(&p, &repo)?;
    let out = git(&dir, &["show", "--numstat", "--date=format:%Y-%m-%d %H:%M", "--pretty=format:%H%x1f%an%x1f%ae%x1f%ad%x1f%B%x1e", hexish(&sha)?])?;
    let (head, stat) = out.split_once('\u{1e}').unwrap_or((&out, ""));
    let f: Vec<&str> = head.split('\u{1f}').collect();
    let files: Vec<Value> = stat.lines().filter(|l| !l.trim().is_empty()).map(|l| { let mut it = l.split('\t'); json!({ "add": it.next().unwrap_or("-"), "del": it.next().unwrap_or("-"), "path": it.next().unwrap_or("") }) }).collect();
    Ok(json!({ "sha": f.first().unwrap_or(&""), "who": f.get(1).unwrap_or(&""), "at": f.get(3).unwrap_or(&""), "message": f.get(4).unwrap_or(&"").trim(), "files": files }))
}
/// A diff: one file as it stands against the last commit (`work`), what is staged (`staged`), or one file of a commit (`commit`).
#[tauri::command(async)]
fn git_diff(repo: String, kind: String, path: String, sha: Option<String>, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    let dir = repo_dir(&p, &repo)?;
    let file = inside(&path)?;
    let text = match kind.as_str() {
        "work" => git(&dir, &["diff", "HEAD", "--", file]).or_else(|_| git(&dir, &["diff", "--", file]))?,
        "staged" => git(&dir, &["diff", "--cached", "--", file])?,
        "commit" => git(&dir, &["show", "--pretty=format:", hexish(sha.as_deref().unwrap_or(""))?, "--", file])?,
        "new" => { let raw = fs::read(dir.join(file)).map_err(|e| e.to_string())?; if raw.iter().take(8000).any(|b| *b == 0) { "Binary file".to_string() } else { String::from_utf8_lossy(&raw).lines().map(|l| format!("+{l}")).collect::<Vec<_>>().join("\n") } }
        _ => return Err("unknown kind of diff".into()),
    };
    Ok(cap(text, 400_000))
}
/// The tracked files and folders directly inside one folder of the repo.
#[tauri::command(async)]
fn git_tree(repo: String, dir_path: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    let dir = repo_dir(&p, &repo)?;
    let sub = if dir_path.is_empty() { String::new() } else { format!("{}/", inside(dir_path.trim_end_matches('/'))?) };
    let out = if sub.is_empty() { git(&dir, &["ls-tree", "HEAD"])? } else { git(&dir, &["ls-tree", "HEAD", &sub])? };
    let mut items: Vec<Value> = out.lines().filter_map(|l| { let (meta, path) = l.split_once('\t')?; let kind = meta.split(' ').nth(1)?; Some(json!({ "path": path, "name": path.rsplit('/').next().unwrap_or(path), "dir": kind == "tree" })) }).collect();
    items.sort_by(|a, b| b["dir"].as_bool().cmp(&a["dir"].as_bool()).then(a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase())));
    Ok(json!(items))
}
/// A file as it is on disk now.
#[tauri::command(async)]
fn git_file(repo: String, path: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    let raw = fs::read(repo_dir(&p, &repo)?.join(inside(&path)?)).map_err(|e| e.to_string())?;
    if raw.iter().take(8000).any(|b| *b == 0) { return Ok(json!({ "text": "", "cut": false, "binary": true, "bytes": raw.len() })); }
    Ok(cap(String::from_utf8_lossy(&raw).to_string(), 300_000))
}

/// The projects of maestro.config.json and which one is open.
#[tauri::command]
fn projects(cur: State<Cur>, window: tauri::WebviewWindow) -> Value {
    let list = config()["projects"].as_array().cloned().unwrap_or_default();
    let names: Vec<Value> = list.iter().map(|p| p["name"].clone()).collect();
    let roots: Vec<Value> = list.iter().map(|p| p["root"].clone()).collect();
    let favs: Vec<bool> = list.iter().map(|p| p["favorite"].as_bool().unwrap_or(false)).collect();
    let native: Vec<bool> = list.iter().map(|p| p["runs"] == "windows" || p["wslRoot"].as_str().unwrap_or("").is_empty()).collect();
    json!({ "names": names, "roots": roots, "favs": favs, "native": native, "current": cur_of(&cur, &window) })
}
#[tauri::command]
fn set_project(i: usize, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    let n = config()["projects"].as_array().map(|a| a.len()).unwrap_or(0);
    if i >= n { return Err("no such project".into()); }
    cur.0.lock().map_err(|e| e.to_string())?.insert(window.label().to_string(), i);
    Ok(())
}

const RUN_KEY: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// Does Windows start Maestro at sign-in?
#[tauri::command]
fn autostart_get() -> bool {
    let mut c = std::process::Command::new("reg.exe");
    c.args(["query", RUN_KEY, "/v", "Maestro"]);
    quiet(c).output().map(|o| o.status.success()).unwrap_or(false)
}
#[tauri::command]
fn autostart_set(on: bool) -> Result<bool, String> {
    let mut c = std::process::Command::new("reg.exe");
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        c.args(["add", RUN_KEY, "/v", "Maestro", "/t", "REG_SZ", "/d", &format!("\"{}\" --hidden", exe.display()), "/f"]);
    } else {
        c.args(["delete", RUN_KEY, "/v", "Maestro", "/f"]);
    }
    let out = quiet(c).output().map_err(|e| e.to_string())?;
    if !out.status.success() && on { return Err(String::from_utf8_lossy(&out.stderr).trim().to_string()); }
    Ok(autostart_get())
}

// ---- Claude Code sessions: the real program in a pseudo-terminal, shown by the page. Maestro adds nothing between you and it. ----
struct Term {
    writer: Box<dyn std::io::Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}
#[derive(Default)]
struct Terms(Mutex<std::collections::HashMap<String, Term>>);

/// The scripts that keep sessions alive inside WSL, built into the program.
const TERM_SH: &str = include_str!("../../wsl/term.sh");
const TMUX_CONF: &str = include_str!("../../wsl/tmux.conf");
/// Put those scripts where the distro can run them (~/.local/share/maestro, written through the \\wsl.localhost share whenever
/// they differ from this build's) and return term.sh's path as WSL sees it.
fn launcher(p: &Project) -> Result<String, String> {
    let home = p.claude_home.parent().ok_or("this project's claudeHome is not a folder")?;
    let linux = linux_of(&home.to_string_lossy()).ok_or("this project's claudeHome is not a folder inside WSL (\\\\wsl.localhost\\<distro>\\home\\<you>\\.claude)")?;
    let dir = home.join(".local").join("share").join("maestro");
    for (name, text) in [("term.sh", TERM_SH), ("tmux.conf", TMUX_CONF)] {
        let want = text.replace("\r\n", "\n");
        if rd(&dir.join(name)) != want {
            fs::create_dir_all(&dir).and_then(|_| fs::write(dir.join(name), want)).map_err(|e| format!("could not write {name} into WSL: {e}"))?;
        }
    }
    Ok(format!("{linux}/.local/share/maestro/term.sh"))
}
/// The project whose distro holds the sessions: this one, or the first of the list that runs inside WSL.
fn wsl_side(p: Project) -> Option<Project> {
    if !p.native { return Some(p); }
    let n = config()["projects"].as_array().map(|a| a.len()).unwrap_or(0);
    (0..n).filter_map(|i| project_at(i).ok()).find(|q| !q.native)
}
/// A session this app started: maestro-…, or reins-… from a build before the rename.
fn ours(n: &str) -> bool {
    (n.starts_with("maestro-") || n.starts_with("reins-")) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
/// Claude Code on Windows itself: claude.exe (started directly) or claude.cmd (started through cmd.exe), from PATH or ~\.local\bin.
fn find_claude() -> Option<(PathBuf, bool)> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(h) = std::env::var_os("USERPROFILE") { dirs.push(PathBuf::from(h).join(".local").join("bin")); }
    for (name, direct) in [("claude.exe", true), ("claude.cmd", false), ("claude.bat", false)] {
        if let Some(f) = dirs.iter().map(|d| d.join(name)).find(|f| f.is_file()) { return Some((f, direct)); }
    }
    None
}

/// Start a session in the open project and return its name.
///   kind "claude" — Claude Code; `mode` is "new", "continue" (the most recent conversation) or "resume" (a session id); `prompt` is an optional first message
///   kind "ubuntu" — a shell in WSL, in the project's folder          kind "powershell" — Windows PowerShell, in the project's folder
///   mode "attach" — a claude or ubuntu session that is still running (`name`)
/// In a project that runs inside WSL, Claude and Ubuntu sessions live in a tmux server there, so they outlive this window. In a
/// project that runs on Windows itself Claude Code is started directly and, like PowerShell, ends with the app.
/// Output arrives as `term` events, the end as `term-exit`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn term_open(id: String, kind: String, mode: String, proj: Option<usize>, name: Option<String>, session: Option<String>, prompt: Option<String>, label: Option<String>, cols: u16, rows: u16, app: tauri::AppHandle, terms: State<Terms>, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<String, String> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    let p = match proj { Some(i) => project_at(i)?, None => project(&cur, &window)? };
    let mut made = String::new();
    let mut cmd;
    // what Claude Code is started with, wherever it runs
    let mut claude_args: Vec<String> = vec![];
    if kind == "claude" && mode != "attach" {
        match mode.as_str() {
            "new" => {}
            "continue" => claude_args.push("--continue".into()),
            "resume" => {
                let sid = session.unwrap_or_default();
                if sid.len() < 8 || !sid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') { return Err("not a session id".into()); }
                claude_args.extend(["--resume".to_string(), sid]);
            }
            _ => return Err("unknown way to start".into()),
        }
        if let Some(text) = prompt.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
            if text.starts_with('-') { return Err("a first message cannot start with a dash".into()); }
            claude_args.push(text);
        }
    }
    if kind == "powershell" {
        cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        cmd.cwd(&p.root);
    } else if kind == "claude" && p.native && mode != "attach" {
        let (exe, direct) = find_claude().ok_or("Claude Code was not found on Windows (looked for claude.exe and claude.cmd on PATH and in %USERPROFILE%\\.local\\bin)")?;
        if direct {
            cmd = CommandBuilder::new(exe);
        } else {
            // a .cmd goes through cmd.exe, which reads & | < > ^ % and quotes as its own
            if claude_args.last().map(|t| t.contains(['&', '|', '<', '>', '^', '%', '"'])).unwrap_or(false) { return Err("this first message has signs the Windows command line would act on (& | < > ^ % \"): start the session empty and type it there".into()); }
            cmd = CommandBuilder::new("cmd.exe");
            cmd.args(["/c".to_string(), exe.to_string_lossy().to_string()]);
        }
        cmd.args(&claude_args);
        cmd.cwd(&p.root);
    } else if kind == "claude" || kind == "ubuntu" {
        // a session that is still running is reached through any project of that distro; a new one belongs to this project's folder
        let p = if mode == "attach" { wsl_side(p) } else if p.native { None } else { Some(p) }.ok_or("this project runs on Windows itself, so there is no WSL shell for it: open PowerShell here")?;
        if mode != "attach" && p.wsl_root.is_empty() { return Err("this project has no wslRoot in maestro.config.json, so a session cannot be opened in WSL".into()); }
        let launcher = launcher(&p)?;
        cmd = CommandBuilder::new("wsl.exe");
        // WSL turns the Windows PATH into Linux paths and complains, inside the session, about every entry on a drive that is not there:
        // give it only the system folders (the launcher sets the Linux PATH itself)
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        cmd.env("PATH", format!("{sys}\\System32;{sys}"));
        cmd.args(["-d", &p.distro, "--cd", &p.wsl_root, "--exec", "bash", &launcher]);
        if mode == "attach" {
            let n = name.unwrap_or_default();
            if !ours(&n) { return Err("not a session of this app".into()); }
            cmd.args(["--attach", &n]);
            made = n;
        } else {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let proj: String = p.name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
            made = format!("maestro-{proj}-{kind}-{stamp}");
            let shown: String = one_line(&label.unwrap_or_default()).chars().take(80).collect();
            cmd.args(["--new", &made, &kind, if shown.is_empty() { "session" } else { &shown }]);
            cmd.args(&claude_args);
        }
    } else {
        return Err("unknown kind of session".into());
    }
    let pair = native_pty_system().openpty(PtySize { rows: rows.max(8), cols: cols.max(20), pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
    let child = pair.slave.spawn_command(cmd).map_err(|e| format!("the session could not be started: {e}"))?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    terms.0.lock().map_err(|e| e.to_string())?.insert(id.clone(), Term { writer, master: pair.master, child });
    // On Windows the output stream does not end when the program does, so nothing would ever say "ended": watch the program itself.
    // When it has exited, letting go of its terminal closes the stream, and the reader below reports the end.
    {
        let (app, id) = (app.clone(), id.clone());
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(600));
            let terms = app.state::<Terms>();
            let Ok(mut map) = terms.0.lock() else { break };
            let gone = match map.get_mut(&id) { None => break, Some(t) => !matches!(t.child.try_wait(), Ok(None)) };
            if gone { map.remove(&id); break; }
        });
    }
    std::thread::spawn(move || {
        let mut buf = [0u8; 16384];
        let mut carry: Vec<u8> = vec![];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    carry.extend_from_slice(&buf[..n]);
                    // send whole characters only: a multi-byte one cut by the read waits for its other half
                    let good = match std::str::from_utf8(&carry) { Ok(_) => carry.len(), Err(e) => if e.error_len().is_some() { carry.len() } else { e.valid_up_to() } };
                    if good > 0 {
                        let text = String::from_utf8_lossy(&carry[..good]).to_string();
                        carry.drain(..good);
                        let _ = app.emit("term", json!({ "id": id, "data": text }));
                    }
                }
            }
        }
        let _ = app.emit("term-exit", json!({ "id": id }));
    });
    Ok(made)
}
/// The claude and ubuntu sessions still running in WSL, whoever started them and whether or not a window shows them.
#[tauri::command(async)]
fn term_list(cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let Some(p) = project(&cur, &window).ok().and_then(wsl_side) else { return Ok(json!([])) };
    let Ok(launcher) = launcher(&p) else { return Ok(json!([])) };
    let mut c = std::process::Command::new("wsl.exe");
    c.args(["-d", &p.distro, "--exec", "bash", &launcher, "--list"]);
    let out = quiet(c).output().map_err(|e| e.to_string())?;
    Ok(json!(String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 6 || !ours(f[0]) { return None; }
        Some(json!({ "name": f[0], "created": f[1].parse::<i64>().unwrap_or(0), "kind": if f[2].is_empty() { "ubuntu" } else { f[2] }, "label": f[3], "path": f[4], "attached": f[5] != "0" }))
    }).collect::<Vec<_>>()))
}
/// Whether the distro has what a session needs: tmux (sessions outlive the app) and claude.
#[tauri::command(async)]
fn term_check(cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    if p.native { return Ok(json!({ "native": true, "claude": find_claude().is_some() })); }
    let launcher = launcher(&p)?;
    let mut c = std::process::Command::new("wsl.exe");
    c.args(["-d", &p.distro, "--exec", "bash", &launcher, "--check"]);
    let out = quiet(c).output().map_err(|e| format!("could not start WSL: {e}"))?;
    let has: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).collect();
    Ok(json!({ "native": false, "distro": p.distro, "tmux": has.iter().any(|h| h == "tmux"), "claude": has.iter().any(|h| h == "claude") }))
}
#[tauri::command]
fn term_write(id: String, data: String, terms: State<Terms>) -> Result<(), String> {
    let mut map = terms.0.lock().map_err(|e| e.to_string())?;
    let t = map.get_mut(&id).ok_or("that session has ended")?;
    std::io::Write::write_all(&mut t.writer, data.as_bytes()).and_then(|_| std::io::Write::flush(&mut t.writer)).map_err(|e| e.to_string())
}
#[tauri::command]
fn term_resize(id: String, cols: u16, rows: u16, terms: State<Terms>) -> Result<(), String> {
    let map = terms.0.lock().map_err(|e| e.to_string())?;
    if let Some(t) = map.get(&id) {
        t.master.resize(portable_pty::PtySize { rows: rows.max(4), cols: cols.max(10), pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
    }
    Ok(())
}
/// Close a session's terminal here. `end` also stops the program: for a claude or ubuntu session (`name`) that means ending it in
/// WSL; without `end` it is only let go of and keeps running there.
#[tauri::command(async)]
fn term_close(id: String, name: Option<String>, end: bool, terms: State<Terms>, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    if let Some(mut t) = terms.0.lock().map_err(|e| e.to_string())?.remove(&id) {
        let _ = t.child.kill();
    }
    if let (true, Some(n)) = (end, name.filter(|n| ours(n))) {
        if let Some(p) = project(&cur, &window).ok().and_then(wsl_side) {
            let mut c = std::process::Command::new("wsl.exe");
            c.args(["-d", &p.distro, "--exec", "bash", &launcher(&p)?, "--kill", &n]);
            let _ = quiet(c).output();
        }
    }
    Ok(())
}

/// Rename a running claude or ubuntu session (the name is kept with the session, so it is still there after a restart).
#[tauri::command(async)]
fn term_label(name: String, label: String, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    if !ours(&name) { return Ok(()); }
    let Some(p) = project(&cur, &window).ok().and_then(wsl_side) else { return Ok(()) };
    let shown: String = one_line(&label).chars().take(80).collect();
    let mut c = std::process::Command::new("wsl.exe");
    c.args(["-d", &p.distro, "--exec", "bash", &launcher(&p)?, "--label", &name, &shown]);
    let _ = quiet(c).output();
    Ok(())
}
/// Give a running claude or ubuntu session a window of its own. The new window attaches to the same session, so nothing is moved
/// or restarted; closing that window only lets go of it.
// async: built from a blocking command, a second window on Windows opens blank and the app stops answering
#[tauri::command]
async fn pop_out(name: String, kind: String, label: String, app: tauri::AppHandle) -> Result<(), String> {
    if !ours(&name) { return Err("only a session that runs inside WSL can have its own window".into()); }
    let n = app.webview_windows().keys().filter(|k| k.starts_with("pop-")).count();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let boot = format!("window.__POP = {};", json!({ "name": name, "kind": kind, "label": label }));
    tauri::WebviewWindowBuilder::new(&app, format!("pop-{stamp}"), tauri::WebviewUrl::App("index.html".into()))
        .title(format!("{label} · Maestro"))
        .inner_size(1040.0, 680.0)
        .min_inner_size(420.0, 260.0)
        .position(120.0 + 36.0 * n as f64, 90.0 + 36.0 * n as f64)
        .resizable(true)
        .initialization_script(&boot)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
/// Give one of the app's views (Git, Memory, the Console…) a window of its own, showing the project the asking window shows.
/// It is an ordinary window: it can be resized, put on another screen, and covered by other windows.
#[tauri::command]
async fn pop_view(tab: String, title: String, app: tauri::AppHandle, window: tauri::WebviewWindow, cur: State<'_, Cur>) -> Result<(), String> {
    if tab.is_empty() || !tab.chars().all(|c| c.is_ascii_lowercase()) { return Err("not a view".into()); }
    let proj = cur_of(&cur, &window);
    let n = app.webview_windows().keys().filter(|k| k.starts_with("view-") || k.starts_with("pop-")).count();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let label = format!("view-{stamp}");
    cur.0.lock().map_err(|e| e.to_string())?.insert(label.clone(), proj);
    let boot = format!("window.__VIEW = {};", json!({ "tab": tab }));
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::App("index.html".into()))
        .title(format!("{} · Maestro", one_line(&title)))
        .inner_size(1180.0, 800.0)
        .min_inner_size(720.0, 420.0)
        .position(140.0 + 36.0 * n as f64, 100.0 + 36.0 * n as f64)
        .resizable(true)
        .initialization_script(&boot)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
/// Keep the window above every other one, or let it be covered like any window.
#[tauri::command]
fn set_on_top(on: bool, window: tauri::WebviewWindow) {
    let _ = window.set_always_on_top(on);
}
/// Ask for the user's eye: the taskbar button flashes until the window is looked at.
#[tauri::command]
fn attention(window: tauri::WebviewWindow) {
    let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
}
#[tauri::command]
fn toggle_max(window: tauri::WebviewWindow) {
    if window.is_maximized().unwrap_or(false) { let _ = window.unmaximize(); } else { let _ = window.maximize(); }
}

/// Put text on the Windows clipboard (used when the page's own clipboard call is refused).
#[tauri::command]
fn clip_set(text: String) -> Result<(), String> {
    use std::io::Write;
    let mut c = std::process::Command::new("clip.exe");
    c.stdin(std::process::Stdio::piped());
    let mut child = quiet(c).spawn().map_err(|e| e.to_string())?;
    // clip.exe reads UTF-16 when the text starts with its byte-order mark; anything else is taken as the console's code page
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
    for u in text.encode_utf16() { bytes.extend_from_slice(&u.to_le_bytes()); }
    child.stdin.take().ok_or("no pipe to clip.exe")?.write_all(&bytes).map_err(|e| e.to_string())?;
    child.wait().map(|_| ()).map_err(|e| e.to_string())
}
/// An error the page caught goes into maestro.log beside the config, so a blank or stuck screen leaves a trace.
#[tauri::command]
fn note(text: String) {
    if let Some(dir) = config_path().and_then(|f| f.parent().map(|d| d.to_path_buf())).or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf()))) {
        use std::io::Write;
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(dir.join("maestro.log")) {
            let line: String = text.replace(['\r', '\n'], " ").chars().take(600).collect();
            let _ = writeln!(f, "{} {}", today(), line);
        }
    }
}
/// Down to the taskbar; the key or a click on the icon brings it back.
#[tauri::command]
fn minimize(window: tauri::WebviewWindow) {
    let _ = window.minimize();
}
#[tauri::command]
fn hide(window: tauri::WebviewWindow) {
    let _ = window.hide();
}
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

fn reveal(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        let _ = w.emit("shown", ());
    }
}
fn toggle(app: &tauri::AppHandle) {
    match app.get_webview_window("main") {
        Some(w) if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) => {
            let _ = w.emit("hidden", ()); // the page stops its live git checks while nobody is looking
            let _ = w.hide();
        }
        _ => reveal(app),
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| reveal(app))) // a second launch shows the one already running
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle(app);
                    }
                })
                .build(),
        )
        .manage(Cur(Mutex::new(std::collections::HashMap::new())))
        .manage(Terms::default())
        .setup(|app| {
            // the key that shows and hides the window: "hotkey" in maestro.config.json (Ctrl+Shift+Space when it is absent or unreadable).
            // Another program may hold the key already: the app then still starts, and says so in maestro.log.
            let named = config()["hotkey"].as_str().unwrap_or("").to_string();
            let (key, shown) = match named.parse::<Shortcut>() {
                Ok(k) => (k, named),
                Err(_) => (Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space), "Ctrl+Shift+Space".to_string()),
            };
            if let Err(e) = app.global_shortcut().register(key) {
                note(format!("the key {shown} could not be registered ({e}): use the tray icon, or name another key as \"hotkey\" in maestro.config.json"));
            }

            let show = MenuItem::with_id(app, "show", format!("Show Maestro    {shown}"), true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new().tooltip("Maestro").menu(&menu).show_menu_on_left_click(false);
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, event| match event.id.as_ref() {
                "show" => reveal(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                    toggle(tray.app_handle());
                }
            })
            .build(app)?;

            // started by Windows at sign-in: stay in the tray until the key is pressed
            if !std::env::args().any(|a| a == "--hidden") {
                reveal(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load, read_log, read_audit, read_memory, save_memory, new_memory, read_skill, save_skill, new_skill, save_law, move_law, retire_law, ledger, mark_live, close_rows, audit, end_session, git_repos, git_status, git_log, git_commit, git_diff, git_tree, git_file, projects, set_project, inspect_project, add_project, machine, starter_plan, starter_apply, term_check, remove_project, set_favorite, pick_folder, autostart_get, autostart_set, term_open, term_list, term_write, term_resize, term_close, term_label, pop_out, pop_view, set_on_top, attention, toggle_max, note, clip_set, minimize, hide, quit
        ])
        .run(tauri::generate_context!())
        .expect("Maestro could not start");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rules_round_trip_is_byte_exact() {
        let raw = "---\npaths:\n  - \"src/**\"\n---\n\n# Title\n\nIntro.\n\n## One\nSection intro.\n- **A law** with words\n  check: foo — bar on: src/**\n- second law\n\n## Two\n- third\n";
        let d = parse_rules(raw);
        assert_eq!(serialize_rules(&d), raw);
        assert_eq!(d.secs.iter().map(|s| s.laws.len()).collect::<Vec<_>>(), vec![2, 1]);
        assert!(d.secs[0].laws[0].contains("check: foo"));
        assert_eq!(globs_of(&d.fm), vec!["src/**".to_string()]);
        let crlf = raw.replace('\n', "\r\n");
        assert_eq!(serialize_rules(&parse_rules(&crlf)), crlf);
    }
    #[test]
    fn a_rule_moves_and_the_blank_lines_stay_with_their_section() {
        let raw = "## One\n- a\n- b\n\n## Two\n- c\n";
        let mut d = parse_rules(raw);
        let body = remove_law(&mut d.secs[0], 1); // the last rule of One, the one carrying the blank line
        assert_eq!(body, "- b");
        place(&mut d, Some(1), &None, &body).unwrap();
        assert_eq!(serialize_rules(&d), "## One\n- a\n\n## Two\n- c\n- b\n");
        let only = remove_law(&mut d.secs[0], 0); // One is left empty and keeps its blank line
        assert_eq!(serialize_rules(&d), "## One\n\n## Two\n- c\n- b\n");
        place(&mut d, None, &Some("New".into()), &only).unwrap();
        assert_eq!(serialize_rules(&d), "## One\n\n## Two\n- c\n- b\n\n## New\n- a\n");
        assert!(checked(&mut d, 1, 0, "- c").is_ok());
        assert!(checked(&mut d, 1, 0, "- changed").is_err());
    }
    #[test]
    fn names_become_slugs_and_headers_split() {
        assert_eq!(slug("Release Checklist").unwrap(), "release-checklist");
        assert!(slug("../x").is_err());
        assert!(slug("").is_err());
        assert_eq!(split_front("---\nname: a\ndescription: b\n---\n\nBody\n"), ("name: a\ndescription: b", "Body\n"));
        assert_eq!(split_front("no header"), ("", "no header"));
        assert_eq!(one_line("a\nb "), "a b");
    }
    #[test]
    fn a_date_is_read_from_a_file_name() {
        assert_eq!(date_in("dependency-audit-2026-10-18.md"), Some("2026-10-18".into()));
        assert_eq!(date_in("AUDIT-2026-06-10.md"), Some("2026-06-10".into()));
        assert_eq!(date_in("security-audit-2026-09"), None);
        assert_eq!(date_in("README.txt"), None);
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(day_of(1_795_132_800), "2026-11-20");
    }
    /// A folder typed four ways is read as the same place.
    #[test]
    fn a_folder_is_read_however_it_is_typed() {
        let a = read_folder("D:\\projects\\my-site\\", "Ubuntu").unwrap();
        let b = read_folder("/mnt/d/projects/my-site", "Ubuntu").unwrap();
        assert_eq!((a.root.as_str(), a.wsl_root.as_str(), a.inside), ("D:\\projects\\my-site", "/mnt/d/projects/my-site", false));
        assert_eq!((b.root.as_str(), b.wsl_root.as_str(), b.inside), (a.root.as_str(), a.wsl_root.as_str(), false));
        let c = read_folder("/home/me/my-site", "Debian").unwrap();
        assert_eq!((c.root.as_str(), c.wsl_root.as_str(), c.inside), ("\\\\wsl.localhost\\Debian\\home\\me\\my-site", "/home/me/my-site", true));
        let d = read_folder("\\\\wsl.localhost\\Arch\\home\\me\\my-site", "Ubuntu").unwrap();
        assert_eq!((d.wsl_root.as_str(), d.distro.as_deref(), d.inside), ("/home/me/my-site", Some("Arch"), true));
        assert!(read_folder("my-site", "Ubuntu").is_err());
        assert!(read_folder("/home/me/x", "").is_err());
        assert_eq!(dashed("/mnt/d/projects/my-site"), "-mnt-d-projects-my-site");
        assert_eq!(dashed("D:\\projects\\my.site"), "D--projects-my-site");
        assert_eq!(linux_of("\\\\wsl.localhost\\Ubuntu\\home\\me").as_deref(), Some("/home/me"));
        assert_eq!(linux_of("C:\\Users\\me"), None);
        assert!(ours("maestro-my-site-claude-1") && ours("reins-x-ubuntu-2") && !ours("other-1") && !ours("maestro-a b"));
    }
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("maestro-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }
    /// The harness is read from .claude, or from .agents where that is where the project keeps it.
    #[test]
    fn the_harness_folder_is_found() {
        let d = scratch("dot");
        assert_eq!(rel(&dot_of(&d)), ".claude"); // nothing yet
        assert_eq!(name_of(&instructions_of(&d, &dot_of(&d))), "CLAUDE.md");
        fs::create_dir_all(d.join(".agents").join("skills")).unwrap();
        fs::write(d.join("AGENTS.md"), "# x\n").unwrap();
        assert_eq!(rel(&dot_of(&d)), ".agents");
        assert_eq!(name_of(&instructions_of(&d, &dot_of(&d))), "AGENTS.md");
        assert_eq!(layout_of(&d), "agents");
        fs::create_dir_all(d.join(".claude")).unwrap(); // a .claude that only holds settings does not take over
        assert_eq!(rel(&dot_of(&d)), ".agents");
        fs::create_dir_all(d.join(".claude").join("rules")).unwrap();
        assert_eq!(rel(&dot_of(&d)), ".claude");
        fs::write(d.join("CLAUDE.md"), "# x\n").unwrap();
        assert_eq!(name_of(&instructions_of(&d, &dot_of(&d))), "CLAUDE.md");
        let _ = fs::remove_dir_all(&d);
    }
    /// The starter writes what is missing, in either layout, and never replaces a file.
    #[test]
    fn the_starter_fills_in_only_what_is_missing() {
        let d = scratch("starter");
        fs::write(d.join("CLAUDE.md"), "mine\n").unwrap();
        let r = write_starter(&d, "my-site", "claude").unwrap();
        assert_eq!(r["kept"], json!(["CLAUDE.md"]));
        assert_eq!(r["written"].as_array().unwrap().len(), 5);
        assert_eq!(rd(&d.join("CLAUDE.md")), "mine\n");
        let rules = rd(&d.join(".claude").join("rules").join("harness.md"));
        assert!(!rules.contains("{{") && rules.contains(".claude/log/") && rules.contains("CLAUDE.md"));
        assert_eq!(serialize_rules(&parse_rules(&rules)), rules);
        assert!(parse_rules(&rules).secs.iter().map(|s| s.laws.len()).sum::<usize>() >= 6);
        let log = log_from_files(&d.join(".claude").join("log"));
        assert_eq!((log.len(), log[0]["title"].as_str()), (1, Some("Harness set up")));
        assert_eq!(write_starter(&d, "my-site", "claude").unwrap()["written"], json!([])); // a second run writes nothing
        let a = scratch("starter-agents");
        let r = write_starter(&a, "my-site", "agents").unwrap();
        assert_eq!(r["written"].as_array().unwrap().len(), 7);
        assert!(rd(&a.join("AGENTS.md")).starts_with("# my-site\n") && rd(&a.join("AGENTS.md")).contains(".agents/log/"));
        assert_eq!(rd(&a.join("CLAUDE.md")), "@AGENTS.md\n");
        assert!(a.join(".agents").join("skills").join("log").join("SKILL.md").exists() && !a.join(".claude").exists());
        assert_eq!(rel(&dot_of(&a)), ".agents");
        assert!(starter_files(&a, "x", "other").is_err());
        let _ = (fs::remove_dir_all(&d), fs::remove_dir_all(&a));
    }
    /// A conversation's first request is read from its transcript; what the program wrote itself is passed over.
    #[test]
    fn earlier_conversations_are_read_from_transcripts() {
        let d = scratch("past");
        fs::create_dir_all(d.join("memory")).unwrap();
        fs::write(d.join("0a1b2c3d-0000-4000-8000-000000000000.jsonl"), concat!(
            "{\"type\":\"summary\",\"summary\":\"x\"}\n",
            "{\"type\":\"user\",\"isMeta\":true,\"message\":{\"role\":\"user\",\"content\":\"Caveat: ignore\"}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/clear</command-name>\"}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"make the header sticky\"}]}}\n",
        )).unwrap();
        let p = Project { name: "x".into(), root: d.clone(), claude_home: d.clone(), memory: d.join("memory"), wsl_root: String::new(), distro: String::new(), kit: String::new(), audits: vec![], native: true, dot: d.join(".claude"), instructions: d.join("CLAUDE.md") };
        let past = past_conversations(&p);
        assert_eq!(past.len(), 1);
        assert_eq!((past[0]["id"].as_str(), past[0]["ask"].as_str()), (Some("0a1b2c3d-0000-4000-8000-000000000000"), Some("make the header sticky")));
        let _ = fs::remove_dir_all(&d);
    }
    /// What this machine answers (not run by default: it starts WSL).   cargo test -- --ignored --nocapture
    #[test]
    #[ignore]
    fn this_machine_is_read() {
        let distros = wsl_distros();
        println!("distros: {distros:?}");
        println!("windows claude: {:?} exe: {:?}", windows_claude_home(), find_claude());
        let d = scratch("machine");
        let folder = d.to_string_lossy().to_string();
        for runs in [None, Some("windows"), Some("wsl")] {
            match derive(&folder, runs) {
                Ok(v) => println!("{runs:?}: runs={} claudeHome={} wslRoot={} memoryDir ends {:?} choices={}", v["entry"]["runs"], v["entry"]["claudeHome"].as_str().map(|h| h.replace(|c: char| c.is_ascii_lowercase(), "x")).unwrap_or_default(), v["entry"]["wslRoot"].as_str().map(|h| h.len()).unwrap_or(0), v["entry"]["memoryDir"].as_str().map(|m| m.ends_with("\\memory")), v["choices"]),
                Err(e) => println!("{runs:?}: {e}"),
            }
        }
        if let Some(distro) = distros.first() {
            let home = wsl_home(distro).expect("the distro's home");
            let p = Project { name: "x".into(), root: d.clone(), claude_home: PathBuf::from(format!("\\\\wsl.localhost\\{distro}{}\\.claude", home.replace('/', "\\"))), memory: d.join("memory"), wsl_root: "/tmp".into(), distro: distro.clone(), kit: String::new(), audits: vec![], native: false, dot: d.join(".claude"), instructions: d.join("CLAUDE.md") };
            let l = launcher(&p).expect("the launcher is written");
            assert!(l.ends_with("/.local/share/maestro/term.sh"));
            let mut c = std::process::Command::new("wsl.exe");
            c.args(["-d", distro, "--exec", "bash", &l, "--check"]);
            let out = c.output().unwrap();
            println!("term.sh --check: {:?} (status {})", String::from_utf8_lossy(&out.stdout), out.status);
            let mut c = std::process::Command::new("wsl.exe");
            c.args(["-d", distro, "--exec", "bash", &l, "--list"]);
            println!("term.sh --list: {} sessions", String::from_utf8_lossy(&c.output().unwrap().stdout).lines().filter(|l| ours(l.split('\t').next().unwrap_or(""))).count());
        }
        let _ = fs::remove_dir_all(&d);
    }
    #[test]
    fn git_paths_and_ids_are_checked() {
        assert_eq!(win_path("/mnt/d/projects/x/y"), PathBuf::from("D:\\projects\\x\\y"));
        assert_eq!(win_path("C:\\a"), PathBuf::from("C:\\a"));
        assert!(inside("src/a.js").is_ok());
        assert!(inside("../a").is_err() && inside("a/../../b").is_err() && inside("/etc/x").is_err() && inside("C:\\x").is_err());
        assert!(hexish("0a1b2c3d").is_ok() && hexish("zzz").is_err() && hexish("--all").is_err());
        assert_eq!(cap("héllo".into(), 2)["text"], "h");
    }
    #[test]
    fn a_pasted_login_is_not_shown() {
        assert_eq!(redact("so for this Username: sample Pin: 0000 Password: Example1 it breaks"), "so for this it breaks");
        assert_eq!(redact("sample 42 Example#999 Hi"), "sample 42 Hi");
        assert_eq!(redact("order 45 <pasted_content id=\"b544\"> Username: sample ID:42 </pasted_content id=\"b544\"> i made"), "order 45 i made");
    }
    /// Every rules file of every configured project must come back byte for byte (skipped where the folder is not there).
    #[test]
    fn the_real_rules_files_round_trip() {
        let mut seen = 0;
        for p in config()["projects"].as_array().cloned().unwrap_or_default() {
            let dir = dot_of(Path::new(p["root"].as_str().unwrap_or(""))).join("rules");
            for f in md_files(&dir) {
                let raw = fs::read_to_string(&f).unwrap();
                assert_eq!(serialize_rules(&parse_rules(&raw)), raw, "{} changed in a round trip", f.display());
                seen += 1;
            }
        }
        println!("round-tripped {seen} rules files");
    }
    #[test]
    fn only_plain_md_names_are_opened() {
        assert!(safe("payments-core.md").is_ok());
        assert!(safe("../x.md").is_err());
        assert!(safe("a/b.md").is_err());
        assert!(safe("x.txt").is_err());
    }
}
