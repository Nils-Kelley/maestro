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

/// Which system this build is for: the page words a few things by it, and a release is fetched for it.
const PLATFORM: &str = if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" };
fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|h| !h.is_empty()).map(PathBuf::from)
}
/// Where the app keeps its own two files (maestro.config.json, maestro.log) when they are not beside the program:
/// %APPDATA%\Maestro on Windows, ~/Library/Application Support/Maestro on macOS, ~/.config/maestro on Linux.
fn app_data() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Maestro"))
    } else if cfg!(target_os = "macos") {
        home_dir().map(|h| h.join("Library").join("Application Support").join("Maestro"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| home_dir().map(|h| h.join(".config"))).map(|d| d.join("maestro"))
    }
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
    // the first project on this machine: the file does not exist yet. On Windows it goes beside the program when that folder
    // can be written; elsewhere (and on macOS and Linux always) into the app's own folder.
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("maestro.config.json")));
    if let (true, Some(f)) = (cfg!(windows), &beside) {
        if fs::write(f, &text).is_ok() { return Ok(()); }
    }
    let dir = app_data().ok_or("there is nowhere to keep maestro.config.json: the home folder of this user is not known")?;
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
/// Where a project's agent runs, which decides how a session is started and whether it outlives the app.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Host {
    /// Windows, with the agent inside a WSL distro: sessions live in tmux there, reached through wsl.exe
    Wsl,
    /// Windows, with the agent on Windows itself: started directly, ends with the app
    Direct,
    /// macOS or Linux: sessions live in tmux on this machine
    Local,
}
impl Project {
    fn host(&self) -> Host {
        if !cfg!(windows) { Host::Local } else if self.native { Host::Direct } else { Host::Wsl }
    }
    /// `bash` where this project's agent lives, as a program and its first arguments; with `cd`, in the project's folder
    /// (the second value is the folder to start in, for a host that takes it that way).
    fn bash(&self, cd: bool) -> (String, Vec<String>, Option<PathBuf>) {
        match self.host() {
            Host::Wsl => {
                let mut a = vec!["-d".to_string(), self.distro.clone()];
                if cd { a.extend(["--cd".to_string(), self.wsl_root.clone()]); }
                a.extend(["--exec".to_string(), "bash".to_string()]);
                ("wsl.exe".into(), a, None)
            }
            _ => ("bash".into(), vec![], if cd { Some(self.root.clone()) } else { None }),
        }
    }
    /// The same as a command that waits for its output (no window flashes up on Windows).
    fn bash_command(&self, cd: bool, args: &[&str]) -> std::process::Command {
        let (program, first, dir) = self.bash(cd);
        let mut c = std::process::Command::new(program);
        c.args(first).args(args);
        if let Some(d) = dir { c.current_dir(d); }
        quiet(c)
    }
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
/// Whether a project of maestro.config.json has its agent on Windows itself. "native" is a Windows matter (no WSL in between);
/// on macOS and Linux every project is simply on this machine.
fn runs_direct(p: &Value) -> bool {
    let s = |k: &str| p[k].as_str().unwrap_or("");
    cfg!(windows) && (s("runs") == "windows" || s("runs") == "native" || s("wslRoot").is_empty())
}
/// The project at one place in the list (a session belongs to its project, whichever one the window is showing).
fn project_at(i: usize) -> Result<Project, String> {
    let c = config();
    let p = c["projects"].get(i).or_else(|| c["projects"].get(0)).ok_or("maestro.config.json names no project")?;
    let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
    let root = PathBuf::from(s("root"));
    let dot = dot_of(&root);
    Ok(Project { name: s("name"), native: runs_direct(p), instructions: instructions_of(&root, &dot), dot, root, claude_home: PathBuf::from(s("claudeHome")), memory: PathBuf::from(s("memoryDir")), wsl_root: s("wslRoot"), kit: s("kit"), audits: p["audits"].as_array().cloned().unwrap_or_default().iter().filter_map(|a| a.as_str().map(PathBuf::from)).collect(), distro: if s("distro").is_empty() { "Ubuntu".into() } else { s("distro") } })
}

/// Run one kit command where the project's agent lives, in the project's folder, and return what it printed. The kit stays the
/// one place that knows how a ledger row closes; this app never re-implements it. Inside WSL `--exec` starts it with the
/// arguments as given: through a shell, wsl.exe re-splits the line and a quoted script arrives in pieces.
fn wsl(p: &Project, args: &[&str]) -> Result<String, String> {
    if p.kit.is_empty() || p.host() == Host::Direct || (p.host() == Host::Wsl && p.wsl_root.is_empty()) { return Err("this needs a kit: a script named as \"kit\" for this project in maestro.config.json (see docs/kit.md)".into()); }
    let mut all: Vec<&str> = vec![&p.kit];
    all.extend_from_slice(args);
    let out = p.bash_command(true, &all).output().map_err(|e| format!("the kit could not be started: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.trim().is_empty() && !out.status.success() { return Err(String::from_utf8_lossy(&out.stderr).trim().to_string()); }
    Ok(text)
}
/// A command that opens no console window of its own (Windows).
#[allow(unused_mut)]
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
/// Claude Code's own folder when it runs on this system itself (not inside WSL): ~/.claude
fn windows_claude_home() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".claude"))
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
    if !cfg!(windows) { return derive_local(folder); }
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
    Ok(json!({ "entry": entry, "choices": { "wsl": can_wsl, "windows": can_win, "distro": distro }, "found": found_in(&root_path, &memory, if native { win_has } else { wsl_has }) }))
}
/// What is in a project's folder: where its harness is and how much of one there is.
fn found_in(root: &Path, memory: &Path, worked: bool) -> Value {
    let dot = dot_of(root);
    let instructions = instructions_of(root, &dot);
    json!({
        "dir": rel(&dot), "claude": dot.is_dir(), "kit": dot.join("ctx.json").exists(),
        "instructions": instructions.exists(), "instructionsName": name_of(&instructions),
        "rules": md_files(&dot.join("rules")).len(), "log": md_files(&dot.join("log")).iter().filter(|f| date_in(&name_of(f)).is_some()).count(),
        "skills": fs::read_dir(dot.join("skills")).map(|r| r.filter_map(|e| e.ok()).filter(|e| e.path().join("SKILL.md").exists()).count()).unwrap_or(0),
        "memory": memory.is_dir(), "memories": md_files(memory).iter().filter(|f| name_of(f) != "MEMORY.md").count(),
        "worked": worked,
    })
}
/// The same on macOS and Linux, where there is one answer: the folder is on this machine and so is the agent.
fn derive_local(folder: &str) -> Result<Value, String> {
    let t = folder.trim().trim_matches('"').to_string();
    if t.is_empty() { return Err("type or pick the project's folder".into()); }
    let home = home_dir().ok_or("the home folder of this user is not known")?;
    let path = if t == "~" { home.clone() } else if let Some(rest) = t.strip_prefix("~/") { home.join(rest) } else { PathBuf::from(&t) };
    if !path.is_absolute() { return Err("give the whole path, for example ~/projects/my-site".into()); }
    if !path.is_dir() { return Err(format!("there is no folder at {}", path.display())); }
    let root = path.canonicalize().unwrap_or(path).to_string_lossy().trim_end_matches('/').to_string();
    let root_path = PathBuf::from(&root);
    let claude_home = home.join(".claude");
    let projects = claude_home.join("projects");
    let (strict, plain) = (dashed(&root), root.replace('/', "-"));
    let slug_dir = [strict.clone(), plain].into_iter().find(|n| projects.join(n).is_dir()).unwrap_or(strict);
    let memory = projects.join(&slug_dir).join("memory");
    let name = root_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Ok(json!({
        "entry": { "name": name, "root": root, "runs": "native", "wslRoot": "", "distro": "", "claudeHome": claude_home.to_string_lossy(), "memoryDir": memory.to_string_lossy(), "kit": "" },
        "choices": { "wsl": false, "windows": false, "distro": "" },
        "found": found_in(&root_path, &memory, projects.join(&slug_dir).is_dir()),
    }))
}
/// What this machine has, for the welcome screen: which system it is, the WSL distros (Windows), and whether Claude Code is here.
#[tauri::command(async)]
fn machine() -> Value {
    json!({ "platform": PLATFORM, "distros": if cfg!(windows) { wsl_distros() } else { vec![] }, "claudeFolder": windows_claude_home().map(|h| h.is_dir()).unwrap_or(false), "claudeProgram": find_claude().is_some() })
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

/// A path the kit wrote the WSL way (/mnt/d/x) as Windows wants it (D:\\x). On macOS and Linux a path is already right.
fn win_path(pth: &str) -> PathBuf {
    let b = pth.as_bytes();
    if cfg!(windows) && pth.starts_with("/mnt/") && b.len() > 6 && b[6] == b'/' {
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
    let out = quiet(c).output().map_err(|e| format!("git could not be started ({e}): is git installed{}?", if cfg!(windows) { " (Git for Windows)" } else { "" }))?;
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
    let native: Vec<bool> = list.iter().map(|p| cfg!(windows) && (p["runs"] == "windows" || p["runs"] == "native" || p["wslRoot"].as_str().unwrap_or("").is_empty())).collect();
    json!({ "names": names, "roots": roots, "favs": favs, "native": native, "current": cur_of(&cur, &window) })
}
#[tauri::command]
fn set_project(i: usize, cur: State<Cur>, window: tauri::WebviewWindow) -> Result<(), String> {
    let n = config()["projects"].as_array().map(|a| a.len()).unwrap_or(0);
    if i >= n { return Err("no such project".into()); }
    cur.0.lock().map_err(|e| e.to_string())?.insert(window.label().to_string(), i);
    Ok(())
}

// ---- updates: asked for by the user (or at start, if they turned that on). Nothing else in this program uses the network. ----
/// The repository whose published releases this build updates from. A fork that publishes its own releases changes this line.
const REPO: &str = "Nils-Kelley/maestro";
/// The build a release carries for this system, and the program's name inside it.
const ASSET: &str = if cfg!(windows) { "maestro-windows-x64.zip" } else if cfg!(target_os = "macos") { if cfg!(target_arch = "aarch64") { "maestro-macos-arm64.tar.gz" } else { "maestro-macos-x64.tar.gz" } } else { "maestro-linux-x64.tar.gz" };
const EXE: &str = if cfg!(windows) { "maestro.exe" } else { "maestro" };
/// curl and tar as the system ships them. On Windows by their full path, so that nothing else on PATH can stand in for them.
fn system_tool(name: &str) -> PathBuf {
    if cfg!(windows) { PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("System32").join(format!("{name}.exe")) } else { PathBuf::from(name) }
}
fn fetch(url: &str, to: Option<&Path>) -> Result<Vec<u8>, String> {
    let mut c = std::process::Command::new(system_tool("curl"));
    c.args(["--silent", "--show-error", "--fail", "--location", "--max-time", "120", "--proto", "=https", "--user-agent", "maestro-update-check", "--header", "Accept: application/vnd.github+json"]);
    if let Some(f) = to { c.arg("--output").arg(f); }
    c.arg(url);
    let out = quiet(c).output().map_err(|e| format!("curl could not be started ({e})"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if why.contains("404") { "no release has been published yet".into() } else { format!("could not reach GitHub: {why}") });
    }
    Ok(out.stdout)
}
/// 0.10.1 is newer than 0.9.4: compared number by number, a leading "v" and anything after a dash ignored.
fn newer(latest: &str, current: &str) -> bool {
    let nums = |v: &str| v.trim().trim_start_matches('v').split('-').next().unwrap_or("").split('.').map(|n| n.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (nums(latest), nums(current));
    (0..a.len().max(b.len())).map(|i| (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0))).find(|(x, y)| x != y).map(|(x, y)| x > y).unwrap_or(false)
}
/// The latest published release: its version, its notes, and the address of the build if it carries one.
fn latest_release() -> Result<Value, String> {
    let raw = fetch(&format!("https://api.github.com/repos/{REPO}/releases/latest"), None)?;
    let r: Value = serde_json::from_slice(&raw).map_err(|e| format!("GitHub's answer could not be read: {e}"))?;
    let tag = r["tag_name"].as_str().ok_or("GitHub's answer names no release")?.to_string();
    let want = format!("https://github.com/{REPO}/releases/download/");
    let asset = r["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"] == ASSET)).and_then(|x| x["browser_download_url"].as_str()).filter(|u| u.starts_with(&want)).map(|u| u.to_string());
    let current = env!("CARGO_PKG_VERSION");
    Ok(json!({ "current": current, "latest": tag.trim_start_matches('v'), "tag": tag, "newer": newer(&tag, current), "notes": r["body"].as_str().unwrap_or("").chars().take(6000).collect::<String>(), "published": r["published_at"], "asset": asset }))
}
/// Ask GitHub whether a newer release has been published. This is the app's only network request, and it is made only here.
#[tauri::command(async)]
fn update_check() -> Result<Value, String> {
    let mut v = latest_release()?;
    v["auto"] = json!(config()["checkUpdates"].as_bool().unwrap_or(false));
    Ok(v)
}
/// What this build is, without asking anyone.
#[tauri::command]
fn update_state() -> Value {
    json!({ "platform": PLATFORM, "current": env!("CARGO_PKG_VERSION"), "auto": config()["checkUpdates"].as_bool().unwrap_or(false), "repo": REPO })
}
/// Check at start, or only when asked (the default).
#[tauri::command]
fn update_auto_set(on: bool) -> Result<bool, String> {
    let mut c = config();
    c["checkUpdates"] = json!(on);
    save_config(&c)?;
    Ok(on)
}
/// Open the release's page in the browser.
#[tauri::command]
fn update_page(tag: String) -> Result<(), String> {
    if tag.is_empty() || !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') { return Err("not a release".into()); }
    let mut c = std::process::Command::new(if cfg!(windows) { "explorer.exe" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
    c.arg(format!("https://github.com/{REPO}/releases/tag/{tag}"));
    quiet(c).spawn().map(|_| ()).map_err(|e| e.to_string())
}
/// Put a downloaded build in the place of this one. The running program cannot be overwritten but it can be renamed: it is moved
/// aside as maestro.exe.old (removed at the next start), the new one takes its name, and a failure puts the old one back.
fn swap_in(dir: &Path, exe: &Path) -> Result<(), String> {
    let size = fs::metadata(dir.join(EXE)).map(|m| m.len()).map_err(|_| format!("the download holds no {EXE}"))?;
    if size < 1_000_000 { return Err(format!("the downloaded {EXE} is too small to be the program")); }
    let here = exe.parent().ok_or("this program has no folder")?;
    let mut done: Vec<(PathBuf, PathBuf)> = vec![]; // (file, where the old one went)
    let put = |name: &str, target: &Path, done: &mut Vec<(PathBuf, PathBuf)>| -> Result<(), String> {
        let old = target.with_file_name(format!("{}.old", name_of(target)));
        let _ = fs::remove_file(&old);
        if target.exists() { fs::rename(target, &old).map_err(|e| format!("{} could not be moved aside: {e}", name_of(target)))?; }
        done.push((target.to_path_buf(), old));
        fs::copy(dir.join(name), target).map(|_| ()).map_err(|e| format!("{name} could not be written: {e}"))
    };
    let mut result = put(EXE, exe, &mut done);
    #[cfg(unix)]
    if result.is_ok() {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(exe, fs::Permissions::from_mode(0o755));
    }
    if result.is_ok() && dir.join("WebView2Loader.dll").exists() { result = put("WebView2Loader.dll", &here.join("WebView2Loader.dll"), &mut done); }
    if let Err(e) = result {
        for (target, old) in done.iter().rev() { let _ = fs::remove_file(target); let _ = fs::rename(old, target); }
        return Err(e);
    }
    Ok(())
}
/// Download the latest release, put it in place of this program and start it. The address comes from GitHub's answer here, never
/// from the page. The download is fetched over HTTPS from this repository's releases; it is not signed.
#[tauri::command(async)]
fn update_install(app: tauri::AppHandle) -> Result<String, String> {
    let r = latest_release()?;
    if r["newer"] != true { return Err(format!("this is already the latest version ({})", r["current"].as_str().unwrap_or(""))); }
    let url = r["asset"].as_str().ok_or("that release carries no build to download: open its page instead")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = std::env::temp_dir().join(format!("maestro-update-{}", r["latest"].as_str().unwrap_or("x")));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join(ASSET);
    fetch(url, Some(&zip))?;
    let mut c = std::process::Command::new(system_tool("tar"));
    c.arg("-xf").arg(&zip).arg("-C").arg(&dir);
    let out = quiet(c).output().map_err(|e| format!("tar could not be started ({e})"))?;
    if !out.status.success() { return Err(format!("the download could not be unpacked: {}", String::from_utf8_lossy(&out.stderr).trim())); }
    swap_in(&dir, &exe)?;
    let _ = fs::remove_dir_all(&dir);
    // start the new one once this one has gone (a second copy started now would only wake this one and leave)
    let mut c;
    if cfg!(windows) {
        c = std::process::Command::new("powershell.exe");
        c.args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", "Start-Sleep -Milliseconds 1500; Start-Process -FilePath $env:MAESTRO_EXE"]);
    } else {
        c = std::process::Command::new("sh");
        c.args(["-c", "sleep 1.5; exec \"$MAESTRO_EXE\" >/dev/null 2>&1 &"]);
    }
    c.env("MAESTRO_EXE", &exe);
    quiet(c).spawn().map_err(|e| format!("the update is in place, but Maestro could not be restarted ({e}): start it yourself"))?;
    let tag = r["latest"].as_str().unwrap_or("").to_string();
    std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_millis(400)); app.exit(0); });
    Ok(tag)
}

const RUN_KEY: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// Where "start at sign-in" is written on macOS (a launch agent) and Linux (an autostart entry). Windows keeps it in the registry.
fn autostart_file() -> Option<PathBuf> {
    let home = home_dir()?;
    if cfg!(target_os = "macos") { Some(home.join("Library").join("LaunchAgents").join("app.maestro.deck.plist")) } else if cfg!(windows) { None } else { Some(std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("autostart").join("maestro.desktop")) }
}
/// Does the system start Maestro at sign-in?
#[tauri::command]
fn autostart_get() -> bool {
    if let Some(f) = autostart_file() { return f.exists(); }
    let mut c = std::process::Command::new("reg.exe");
    c.args(["query", RUN_KEY, "/v", "Maestro"]);
    quiet(c).output().map(|o| o.status.success()).unwrap_or(false)
}
#[tauri::command]
fn autostart_set(on: bool) -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(f) = autostart_file() {
        if !on { let _ = fs::remove_file(&f); return Ok(autostart_get()); }
        let xml = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let text = if cfg!(target_os = "macos") {
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>app.maestro.deck</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--hidden</string></array>\n<key>RunAtLoad</key><true/>\n</dict></plist>\n", xml(&exe.to_string_lossy()))
        } else {
            format!("[Desktop Entry]\nType=Application\nName=Maestro\nComment=Your agents and their harness, in one place\nExec=\"{}\" --hidden\nTerminal=false\nX-GNOME-Autostart-enabled=true\n", exe.to_string_lossy().replace('"', "\\\""))
        };
        if let Some(d) = f.parent() { fs::create_dir_all(d).map_err(|e| e.to_string())?; }
        fs::write(&f, text).map_err(|e| e.to_string())?;
        return Ok(autostart_get());
    }
    let mut c = std::process::Command::new("reg.exe");
    if on {
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
/// The open terminals of one viewer: the app's windows share one, each remote connection has its own.
type TermMap = std::sync::Arc<Mutex<std::collections::HashMap<String, Term>>>;
#[derive(Default)]
struct Terms(TermMap);

/// The scripts that keep sessions alive inside WSL, built into the program.
const TERM_SH: &str = include_str!("../../wsl/term.sh");
const TMUX_CONF: &str = include_str!("../../wsl/tmux.conf");
/// Settings for the Playwright tool that make the Chrome it starts one that can be watched (see docs/remote.md).
const PLAYWRIGHT_JSON: &str = include_str!("../../wsl/playwright.json");
/// Put the session scripts where bash can run them (~/.local/share/maestro; inside WSL that is written through the
/// \\wsl.localhost share) whenever they differ from this build's, and return term.sh's path as that bash sees it.
fn launcher(p: &Project) -> Result<String, String> {
    let (dir, seen) = if p.host() == Host::Local {
        let d = home_dir().ok_or("the home folder of this user is not known")?.join(".local").join("share").join("maestro");
        let seen = d.join("term.sh").to_string_lossy().to_string();
        (d, seen)
    } else {
        let home = p.claude_home.parent().ok_or("this project's claudeHome is not a folder")?;
        let linux = linux_of(&home.to_string_lossy()).ok_or("this project's claudeHome is not a folder inside WSL (\\\\wsl.localhost\\<distro>\\home\\<you>\\.claude)")?;
        (home.join(".local").join("share").join("maestro"), format!("{linux}/.local/share/maestro/term.sh"))
    };
    for (name, text) in [("term.sh", TERM_SH), ("tmux.conf", TMUX_CONF), ("playwright.json", PLAYWRIGHT_JSON)] {
        let want = text.replace("\r\n", "\n");
        if rd(&dir.join(name)) != want {
            fs::create_dir_all(&dir).and_then(|_| fs::write(dir.join(name), want)).map_err(|e| format!("could not write {name} into {}: {e}", dir.display()))?;
        }
    }
    Ok(seen)
}
/// The project through which the sessions that outlive the app are reached: this one, or (for a project that runs on Windows
/// itself) the first of the list that runs inside WSL.
fn wsl_side(p: Project) -> Option<Project> {
    if p.host() != Host::Direct { return Some(p); }
    let n = config()["projects"].as_array().map(|a| a.len()).unwrap_or(0);
    (0..n).filter_map(|i| project_at(i).ok()).find(|q| q.host() == Host::Wsl)
}
/// A session this app started: maestro-…, or reins-… from a build before the rename.
fn ours(n: &str) -> bool {
    (n.starts_with("maestro-") || n.starts_with("reins-")) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
/// Claude Code on this system itself: claude.exe or claude (started directly) or claude.cmd (started through cmd.exe), from PATH
/// or ~/.local/bin.
fn find_claude() -> Option<(PathBuf, bool)> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(h) = home_dir() { dirs.push(h.join(".local").join("bin")); }
    let names: &[(&str, bool)] = if cfg!(windows) { &[("claude.exe", true), ("claude.cmd", false), ("claude.bat", false)] } else { &[("claude", true)] };
    for (name, direct) in names {
        if let Some(f) = dirs.iter().map(|d| d.join(name)).find(|f| f.is_file()) { return Some((f, *direct)); }
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
    let p = match proj { Some(i) => project_at(i)?, None => project(&cur, &window)? };
    let (cmd, made) = session_command(p, kind, mode, name, session, prompt, label)?;
    let shown = id.clone();
    spawn_term(&terms.0, id, cmd, cols, rows, move |data| {
        let _ = match data { Some(text) => app.emit("term", json!({ "id": shown, "data": text })), None => app.emit("term-exit", json!({ "id": shown })) };
    })?;
    Ok(made)
}
/// The program a session is, and the name it is given (empty for one that ends with its terminal).
fn session_command(p: Project, kind: String, mode: String, name: Option<String>, session: Option<String>, prompt: Option<String>, label: Option<String>) -> Result<(portable_pty::CommandBuilder, String), String> {
    use portable_pty::CommandBuilder;
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
        if !cfg!(windows) { return Err("PowerShell sessions are for Windows: open a shell instead".into()); }
        cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        cmd.cwd(&p.root);
    } else if kind == "claude" && p.host() == Host::Direct && mode != "attach" {
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
        let p = if mode == "attach" { wsl_side(p) } else if p.host() == Host::Direct { None } else { Some(p) }.ok_or("this project runs on Windows itself, so there is no WSL shell for it: open PowerShell here")?;
        if mode != "attach" && p.host() == Host::Wsl && p.wsl_root.is_empty() { return Err("this project has no wslRoot in maestro.config.json, so a session cannot be opened in WSL".into()); }
        let launcher = launcher(&p)?;
        let (program, first, dir) = p.bash(mode != "attach");
        cmd = CommandBuilder::new(program);
        if p.host() == Host::Wsl {
            // WSL turns the Windows PATH into Linux paths and complains, inside the session, about every entry on a drive that is not there:
            // give it only the system folders (the launcher sets the Linux PATH itself)
            let sys = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
            cmd.env("PATH", format!("{sys}\\System32;{sys}"));
        }
        cmd.args(first);
        cmd.arg(&launcher);
        if let Some(d) = dir { cmd.cwd(d); }
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
    Ok((cmd, made))
}
/// Run a session's program in a pseudo-terminal kept in `map` under `id`. `out` is given what it prints, and `None` when it ends.
fn spawn_term(map: &TermMap, id: String, cmd: portable_pty::CommandBuilder, cols: u16, rows: u16, out: impl Fn(Option<String>) + Send + 'static) -> Result<(), String> {
    use portable_pty::{native_pty_system, PtySize};
    let pair = native_pty_system().openpty(PtySize { rows: rows.max(8), cols: cols.max(20), pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
    let child = pair.slave.spawn_command(cmd).map_err(|e| format!("the session could not be started: {e}"))?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    map.lock().map_err(|e| e.to_string())?.insert(id.clone(), Term { writer, master: pair.master, child });
    // On Windows the output stream does not end when the program does, so nothing would ever say "ended": watch the program itself.
    // When it has exited, letting go of its terminal closes the stream, and the reader below reports the end.
    {
        let map = map.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(600));
            let Ok(mut map) = map.lock() else { break };
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
                        out(Some(text));
                    }
                }
            }
        }
        out(None);
    });
    Ok(())
}
/// The claude and shell sessions still running in tmux, whoever started them and whether or not a window shows them.
#[tauri::command(async)]
fn term_list(cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let Some(p) = project(&cur, &window).ok().and_then(wsl_side) else { return Ok(json!([])) };
    let Ok(launcher) = launcher(&p) else { return Ok(json!([])) };
    let Ok(out) = p.bash_command(false, &[&launcher, "--list"]).output() else { return Ok(json!([])) };
    Ok(json!(String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 6 || !ours(f[0]) { return None; }
        Some(json!({ "name": f[0], "created": f[1].parse::<i64>().unwrap_or(0), "kind": if f[2].is_empty() { "ubuntu" } else { f[2] }, "label": f[3], "path": f[4], "attached": f[5] != "0" }))
    }).collect::<Vec<_>>()))
}
/// Whether a session has what it needs where this project's agent runs: tmux (sessions outlive the app) and claude.
#[tauri::command(async)]
fn term_check(cur: State<Cur>, window: tauri::WebviewWindow) -> Result<Value, String> {
    let p = project(&cur, &window)?;
    if p.host() == Host::Direct { return Ok(json!({ "native": true, "claude": find_claude().is_some(), "place": "on Windows" })); }
    let launcher = launcher(&p)?;
    let out = p.bash_command(false, &[&launcher, "--check"]).output().map_err(|e| format!("bash could not be started: {e}"))?;
    let has: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).collect();
    let place = if p.host() == Host::Wsl { format!("in {}", p.distro) } else { "on this machine".to_string() };
    Ok(json!({ "native": false, "place": place, "tmux": has.iter().any(|h| h == "tmux"), "claude": has.iter().any(|h| h == "claude"), "install": if cfg!(target_os = "macos") { "brew install tmux" } else { "sudo apt install tmux" } }))
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
            let _ = p.bash_command(false, &[&launcher(&p)?, "--kill", &n]).output();
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
    let _ = p.bash_command(false, &[&launcher(&p)?, "--label", &name, &shown]).output();
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

/// The programs that write and read the clipboard on this system, in the order they are tried.
fn clipboard_tools(read: bool) -> Vec<(&'static str, Vec<&'static str>)> {
    if cfg!(windows) {
        if read { vec![("powershell.exe", vec!["-NoProfile", "-NonInteractive", "-Command", "[Console]::OutputEncoding = [Text.Encoding]::UTF8; $t = Get-Clipboard -Raw; if ($t) { [Console]::Out.Write($t) }"])] } else { vec![("clip.exe", vec![])] }
    } else if cfg!(target_os = "macos") {
        vec![(if read { "pbpaste" } else { "pbcopy" }, vec![])]
    } else if read {
        vec![("wl-paste", vec!["--no-newline"]), ("xclip", vec!["-selection", "clipboard", "-o"]), ("xsel", vec!["--clipboard", "--output"])]
    } else {
        vec![("wl-copy", vec![]), ("xclip", vec!["-selection", "clipboard"]), ("xsel", vec!["--clipboard", "--input"])]
    }
}
/// Put text on the clipboard (used when the page's own clipboard call is refused).
#[tauri::command]
fn clip_set(text: String) -> Result<(), String> {
    use std::io::Write;
    // clip.exe reads UTF-16 when the text starts with its byte-order mark; anything else is taken as the console's code page
    let bytes: Vec<u8> = if cfg!(windows) { let mut b = vec![0xFF, 0xFE]; for u in text.encode_utf16() { b.extend_from_slice(&u.to_le_bytes()); } b } else { text.into_bytes() };
    let mut last = String::from("no clipboard program was found");
    for (program, args) in clipboard_tools(false) {
        let mut c = std::process::Command::new(program);
        c.args(args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        match quiet(c).spawn() {
            Ok(mut child) => {
                let wrote = child.stdin.take().map(|mut i| i.write_all(&bytes)).unwrap_or(Ok(()));
                // wl-copy and xclip stay behind to serve the text: they are not waited for
                if cfg!(windows) || cfg!(target_os = "macos") { let _ = child.wait(); }
                return wrote.map_err(|e| e.to_string());
            }
            Err(e) => last = format!("{program}: {e}"),
        }
    }
    Err(last)
}
/// The text on the clipboard (used when the page's own clipboard call is refused). Empty when it holds no text.
#[tauri::command(async)]
fn clip_get() -> Result<String, String> {
    for (program, args) in clipboard_tools(true) {
        let mut c = std::process::Command::new(program);
        c.args(args);
        if let Ok(out) = quiet(c).output() { if out.status.success() { return Ok(String::from_utf8_lossy(&out.stdout).to_string()); } }
    }
    Ok(String::new())
}
/// An error the page caught goes into maestro.log beside the config, so a blank or stuck screen leaves a trace.
#[tauri::command]
fn note(text: String) {
    let beside = || if cfg!(windows) { std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) } else { app_data() };
    if let Some(dir) = config_path().and_then(|f| f.parent().map(|d| d.to_path_buf())).or_else(beside) {
        let _ = fs::create_dir_all(&dir);
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

// ---- Remote: the sessions from a phone or another computer (docs/remote.md). Off unless it is turned on. ----
// One port serves a small page and a WebSocket. Only connections that arrive over Tailscale (and from this machine itself)
// are answered. A device pairs once with a code that works one time for ten minutes, and from then on gives a token of its
// own, of which only the hash is kept; a device can be removed, which cuts it off at once. A remote terminal is a second
// viewer of the same tmux session: only sessions that outlive the app can be reached, and nothing here can read or change a
// project's files.
const REMOTE_PAGE: &str = include_str!("../../ui/remote.html");
const REMOTE_FILES: [(&str, &str, &str); 3] = [
    ("/xterm.js", "text/javascript; charset=utf-8", include_str!("../../ui/vendor/xterm.js")),
    ("/addon-fit.js", "text/javascript; charset=utf-8", include_str!("../../ui/vendor/addon-fit.js")),
    ("/xterm.css", "text/css; charset=utf-8", include_str!("../../ui/vendor/xterm.css")),
];
const REMOTE_ICON: &[u8] = include_bytes!("../icons/icon.png");
const REMOTE_SW: &str = include_str!("../../ui/remote-sw.js");
const REMOTE_MANIFEST: &str = r##"{"name":"Maestro","short_name":"Maestro","start_url":"/","display":"standalone","background_color":"#0b0e13","theme_color":"#0b0e13","icons":[{"src":"/icon.png","sizes":"512x512","type":"image/png"}]}"##;
const REMOTE_PORT: u16 = 4747;
struct RemoteRun {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    port: u16,
}
static REMOTE: Mutex<Option<RemoteRun>> = Mutex::new(None);

/// An address Tailscale hands out (100.64.0.0/10). Some internet providers hand out the same range, so the range alone
/// does not say "Tailscale": see `remote_gate`.
fn is_tailnet(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64
}
/// The address in what `tailscale ip -4` printed: one line, an address of the Tailscale range. Anything else (signed out,
/// stopped, an error message) is no address.
fn tailnet_ip_in(printed: &str) -> Option<std::net::Ipv4Addr> {
    printed.lines().find_map(|l| l.trim().parse::<std::net::Ipv4Addr>().ok()).filter(|ip| is_tailnet(*ip))
}
/// Run Tailscale's own command line and give what it printed. On macOS the app and its command line are one program;
/// the variable says which of the two is meant.
fn tailscale(args: &[&str]) -> Option<String> {
    let mut places: Vec<PathBuf> = vec![PathBuf::from("tailscale")];
    if cfg!(windows) {
        places.push(PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into())).join("Tailscale").join("tailscale.exe"));
    } else {
        places.extend(["/Applications/Tailscale.app/Contents/MacOS/Tailscale", "/usr/local/bin/tailscale", "/opt/homebrew/bin/tailscale", "/usr/bin/tailscale"].map(PathBuf::from));
    }
    places.iter().find_map(|exe| {
        let mut c = std::process::Command::new(exe);
        c.args(args).env("TAILSCALE_BE_CLI", "1");
        quiet(c).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    })
}
/// This machine's Tailscale address, as Tailscale itself says it (`tailscale ip -4`). Asked at most every 15 seconds.
fn tailscale_ip() -> Option<std::net::Ipv4Addr> {
    static SEEN: Mutex<Option<(std::time::Instant, Option<std::net::Ipv4Addr>)>> = Mutex::new(None);
    let mut seen = SEEN.lock().ok()?;
    if let Some((at, ip)) = *seen {
        if at.elapsed() < std::time::Duration::from_secs(15) { return ip; }
    }
    let ip = tailscale(&["ip", "-4"]).and_then(|printed| tailnet_ip_in(&printed));
    *seen = Some((std::time::Instant::now(), ip));
    ip
}
/// The secure address: Tailscale can answer https://<this machine's name> with a certificate of its own and hand the
/// connection to Maestro on this machine (`tailscale serve`). A page needs it to be allowed notifications. Gives the name,
/// and whether that is set up for this port. Asked at most every 15 seconds.
fn remote_secure_state(port: u16, fresh: bool) -> (String, bool) {
    static SEEN: Mutex<Option<(std::time::Instant, u16, String, bool)>> = Mutex::new(None);
    let Ok(mut seen) = SEEN.lock() else { return (String::new(), false) };
    if let Some((at, p, name, on)) = seen.as_ref() {
        if !fresh && *p == port && at.elapsed() < std::time::Duration::from_secs(15) { return (name.clone(), *on); }
    }
    let name = tailscale(&["status", "--json"]).and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["Self"]["DNSName"].as_str().map(|n| n.trim_end_matches('.').to_string())).unwrap_or_default();
    let on = !name.is_empty() && tailscale(&["serve", "status", "--json"]).map(|t| t.contains(&format!("127.0.0.1:{port}"))).unwrap_or(false);
    *seen = Some((std::time::Instant::now(), port, name.clone(), on));
    (name, on)
}
/// Who is answered at all. A connection from this machine to itself; or one that arrived on this machine's own Tailscale
/// address from an address of the Tailscale range. A connection that came in over any other interface is not answered,
/// whatever address it claims to come from.
fn remote_gate(peer: std::net::IpAddr, local: std::net::IpAddr, tailscale: Option<std::net::Ipv4Addr>) -> bool {
    let v4 = |ip: std::net::IpAddr| match ip { std::net::IpAddr::V4(a) => Some(a), std::net::IpAddr::V6(a) => a.to_ipv4_mapped() };
    let own = |ip: std::net::IpAddr| ip.is_loopback() || v4(ip).map(|a| a.is_loopback()).unwrap_or(false);
    if own(peer) || own(local) { return own(peer) && own(local); }
    matches!((v4(peer), v4(local), tailscale), (Some(p), Some(l), Some(t)) if l == t && is_tailnet(p))
}
fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
fn hex_random(bytes: usize) -> Result<String, String> {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).map_err(|e| format!("no random numbers from this system: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}
/// What is kept of a device's token: its SHA-256. The token itself exists only on the device.
fn token_hash(t: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(t.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}
/// Compared without stopping at the first difference, so the time taken says nothing about the secret.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}
/// A pairing code works once, for ten minutes, and is never written to disk.
const PAIR_SECS: u64 = 600;
/// A device that has not connected for this long has to pair again.
const DEVICE_IDLE_SECS: u64 = 30 * 86400;
static REMOTE_PAIR: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);
/// One writer of the remote part of maestro.config.json at a time.
static REMOTE_CONFIG: Mutex<()> = Mutex::new(());
fn pair_ok(have: &Option<(String, std::time::Instant)>, said: &str) -> bool {
    matches!(have, Some((code, at)) if at.elapsed().as_secs() < PAIR_SECS && same_token(code, said))
}
fn remote_pair_code() -> Result<String, String> {
    let code = hex_random(16)?;
    *REMOTE_PAIR.lock().map_err(|e| e.to_string())? = Some((code.clone(), std::time::Instant::now()));
    Ok(code)
}
/// A device gives the pairing code: the code is used up and the device gets a token of its own (id, name, token).
fn remote_enroll(said: &str, name: &str) -> Option<(String, String, String)> {
    {
        let mut pair = REMOTE_PAIR.lock().ok()?;
        if !pair_ok(&pair, said) { return None; }
        *pair = None;
    }
    let (id, token) = (hex_random(4).ok()?, hex_random(16).ok()?);
    let shown: String = one_line(name).chars().filter(|c| !c.is_control()).take(40).collect();
    let shown = if shown.trim().is_empty() { "A device".to_string() } else { shown };
    let _one = REMOTE_CONFIG.lock().ok()?;
    let mut c = config();
    if !c["remote"].is_object() { c["remote"] = json!({}); }
    if !c["remote"]["devices"].is_array() { c["remote"]["devices"] = json!([]); }
    c["remote"]["devices"].as_array_mut()?.push(json!({ "id": id, "name": shown, "hash": token_hash(&token), "added": now_secs(), "seen": now_secs() }));
    save_config(&c).ok()?;
    Some((id, shown, token))
}
/// Which paired device a token belongs to (id, name), if it has connected recently enough.
fn device_of(devices: &Value, token: &str, now: u64) -> Option<(String, String)> {
    if token.len() < 32 { return None; }
    let hash = token_hash(token);
    devices.as_array()?.iter().find(|d| same_token(d["hash"].as_str().unwrap_or(""), &hash) && now.saturating_sub(d["seen"].as_u64().unwrap_or(0)) < DEVICE_IDLE_SECS)
        .map(|d| (d["id"].as_str().unwrap_or("").to_string(), d["name"].as_str().unwrap_or("A device").to_string()))
}
/// The device a token belongs to; when it was last seen is written down at most once an hour.
fn remote_known(token: &str) -> Option<(String, String)> {
    let _one = REMOTE_CONFIG.lock().ok()?;
    let mut c = config();
    let now = now_secs();
    let found = device_of(&c["remote"]["devices"], token, now)?;
    let stale = c["remote"]["devices"].as_array_mut().and_then(|a| a.iter_mut().find(|d| d["id"] == found.0.as_str())).map(|d| {
        let old = now.saturating_sub(d["seen"].as_u64().unwrap_or(0)) > 3600;
        if old { d["seen"] = json!(now); }
        old
    }).unwrap_or(false);
    if stale { let _ = save_config(&c); }
    Some(found)
}
/// Wrong codes from one address: after five, that address is not listened to for five minutes.
static REMOTE_FAILS: Mutex<Vec<(std::net::IpAddr, u32, std::time::Instant)>> = Mutex::new(Vec::new());
fn remote_blocked(ip: std::net::IpAddr) -> bool {
    let Ok(mut fails) = REMOTE_FAILS.lock() else { return true };
    fails.retain(|f| f.2.elapsed() < std::time::Duration::from_secs(300));
    fails.iter().any(|f| f.0 == ip && f.1 >= 5)
}
fn remote_failed(ip: std::net::IpAddr) {
    if let Ok(mut fails) = REMOTE_FAILS.lock() {
        match fails.iter_mut().find(|f| f.0 == ip) {
            Some(f) => { f.1 += 1; f.2 = std::time::Instant::now(); }
            None => fails.push((ip, 1, std::time::Instant::now())),
        }
    }
}
/// The devices connected right now: which device, and the switch that cuts that connection.
struct RemoteLive {
    conn: u64,
    device: String,
    cut: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
static REMOTE_LIVE: Mutex<Vec<RemoteLive>> = Mutex::new(Vec::new());
static REMOTE_APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
/// Every pairing, connection, refusal and removal goes into maestro.log, and to the window if there is one.
fn remote_tell(what: &str, name: &str, from: Option<std::net::IpAddr>) {
    let t = now_secs() % 86400;
    note(format!("{:02}:{:02} UTC remote access: {what} — {name}{}", t / 3600, t % 3600 / 60, from.map(|ip| format!(" ({ip})")).unwrap_or_default()));
    if let Some(app) = REMOTE_APP.get() { let _ = app.emit("remote", json!({ "what": what, "name": name })); }
}
fn remote_start() -> Result<u16, String> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    let mut run = REMOTE.lock().map_err(|e| e.to_string())?;
    if let Some(r) = run.as_ref() { return Ok(r.port); }
    let port = config()["remote"]["port"].as_u64().filter(|p| (1024..65536).contains(p)).map(|p| p as u16).unwrap_or(REMOTE_PORT);
    // a build before devices had tokens of their own kept one shared code in the file: it is no longer accepted, take it out
    if let Ok(_one) = REMOTE_CONFIG.lock() {
        let mut c = config();
        if c["remote"].as_object_mut().map(|r| r.remove("token").is_some()).unwrap_or(false) { let _ = save_config(&c); }
    }
    // a server that was just stopped lets go of the port a moment later
    let mut tries = 0;
    let listener = loop {
        match std::net::TcpListener::bind(("0.0.0.0", port)) {
            Ok(l) => break l,
            Err(e) if tries >= 10 => return Err(format!("port {port} could not be opened: {e}")),
            Err(_) => { tries += 1; std::thread::sleep(std::time::Duration::from_millis(150)); }
        }
    };
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    *run = Some(RemoteRun { stop: stop.clone(), port });
    { let stop = stop.clone(); std::thread::spawn(move || push_watch(stop)); }
    std::thread::spawn(move || {
        let live = Arc::new(AtomicUsize::new(0));
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((s, peer)) => {
                    let Ok(local) = s.local_addr() else { continue };
                    // dropped unanswered: not from this machine or over Tailscale, too many at once, or too many wrong codes
                    let tailscale = if peer.ip().is_loopback() { None } else { tailscale_ip() };
                    if !remote_gate(peer.ip(), local.ip(), tailscale) || live.load(Ordering::Relaxed) >= 32 || remote_blocked(peer.ip()) { continue; }
                    let (stop, live) = (stop.clone(), live.clone());
                    live.fetch_add(1, Ordering::Relaxed);
                    std::thread::spawn(move || {
                        let _ = s.set_nonblocking(false);
                        remote_conn(s, peer.ip(), &stop);
                        live.fetch_sub(1, Ordering::Relaxed);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
            }
        }
    });
    Ok(port)
}
fn remote_stop() {
    if let Some(r) = REMOTE.lock().ok().and_then(|mut r| r.take()) { r.stop.store(true, std::sync::atomic::Ordering::Relaxed); }
    if let Ok(mut pair) = REMOTE_PAIR.lock() { *pair = None; }
}
/// The first lines of a request: the method, the path without its query, and the headers with their names in lower case.
/// Anything that is not a plain HTTP/1 request for a path is refused.
fn remote_head(head: &[u8]) -> Option<(String, String, Vec<(String, String)>)> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target, version) = (first.next()?, first.next()?, first.next()?);
    if first.next().is_some() || !version.starts_with("HTTP/1.") || !target.starts_with('/') || target.contains("..") { return None; }
    let headers = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
    Some((method.to_string(), target.split('?').next().unwrap_or("/").to_string(), headers))
}
/// One connection: a file of the page, or the WebSocket the page talks through.
fn remote_conn(mut s: std::net::TcpStream, peer: std::net::IpAddr, stop: &std::sync::atomic::AtomicBool) {
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
    let _ = s.set_nodelay(true);
    // the whole request head has to arrive within five seconds and fit in 16 KB
    let began = Instant::now();
    let mut head: Vec<u8> = vec![];
    let mut b = [0u8; 2048];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        if head.len() > 16384 || began.elapsed() > Duration::from_secs(5) { return; }
        match s.read(&mut b) { Ok(0) | Err(_) => return, Ok(n) => head.extend_from_slice(&b[..n]) }
    }
    let plain = |s: &mut std::net::TcpStream, status: &str, kind: &str, body: &[u8]| {
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws: wss:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'\r\nConnection: close\r\n\r\n", body.len());
        let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(body)).and_then(|_| s.flush());
    };
    let Some((method, path, headers)) = remote_head(&head) else { return plain(&mut s, "400 Bad Request", "text/plain", b"not a request this answers") };
    let hdr = |k: &str| headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if method != "GET" { return plain(&mut s, "405 Method Not Allowed", "text/plain", b"GET only"); }
    if path != "/ws" {
        return match path.as_str() {
            // for work on the page itself: "remote.page" in maestro.config.json names a file to serve in place of the built-in one
            "/" | "/index.html" => match config()["remote"]["page"].as_str().and_then(|f| fs::read(f).ok()) {
                Some(page) => plain(&mut s, "200 OK", "text/html; charset=utf-8", &page),
                None => plain(&mut s, "200 OK", "text/html; charset=utf-8", REMOTE_PAGE.as_bytes()),
            },
            "/icon.png" => plain(&mut s, "200 OK", "image/png", REMOTE_ICON),
            "/sw.js" => plain(&mut s, "200 OK", "text/javascript; charset=utf-8", REMOTE_SW.as_bytes()),
            "/manifest.webmanifest" => plain(&mut s, "200 OK", "application/manifest+json", REMOTE_MANIFEST.as_bytes()),
            _ => match REMOTE_FILES.iter().find(|(p, _, _)| *p == path) {
                Some((_, kind, body)) => plain(&mut s, "200 OK", kind, body.as_bytes()),
                None => plain(&mut s, "404 Not Found", "text/plain", b"not here"),
            },
        };
    }
    // the WebSocket: only from the page this server served
    let Some(key) = hdr("sec-websocket-key") else { return plain(&mut s, "400 Bad Request", "text/plain", b"a WebSocket is expected here") };
    // (behind the secure address the request is handed on by Tailscale, which may name the first host in a header of its own)
    let from = hdr("origin").and_then(|o| o.split("://").nth(1));
    if from.is_none() || (from != hdr("host") && from != hdr("x-forwarded-host")) { return plain(&mut s, "403 Forbidden", "text/plain", b"only the page served here may connect"); }
    let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
    if s.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).is_err() { return; }
    let mut limits = tungstenite::protocol::WebSocketConfig::default();
    limits.max_message_size = Some(1 << 20);
    limits.max_frame_size = Some(1 << 20);
    let mut ws = tungstenite::WebSocket::from_raw_socket(s, tungstenite::protocol::Role::Server, Some(limits));
    let send = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>, v: String| ws.send(tungstenite::Message::text(v)).is_ok();
    // the first thing said has to be a device's own token, or the pairing code that earns it one
    let hello = match ws.read() { Ok(m) if m.is_text() => m.into_text().map(|t| t.as_str().to_string()).unwrap_or_default(), _ => return };
    let said = serde_json::from_str::<Value>(&hello).ok().filter(|m| m["op"] == "hello");
    let who = said.and_then(|m| match m["pair"].as_str() {
        Some(code) => remote_enroll(code, m["name"].as_str().unwrap_or("")).map(|(id, name, token)| (id, name, Some(token))),
        None => remote_known(m["token"].as_str().unwrap_or("")).map(|(id, name)| (id, name, None)),
    });
    let Some((device, name, fresh)) = who else {
        remote_failed(peer);
        remote_tell("refused", "a wrong, used or expired code", Some(peer));
        std::thread::sleep(Duration::from_secs(1));
        send(&mut ws, json!({ "op": "denied" }).to_string());
        let _ = ws.close(None);
        return;
    };
    let vapid = { use base64::Engine; vapid_key().ok().map(|k| b64u().encode(public_bytes(&k))) };
    if !send(&mut ws, json!({ "op": "hello", "host": host_name(), "version": env!("CARGO_PKG_VERSION"), "device": name, "token": fresh, "vapid": vapid }).to_string()) { return; }
    static CONNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let conn = CONNS.fetch_add(1, Ordering::Relaxed);
    let cut = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Ok(mut live) = REMOTE_LIVE.lock() { live.push(RemoteLive { conn, device: device.clone(), cut: cut.clone() }); }
    remote_tell(if fresh.is_some() { "paired" } else { "connected" }, &name, Some(peer));
    // from here on: read what the page says for a moment, send what the sessions printed, and again
    let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(25)));
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let terms: TermMap = Default::default();
    let watching: Watching = Default::default();
    let mut pinged = Instant::now();
    'conn: while !stop.load(Ordering::Relaxed) && !cut.load(Ordering::Relaxed) {
        match ws.read() {
            Ok(m) if m.is_text() => remote_op(m.into_text().map(|t| t.as_str().to_string()).unwrap_or_default().as_str(), &device, &tx, &terms, &watching),
            Ok(m) if m.is_close() => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
        while let Ok(m) = rx.try_recv() {
            if !send(&mut ws, m) { break 'conn; }
        }
        if pinged.elapsed() > Duration::from_secs(20) {
            pinged = Instant::now();
            if ws.send(tungstenite::Message::Ping(Vec::new().into())).is_err() { break; }
        }
    }
    let _ = ws.close(None);
    if let Ok(mut live) = REMOTE_LIVE.lock() { live.retain(|l| l.conn != conn); }
    remote_tell("left", &name, Some(peer));
    if let Some(w) = watching.lock().ok().and_then(|mut h| h.take()) { w.stop.store(true, Ordering::Relaxed); }
    // letting go of the terminals only detaches: the sessions keep running in tmux
    let held: Vec<Term> = terms.lock().map(|mut map| map.drain().map(|(_, t)| t).collect()).unwrap_or_default();
    for mut t in held { let _ = t.child.kill(); }
}
fn host_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok().filter(|h| !h.is_empty()).or_else(|| fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_string())).unwrap_or_else(|| "this computer".into())
}
/// The project through which the sessions that outlive the app are reached, with no window to ask: the first that keeps them.
fn remote_side() -> Option<Project> {
    let c = config();
    let i = c["projects"].as_array()?.iter().position(|p| !runs_direct(p))?;
    project_at(i).ok()
}
/// What a remote page shows first: the running sessions with the last lines on their screens, and the projects to start one in.
fn remote_list() -> Value {
    let c = config();
    let projects: Vec<Value> = c["projects"].as_array().map(|a| a.iter().enumerate().map(|(i, p)| json!({ "i": i, "name": p["name"].as_str().unwrap_or(""), "keeps": !runs_direct(p), "favorite": p["favorite"].as_bool().unwrap_or(false) })).collect()).unwrap_or_default();
    let mut sessions: Vec<Value> = vec![];
    if let Some((p, launcher)) = remote_side().and_then(|p| launcher(&p).ok().map(|l| (p, l))) {
        let run = |what: &str| p.bash_command(false, &[&launcher, what]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        let screens = run("--peek");
        let browsers = run("--browsers");
        let screen_of = |name: &str| screens.split('\u{1e}').find_map(|b| b.strip_prefix(name).and_then(|r| r.strip_prefix('\n'))).unwrap_or("").to_string();
        for l in run("--list").lines() {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 6 || !ours(f[0]) { continue; }
            sessions.push(json!({ "name": f[0], "created": f[1].parse::<i64>().unwrap_or(0), "kind": if f[2].is_empty() { "ubuntu" } else { f[2] }, "label": f[3], "project": f[4].rsplit('/').next().unwrap_or(""), "screen": screen_of(f[0]), "browser": browsers.lines().any(|b| b.split('\t').next() == Some(f[0])) }));
        }
    }
    json!({ "op": "list", "projects": projects, "sessions": sessions })
}
// ---- Notifications to a paired device (Web Push): a page that was allowed them is told when a session starts waiting or
// finishes, while the page is closed. The message is encrypted for that one browser; the service that carries it (Apple's,
// Google's, Mozilla's) sees only that Maestro sent something. This is the one thing remote access sends out by itself.
fn b64u() -> base64::engine::GeneralPurpose { base64::engine::general_purpose::URL_SAFE_NO_PAD }
fn unhex(t: &str) -> Option<Vec<u8>> {
    if t.len() % 2 != 0 { return None; }
    (0..t.len()).step_by(2).map(|i| u8::from_str_radix(t.get(i..i + 2)?, 16).ok()).collect()
}
fn fresh_secret() -> Result<p256::SecretKey, String> {
    for _ in 0..8 {
        if let Some(k) = unhex(&hex_random(32)?).and_then(|b| p256::SecretKey::from_slice(&b).ok()) { return Ok(k); }
    }
    Err("no key could be made".into())
}
fn public_bytes(k: &p256::SecretKey) -> Vec<u8> {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    k.public_key().to_encoded_point(false).as_bytes().to_vec()
}
/// The key this computer signs its notifications with, made once and kept in maestro.config.json.
fn vapid_key() -> Result<p256::SecretKey, String> {
    let _one = REMOTE_CONFIG.lock().map_err(|e| e.to_string())?;
    let mut c = config();
    if let Some(k) = c["remote"]["vapid"].as_str().and_then(unhex).and_then(|b| p256::SecretKey::from_slice(&b).ok()) { return Ok(k); }
    let k = fresh_secret()?;
    if !c["remote"].is_object() { c["remote"] = json!({}); }
    c["remote"]["vapid"] = json!(k.to_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>());
    save_config(&c)?;
    Ok(k)
}
/// A message sealed for one browser (RFC 8291, "aes128gcm"): `ua` and `auth` are what the browser gave when it subscribed,
/// `mine` a key used for this message only, `salt` sixteen random bytes.
fn push_seal(ua: &[u8], auth: &[u8], payload: &[u8], mine: &p256::SecretKey, salt: &[u8; 16]) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::Aead;
    use aes_gcm::KeyInit;
    let theirs = p256::PublicKey::from_sec1_bytes(ua).map_err(|_| "the device's key is not a key")?;
    let shared = p256::ecdh::diffie_hellman(mine.to_nonzero_scalar(), theirs.as_affine());
    let my_public = public_bytes(mine);
    let mut ikm = [0u8; 32];
    let info = [b"WebPush: info\0".as_slice(), ua, &my_public].concat();
    hkdf::Hkdf::<sha2::Sha256>::new(Some(auth), shared.raw_secret_bytes()).expand(&info, &mut ikm).map_err(|e| e.to_string())?;
    let prk = hkdf::Hkdf::<sha2::Sha256>::new(Some(salt), &ikm);
    let (mut key, mut nonce) = ([0u8; 16], [0u8; 12]);
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut key).map_err(|e| e.to_string())?;
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce).map_err(|e| e.to_string())?;
    let plain = [payload, &[2u8]].concat(); // 2 marks the last (here: the only) record
    let sealed = aes_gcm::Aes128Gcm::new_from_slice(&key).map_err(|e| e.to_string())?.encrypt(aes_gcm::Nonce::from_slice(&nonce), plain.as_slice()).map_err(|_| "the message could not be sealed")?;
    Ok([salt.as_slice(), &4096u32.to_be_bytes(), &[my_public.len() as u8], &my_public, &sealed].concat())
}
/// The signed note that says who is sending (RFC 8292): for the service at `audience`, good for twelve hours.
fn push_pass(key: &p256::SecretKey, audience: &str, now: u64) -> String {
    use base64::Engine;
    use p256::ecdsa::signature::Signer;
    let head = b64u().encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let body = b64u().encode(json!({ "aud": audience, "exp": now + 12 * 3600, "sub": format!("https://github.com/{REPO}") }).to_string());
    let sig: p256::ecdsa::Signature = p256::ecdsa::SigningKey::from(key).sign(format!("{head}.{body}").as_bytes());
    format!("{head}.{body}.{}", b64u().encode(sig.to_bytes()))
}
/// Where a notification may be sent: only the services browsers really use, only over https.
fn push_host(endpoint: &str) -> Option<String> {
    let rest = endpoint.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    if host.contains('@') || host.contains(':') { return None; }
    ["push.apple.com", "googleapis.com", "push.services.mozilla.com", "notify.windows.com"].iter().any(|ok| host == *ok || host.ends_with(&format!(".{ok}"))).then_some(host)
}
/// Send one notification to one subscribed browser. Gives the service's answer (201 is "accepted"; 404 and 410 mean the
/// subscription is gone).
fn push_send(sub: &Value, payload: &Value) -> Result<u16, String> {
    use base64::Engine;
    use std::io::Write;
    let endpoint = sub["endpoint"].as_str().unwrap_or("");
    let host = push_host(endpoint).ok_or("not a notification service this app sends to")?;
    let un = |k: &str| b64u().decode(sub["keys"][k].as_str().unwrap_or("").trim_end_matches('=').replace('+', "-").replace('/', "_")).map_err(|_| "the subscription could not be read".to_string());
    let (ua, auth) = (un("p256dh")?, un("auth")?);
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
    let body = push_seal(&ua, &auth, payload.to_string().as_bytes(), &fresh_secret()?, &salt)?;
    let key = vapid_key()?;
    let pass = push_pass(&key, &format!("https://{host}"), now_secs());
    let mut c = std::process::Command::new(system_tool("curl"));
    c.args(["--silent", "--show-error", "--max-time", "20", "--proto", "=https", "--request", "POST", "--header", "TTL: 900", "--header", "Urgency: high", "--header", "Content-Encoding: aes128gcm", "--header", "Content-Type: application/octet-stream"]);
    c.args(["--header", &format!("Authorization: vapid t={pass}, k={}", b64u().encode(public_bytes(&key)))]);
    c.args(["--data-binary", "@-", "--output", if cfg!(windows) { "NUL" } else { "/dev/null" }, "--write-out", "%{http_code}", endpoint]);
    let mut child = quiet(c).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| format!("curl could not be started ({e})"))?;
    if let Some(mut input) = child.stdin.take() { let _ = input.write_all(&body); }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<u16>().ok().filter(|c| *c > 0).ok_or_else(|| format!("the notification service was not reached: {}", String::from_utf8_lossy(&out.stderr).trim()))
}
/// What a session's screen says, the way the pages read it: ("needs", the question) when Claude Code waits for an answer,
/// ("working", what it is doing) while it works, else ("idle", "").
fn screen_state(screen: &str, claude: bool) -> (&'static str, String) {
    let plain = |t: &str| t.chars().map(|c| if ('\u{2500}'..='\u{259f}').contains(&c) { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    let body: Vec<String> = screen.lines().map(plain).filter(|t| {
        let low = t.to_lowercase();
        let words = low.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
        let two = t.as_bytes().windows(2).any(|w| w[0].is_ascii_alphanumeric() && w[1].is_ascii_alphanumeric());
        two && !["for shortcuts", "shift+tab", "esc to", "auto mode", "bypass", "accept edits", "plan mode"].iter().any(|f| words.starts_with(f)) && !["shift+tab to cycle", "/effort", "for shortcuts", "/clear to save"].iter().any(|f| low.contains(f))
    }).collect();
    let timed = |t: &str| t.match_indices('(').any(|(i, _)| { let d: String = t[i + 1..].chars().take_while(|c| c.is_ascii_digit()).collect(); !d.is_empty() && t[i + 1 + d.len()..].starts_with('s') });
    let busy = body.iter().rev().take(25).find(|t| { let l = t.to_lowercase(); l.contains("esc to interrupt") || l.contains("tokens)") || timed(t) || l.contains("…(") || l.contains("… (") });
    if let Some(now) = busy {
        let said = now.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
        return ("working", said.split(" (").next().unwrap_or(said).to_string());
    }
    if !claude { return ("idle", String::new()); }
    // numbered answers at the end of the screen: "1. Yes", "2. …", counted from one
    let answer = |t: &str| -> Option<u32> {
        let t = t.trim_start_matches(|c: char| c == '❯' || c == '>' || c == '›' || c.is_whitespace());
        let mut ch = t.chars();
        let n = ch.next()?.to_digit(10)?;
        let rest = ch.as_str();
        ((rest.starts_with(". ") || rest.starts_with(") ")) && rest.chars().count() <= 72 && rest.trim().chars().count() > 1).then_some(n)
    };
    let (mut nums, mut at) = (vec![], body.len());
    for i in (body.len().saturating_sub(12)..body.len()).rev() {
        match answer(&body[i]) { Some(n) => { nums.insert(0, n); at = i; } None if !nums.is_empty() => break, None => {} }
    }
    if nums.len() >= 2 && nums.iter().enumerate().all(|(i, n)| *n as usize == i + 1) {
        return ("needs", body[..at].last().cloned().unwrap_or_default());
    }
    ("idle", String::new())
}
/// While remote access is on: watch the sessions for the devices that asked to be told, and tell them. A device that has
/// the page open is not told (it sees it there).
fn push_watch(stop: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::time::{Duration, Instant};
    // name → (state, since when, told about the end of this stretch of work)
    let mut was: std::collections::HashMap<String, (&'static str, Instant)> = Default::default();
    let mut worked: std::collections::HashMap<String, Instant> = Default::default();
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(4));
        let devices: Vec<(String, Value)> = config()["remote"]["devices"].as_array().map(|a| a.iter().filter(|d| d["push"]["endpoint"].is_string()).map(|d| (d["id"].as_str().unwrap_or("").to_string(), d["push"].clone())).collect()).unwrap_or_default();
        if devices.is_empty() { was.clear(); worked.clear(); continue; }
        let list = remote_list();
        let mut tell: Vec<Value> = vec![];
        let mut here: Vec<String> = vec![];
        for s in list["sessions"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            let (name, label) = (s["name"].as_str().unwrap_or("").to_string(), s["label"].as_str().unwrap_or("A session").to_string());
            let (st, detail) = screen_state(s["screen"].as_str().unwrap_or(""), s["kind"] == "claude");
            here.push(name.clone());
            let before = was.get(&name).map(|w| w.0);
            if before != Some(st) {
                if st == "working" { worked.insert(name.clone(), Instant::now()); }
                // a first look at a session says nothing: only a change does
                if before.is_some() && st == "needs" { tell.push(json!({ "title": label, "body": if detail.is_empty() { "Waiting for you".to_string() } else { format!("Waiting for you: {detail}") }, "name": name })); }
                was.insert(name.clone(), (st, Instant::now()));
            } else if st == "idle" && s["kind"] == "claude" {
                // work that lasted, then quiet for two looks: it has finished
                if let Some(began) = worked.get(&name).copied() {
                    let quiet_for = was.get(&name).map(|w| w.1.elapsed()).unwrap_or_default();
                    if quiet_for >= Duration::from_secs(7) {
                        worked.remove(&name);
                        if quiet_for.as_secs() < 60 && began.elapsed() - quiet_for >= Duration::from_secs(20) { tell.push(json!({ "title": label, "body": "Finished", "name": name })); }
                    }
                }
            }
        }
        was.retain(|n, _| here.contains(n));
        worked.retain(|n, _| here.contains(n));
        if tell.is_empty() { continue; }
        let live: Vec<String> = REMOTE_LIVE.lock().map(|l| l.iter().map(|x| x.device.clone()).collect()).unwrap_or_default();
        for (id, sub) in devices.iter().filter(|(id, _)| !live.contains(id)) {
            for t in &tell {
                match push_send(sub, t) {
                    Ok(404) | Ok(410) => { push_forget(id); break; }
                    Ok(code) if code >= 400 => note(format!("remote access: a notification was refused by the service ({code})")),
                    Ok(_) => {}
                    Err(e) => { note(format!("remote access: a notification was not sent: {e}")); break; }
                }
            }
        }
    }
}
/// Keep, or (with `None`) forget, where a device is sent its notifications.
fn push_keep(device: &str, sub: Option<Value>) {
    let Ok(_one) = REMOTE_CONFIG.lock() else { return };
    let mut c = config();
    let Some(d) = c["remote"]["devices"].as_array_mut().and_then(|a| a.iter_mut().find(|d| d["id"] == device)) else { return };
    match sub { Some(s) => d["push"] = s, None => { if let Some(o) = d.as_object_mut() { o.remove("push"); } } }
    let _ = save_config(&c);
}
fn push_forget(device: &str) { push_keep(device, None); }

/// The end of a file, as text: a conversation file grows to many megabytes and only its last part is wanted.
fn file_tail(f: &Path, bytes: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = fs::File::open(f) else { return String::new() };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(bytes);
    if file.seek(SeekFrom::Start(from)).is_err() { return String::new(); }
    let mut buf = Vec::new();
    let _ = file.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    // the first line is a cut one unless the whole file was read
    if from == 0 { text } else { text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default() }
}
/// What was said in a conversation file, oldest first, as (id, role, text): role is "user", "assistant" or "tool" (one line
/// saying what was done). Thinking, tool results, side conversations and the program's own notes are left out.
fn chat_items(text: &str) -> Vec<(String, &'static str, String)> {
    let mut out = vec![];
    let clip = |t: &str, n: usize| -> String { let t = t.trim(); if t.chars().count() > n { format!("{}…", t.chars().take(n).collect::<String>()) } else { t.to_string() } };
    for line in text.lines() {
        // most lines of the file are not messages: skip them before they are parsed
        if !(line.contains("\"type\":\"user\"") || line.contains("\"type\":\"assistant\"")) { continue; }
        let Ok(o) = serde_json::from_str::<Value>(line) else { continue };
        let role = match o["type"].as_str() { Some("user") => "user", Some("assistant") => "assistant", _ => continue };
        if o["isSidechain"].as_bool().unwrap_or(false) || o["isMeta"].as_bool().unwrap_or(false) { continue; }
        let Some(id) = o["uuid"].as_str() else { continue };
        let said = |t: &str| !t.trim().is_empty() && !(role == "user" && t.trim_start().starts_with('<'));
        match &o["message"]["content"] {
            Value::String(t) => if said(t) { out.push((id.to_string(), role, clip(t, 8000))); },
            Value::Array(blocks) => for (i, b) in blocks.iter().enumerate() {
                match b["type"].as_str() {
                    Some("text") => { let t = b["text"].as_str().unwrap_or(""); if said(t) { out.push((format!("{id}-{i}"), role, clip(t, 8000))); } }
                    Some("tool_use") if role == "assistant" => {
                        let input = &b["input"];
                        let what = ["command", "file_path", "path", "pattern", "url", "query", "description", "prompt", "skill"].iter().find_map(|k| input[*k].as_str()).unwrap_or("");
                        out.push((format!("{id}-{i}"), "tool", clip(&one_line(&format!("{} {what}", b["name"].as_str().unwrap_or("tool"))), 160)));
                    }
                    _ => {}
                }
            },
            _ => {}
        }
    }
    out
}
/// Letters and digits only, lower case: how a screen and a conversation are compared, whatever the terminal did to the line breaks.
fn squeeze(t: &str) -> String {
    t.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}
/// How well a conversation file fits what is on a session's screen: how many of its last things said can be read there.
fn chat_fit(items: &[(String, &'static str, String)], screen: &str) -> usize {
    items.iter().rev().filter(|i| i.1 != "tool").take(6).filter(|i| {
        // a stretch from the middle of the first line: the start may carry marks the screen draws differently
        let s = squeeze(i.2.lines().next().unwrap_or(""));
        let piece: String = s.chars().skip(if s.chars().count() > 30 { 4 } else { 0 }).take(24).collect();
        piece.chars().count() >= 8 && screen.contains(&piece)
    }).count()
}
/// The conversation file of a running Claude Code session. Sessions this app started carry their conversation's id; for the
/// others the files of that folder written since the session began are compared with what is on its screen. Kept for a while.
fn chat_file(p: &Project, launcher: &str, name: &str) -> Option<PathBuf> {
    static KNOWN: Mutex<Vec<(String, PathBuf, std::time::Instant)>> = Mutex::new(Vec::new());
    if let Ok(known) = KNOWN.lock() {
        if let Some(k) = known.iter().find(|k| k.0 == name && k.2.elapsed() < std::time::Duration::from_secs(20)) { return Some(k.1.clone()); }
    }
    let out = p.bash_command(false, &[launcher, "--chat", name]).output().ok()?;
    let printed = String::from_utf8_lossy(&out.stdout).to_string();
    let (first, screen) = printed.split_once('\n')?;
    let f: Vec<&str> = first.trim_end_matches('\r').split('\t').collect();
    if f.len() < 3 { return None; }
    let created = f[0].parse::<u64>().unwrap_or(0);
    // Claude Code names a project's folder after its path, every other character turned into a dash
    let folder: String = f[1].chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let dir = p.claude_home.join("projects").join(folder);
    // a session that carries its conversation's id has that conversation and no other: until Claude Code has written the file
    // (it does with the first message) nothing has been said, and the file of another conversation is not shown in its place
    let by_id = Some(f[2].trim()).filter(|s| s.len() >= 32 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).map(|s| dir.join(format!("{s}.jsonl")));
    if let Some(mine) = by_id.as_ref().filter(|f| !f.is_file()) { return Some(mine.clone()); }
    let found = by_id.or_else(|| {
        let mut files: Vec<(PathBuf, u64)> = fs::read_dir(&dir).ok()?.flatten().filter_map(|e| {
            let path = e.path();
            if path.extension().map(|x| x != "jsonl").unwrap_or(true) { return None; }
            let at = e.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            (at + 120 >= created).then_some((path, at))
        }).collect();
        files.sort_by(|a, b| b.1.cmp(&a.1));
        files.truncate(8);
        let screen = squeeze(screen);
        let scored: Vec<(usize, PathBuf)> = files.iter().map(|(f, _)| (chat_fit(&chat_items(&file_tail(f, 900_000)), &screen), f.clone())).collect();
        let best = scored.iter().max_by_key(|s| s.0)?;
        // nothing of it on the screen: only an only candidate is believed
        if best.0 > 0 { Some(best.1.clone()) } else if files.len() == 1 { Some(files[0].0.clone()) } else { None }
    });
    // a long stretch of work can leave nothing of the conversation on the screen: the file found before is still the one
    let found = found.or_else(|| KNOWN.lock().ok()?.iter().find(|k| k.0 == name && k.1.is_file()).map(|k| k.1.clone()))?;
    if let Ok(mut known) = KNOWN.lock() {
        known.retain(|k| k.0 != name && k.2.elapsed() < std::time::Duration::from_secs(600));
        known.push((name.to_string(), found.clone(), std::time::Instant::now()));
    }
    Some(found)
}
/// A session's conversation for a remote page: the last things said, or only those after `after` when the page has that one.
fn remote_chat(name: &str, after: &str) -> Value {
    let none = json!({ "op": "chat", "name": name, "found": false, "reset": true, "items": [] });
    if !ours(name) { return none; }
    let Some((p, launcher)) = remote_side().and_then(|p| launcher(&p).ok().map(|l| (p, l))) else { return none };
    let Some(file) = chat_file(&p, &launcher, name) else { return none };
    let all = chat_items(&file_tail(&file, 900_000));
    let from = all.iter().position(|i| i.0 == after).map(|i| i + 1);
    let items: Vec<Value> = all.iter().skip(from.unwrap_or(all.len().saturating_sub(60))).map(|(id, role, text)| json!({ "id": id, "role": role, "text": text })).collect();
    json!({ "op": "chat", "name": name, "found": true, "reset": from.is_none(), "items": items })
}
// ---- A session's browser, watched from a paired page. A program in a session (Playwright, for Claude Code) drives a Chrome;
// started with --remote-debugging-port, that Chrome takes a second connection on a port of this computer only, and over it
// sends a picture of the page each time the page changes. Those pictures are passed on to the one page that asked, at the
// pace it shows them. Nothing is typed or clicked from here: the page is only looked at.
struct Watch { name: String, stop: std::sync::atomic::AtomicBool, seen: std::sync::atomic::AtomicU64, pick: Mutex<String> }
type Watching = std::sync::Arc<Mutex<Option<std::sync::Arc<Watch>>>>;
/// Which sessions drive a Chrome that can be watched, and the port each one chose.
fn browser_ports() -> Vec<(String, u16)> {
    let Some((p, l)) = remote_side().and_then(|p| launcher(&p).ok().map(|l| (p, l))) else { return vec![] };
    let out = p.bash_command(false, &[&l, "--browsers"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    out.lines().filter_map(|l| { let (n, port) = l.trim_end_matches('\r').split_once('\t')?; Some((n.to_string(), port.trim().parse().ok()?)) }).filter(|(n, _)| ours(n)).collect()
}
/// What that Chrome has open, as it lists it itself: (id, title, address) of each page, the one in front first.
fn browser_pages(port: u16) -> Option<Vec<(String, String, String)>> {
    use std::io::{Read, Write};
    use std::time::Duration;
    let mut s = std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(2)).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
    s.write_all(format!("GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes()).ok()?;
    // the answer says how long it is; the connection may stay open after it
    let (mut got, mut b) = (Vec::new(), [0u8; 8192]);
    let body = loop {
        if let Some(at) = got.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&got[..at]).to_ascii_lowercase();
            let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok());
            if len.map(|n| got.len() >= at + 4 + n).unwrap_or(false) { break got[at + 4..].to_vec(); }
        }
        if got.len() > (4 << 20) { return None; }
        match s.read(&mut b) { Ok(0) | Err(_) => { let at = got.windows(4).position(|w| w == b"\r\n\r\n")?; break got[at + 4..].to_vec(); } Ok(n) => got.extend_from_slice(&b[..n]) }
    };
    let v: Value = serde_json::from_slice(&body).ok()?;
    let t = |o: &Value, k: &str| o[k].as_str().unwrap_or("").to_string();
    Some(v.as_array()?.iter().filter(|o| o["type"] == "page" && !t(o, "url").starts_with("devtools://") && !t(o, "url").starts_with("chrome-extension://")).map(|o| (t(o, "id"), t(o, "title"), t(o, "url"))).filter(|p| !p.0.is_empty() && p.0.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).collect())
}
/// Watch the browser of one session for one page, until told to stop: say what it has open, and pass on each picture of the
/// page in front (or of the one chosen). The browser is asked for the next picture only once this one was shown, or has had
/// its time: a slow connection gets fewer pictures, not a growing queue of old ones.
fn browser_watch(width: u32, w: std::sync::Arc<Watch>, tx: std::sync::mpsc::Sender<String>) {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let name = w.name.clone();
    let gone = || w.stop.load(Ordering::Relaxed);
    let nap = |ms: u64| { let till = Instant::now() + Duration::from_millis(ms); while Instant::now() < till && !gone() { std::thread::sleep(Duration::from_millis(50)); } };
    let state = |pages: &[(String, String, String)], page: &str| json!({ "op": "browser", "name": name, "state": if pages.is_empty() { "none" } else { "on" }, "page": page, "pages": pages.iter().map(|(id, title, url)| json!({ "id": id, "title": title, "url": url })).collect::<Vec<Value>>() }).to_string();
    let chosen = |pages: &[(String, String, String)]| { let pick = w.pick.lock().map(|p| p.clone()).unwrap_or_default(); pages.iter().find(|p| p.0 == pick).or(pages.first()).map(|p| p.0.clone()).unwrap_or_default() };
    let (mut said, mut n) = (String::new(), 0u64);
    while !gone() {
        let port = browser_ports().into_iter().find(|(s, _)| *s == name).map(|(_, p)| p);
        let pages = port.and_then(browser_pages).unwrap_or_default();
        let page = chosen(&pages);
        let now = state(&pages, &page);
        if now != said { said = now.clone(); if tx.send(now).is_err() { return; } }
        let (Some(port), false) = (port, page.is_empty()) else { nap(2000); continue };
        let Ok(s) = std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(2)) else { nap(1000); continue };
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
        let _ = s.set_nodelay(true);
        let mut limits = tungstenite::protocol::WebSocketConfig::default();
        limits.max_message_size = Some(24 << 20);
        limits.max_frame_size = Some(24 << 20);
        let Ok((mut ws, _)) = tungstenite::client::client_with_config(format!("ws://127.0.0.1:{port}/devtools/page/{page}"), s, Some(limits)) else { nap(1000); continue };
        let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(80)));
        let ask = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>, id: u64, method: &str, params: Value| ws.send(tungstenite::Message::text(json!({ "id": id, "method": method, "params": params }).to_string())).is_ok();
        if !ask(&mut ws, 1, "Page.startScreencast", json!({ "format": "jpeg", "quality": 60, "maxWidth": width, "maxHeight": width * 4, "everyNthFrame": 1 })) { nap(1000); continue; }
        let mut owed: Option<(i64, u64, Instant)> = None;
        let mut looked = Instant::now();
        loop {
            if gone() { let _ = ws.close(None); return; }
            match ws.read() {
                Ok(m) if m.is_text() => {
                    let text = m.into_text().map(|t| t.as_str().to_string()).unwrap_or_default();
                    // (most of what a page's connection says is not a picture: look before parsing something that large)
                    if text.contains("\"Page.screencastFrame\"") {
                        if let Ok(v) = serde_json::from_str::<Value>(&text) {
                            let p = &v["params"];
                            n += 1;
                            if tx.send(json!({ "op": "frame", "name": name, "n": n, "data": p["data"], "w": p["metadata"]["deviceWidth"], "h": p["metadata"]["deviceHeight"] }).to_string()).is_err() { return; }
                            owed = Some((p["sessionId"].as_i64().unwrap_or(0), n, Instant::now()));
                        }
                    }
                }
                Ok(m) if m.is_close() => break,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
            if let Some((sid, k, at)) = owed {
                if w.seen.load(Ordering::Relaxed) >= k || at.elapsed() > Duration::from_millis(1500) {
                    owed = None;
                    if !ask(&mut ws, 2, "Page.screencastFrameAck", json!({ "sessionId": sid })) { break; }
                }
            }
            // every so often: is this still the page to show, and is it still called the same
            if looked.elapsed() > Duration::from_millis(1500) || chosen_changed(&w, &page) {
                looked = Instant::now();
                let Some(pages) = browser_pages(port) else { break };
                let want = chosen(&pages);
                let now = state(&pages, &want);
                if now != said { said = now.clone(); if tx.send(now).is_err() { return; } }
                if want != page { break; }
            }
        }
        let _ = ws.close(None);
        nap(250);
    }
}
/// Whether another page than the one being shown has been asked for.
fn chosen_changed(w: &Watch, page: &str) -> bool { w.pick.lock().map(|p| !p.is_empty() && *p != page).unwrap_or(false) }
/// One thing a paired page asked for. Answers and what the sessions print go back through `tx`.
fn remote_op(text: &str, device: &str, tx: &std::sync::mpsc::Sender<String>, terms: &TermMap, watching: &Watching) {
    let Ok(m) = serde_json::from_str::<Value>(text) else { return };
    let s = |k: &str| m[k].as_str().unwrap_or("").to_string();
    let n = |k: &str, d: u64| m[k].as_u64().unwrap_or(d).min(500) as u16;
    let say = |v: Value| { let _ = tx.send(v.to_string()); };
    let op = s("op");
    match op.as_str() {
        "list" => {
            let tx = tx.clone();
            std::thread::spawn(move || { let _ = tx.send(remote_list().to_string()); });
        }
        // this device asks to be told (it gives where and the keys to seal messages for it), or asks not to be
        "push" => {
            if m["off"].as_bool().unwrap_or(false) { return push_forget(device); }
            let sub = &m["sub"];
            let fits = |k: &str| sub["keys"][k].as_str().map(|v| (16..=128).contains(&v.len())).unwrap_or(false);
            if sub["endpoint"].as_str().and_then(push_host).is_some() && sub["endpoint"].as_str().map(|e| e.len() < 1024).unwrap_or(false) && fits("p256dh") && fits("auth") {
                push_keep(device, Some(json!({ "endpoint": sub["endpoint"], "keys": { "p256dh": sub["keys"]["p256dh"], "auth": sub["keys"]["auth"] } })));
            } else { say(json!({ "op": "err", "text": "this browser's notification service is not one Maestro sends to" })); }
        }
        // watch the browser of a session (one at a time for a page), another of its pages, or no longer
        "browser" => {
            use std::sync::atomic::Ordering;
            let name = s("name");
            let Ok(mut held) = watching.lock() else { return };
            if let Some(w) = held.as_ref().filter(|w| w.name == name && !name.is_empty()) {
                if let Ok(mut pick) = w.pick.lock() { *pick = s("page"); }
                return;
            }
            if let Some(w) = held.take() { w.stop.store(true, Ordering::Relaxed); }
            if !ours(&name) { return; }
            let w = std::sync::Arc::new(Watch { name, stop: Default::default(), seen: Default::default(), pick: Mutex::new(s("page")) });
            *held = Some(w.clone());
            let (tx, width) = (tx.clone(), m["width"].as_u64().unwrap_or(1200).clamp(320, 2000) as u32);
            std::thread::spawn(move || browser_watch(width, w, tx));
        }
        "seen" => {
            if let Some(w) = watching.lock().ok().and_then(|h| h.clone()) { w.seen.fetch_max(m["n"].as_u64().unwrap_or(0), std::sync::atomic::Ordering::Relaxed); }
        }
        "chat" => {
            let (tx, name, after) = (tx.clone(), s("name"), s("after"));
            std::thread::spawn(move || { let _ = tx.send(remote_chat(&name, &after).to_string()); });
        }
        "open" | "new" => {
            let id = s("id");
            if id.is_empty() || id.len() > 40 || terms.lock().map(|t| t.len() >= 12 || t.contains_key(&id)).unwrap_or(true) { return; }
            let made = (|| -> Result<String, String> {
                let first = Some(s("prompt")).filter(|t| !t.trim().is_empty());
                let (p, kind, mode, name) = if op == "open" {
                    (remote_side().ok_or("no project here keeps its sessions running")?, "claude".to_string(), "attach".to_string(), Some(s("name")))
                } else {
                    let kind = s("kind");
                    if kind != "claude" && kind != "ubuntu" { return Err("from here a session is Claude Code or a shell".into()); }
                    let i = m["proj"].as_u64().map(|i| i as usize).filter(|i| *i < config()["projects"].as_array().map(|a| a.len()).unwrap_or(0)).ok_or("that project is not in the list")?;
                    let p = project_at(i)?;
                    if p.host() == Host::Direct { return Err("this project's sessions end with the app: start it at the computer".into()); }
                    (p, kind, "new".to_string(), None)
                };
                let label = first.clone().unwrap_or_else(|| if kind == "claude" { "Claude".into() } else { "Shell".into() });
                let (cmd, made) = session_command(p, kind, mode, name, None, first, Some(label))?;
                let (tx, shown) = (tx.clone(), id.clone());
                spawn_term(terms, id.clone(), cmd, n("cols", 80), n("rows", 24), move |data| {
                    let _ = tx.send(match data { Some(t) => json!({ "op": "out", "id": shown, "data": t }), None => json!({ "op": "exit", "id": shown }) }.to_string());
                })?;
                Ok(made)
            })();
            match made {
                Ok(name) => {
                    if op == "new" { if let Some(app) = REMOTE_APP.get() { let _ = app.emit("remote", json!({ "what": "session", "name": name })); } }
                    say(json!({ "op": "opened", "id": id, "name": name }))
                }
                Err(e) => say(json!({ "op": "err", "id": id, "text": e })),
            }
        }
        "in" => {
            if let Ok(mut map) = terms.lock() {
                if let Some(t) = map.get_mut(&s("id")) { let _ = std::io::Write::write_all(&mut t.writer, s("data").as_bytes()).and_then(|_| std::io::Write::flush(&mut t.writer)); }
            }
        }
        "size" => {
            if let Ok(map) = terms.lock() {
                if let Some(t) = map.get(&s("id")) { let _ = t.master.resize(portable_pty::PtySize { rows: n("rows", 24).max(4), cols: n("cols", 80).max(10), pixel_width: 0, pixel_height: 0 }); }
            }
        }
        "close" => {
            if let Some(mut t) = terms.lock().ok().and_then(|mut map| map.remove(&s("id"))) { let _ = t.child.kill(); }
        }
        // without opening it: type a line into a session, or end it
        "say" | "end" => {
            let (name, line, enter) = (s("name"), s("text"), m["enter"].as_bool().unwrap_or(true));
            if !ours(&name) || line.len() > 4000 { return; }
            let tx = tx.clone();
            std::thread::spawn(move || {
                if let Some((p, l)) = remote_side().and_then(|p| launcher(&p).ok().map(|l| (p, l))) {
                    let _ = if op == "end" { p.bash_command(false, &[&l, "--kill", &name]).output() } else { p.bash_command(false, &[&l, "--say", &name, &line, if enter { "enter" } else { "stay" }]).output() };
                }
                let _ = tx.send(remote_list().to_string());
            });
        }
        _ => {}
    }
}
/// Windows Firewall decides per kind of network, and Tailscale's adapter is a private one: the rule Windows offers to make
/// when a program first listens often covers only the network you are on. So Maestro has a rule of its own, by name: this
/// program, this port, from Tailscale's addresses only.
const FIREWALL_RULE: &str = "Maestro remote access";
/// Whether that rule is in place: "ok", "missing", or "blocked" when a rule that blocks this program stands in the way
/// (Windows makes one when its question is answered with Cancel). `None` where there is no Windows Firewall.
fn remote_firewall(port: u16) -> Option<String> {
    if !cfg!(windows) { return None; }
    static OK: Mutex<Option<u16>> = Mutex::new(None);
    if OK.lock().ok().map(|o| *o == Some(port)).unwrap_or(false) { return Some("ok".into()); }
    let exe = std::env::current_exe().ok()?.to_string_lossy().replace('\'', "''");
    let script = format!("$me = '{exe}'; $r = Get-NetFirewallRule -DisplayName '{FIREWALL_RULE}' -ErrorAction SilentlyContinue | Where-Object {{ $_.Enabled -eq 'True' -and $_.Action -eq 'Allow' -and $_.Direction -eq 'Inbound' }} | Where-Object {{ ($_ | Get-NetFirewallApplicationFilter).Program -eq $me -and ($_ | Get-NetFirewallPortFilter).LocalPort -eq '{port}' }}; $b = Get-NetFirewallApplicationFilter -Program $me -ErrorAction SilentlyContinue | Get-NetFirewallRule | Where-Object {{ $_.Enabled -eq 'True' -and $_.Action -eq 'Block' -and $_.Direction -eq 'Inbound' }}; if ($b) {{ 'blocked' }} elseif ($r) {{ 'ok' }} else {{ 'missing' }}");
    let mut c = std::process::Command::new("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    let out = quiet(c).output().ok()?;
    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if said == "ok" { if let Ok(mut o) = OK.lock() { *o = Some(port); } }
    Some(if said == "ok" || said == "blocked" { said } else { "missing".into() })
}
/// The script that puts the rule in place, run with administrator rights (Windows asks). It replaces an earlier rule of
/// this name and takes away rules that block this program from being reached.
fn firewall_script(exe: &str, port: u16) -> String {
    let exe = exe.replace('\'', "''");
    format!("$me = '{exe}'; Get-NetFirewallRule -DisplayName '{FIREWALL_RULE}' -ErrorAction SilentlyContinue | Remove-NetFirewallRule; Get-NetFirewallApplicationFilter -Program $me -ErrorAction SilentlyContinue | Get-NetFirewallRule | Where-Object {{ $_.Action -eq 'Block' -and $_.Direction -eq 'Inbound' }} | Remove-NetFirewallRule; New-NetFirewallRule -DisplayName '{FIREWALL_RULE}' -Description 'Lets devices of your Tailscale network reach Maestro. Made by Maestro; remove it freely.' -Direction Inbound -Action Allow -Program $me -Protocol TCP -LocalPort {port} -RemoteAddress 100.64.0.0/10 -Profile Any | Out-Null")
}
/// Put the firewall rule in place. Windows asks for permission; saying no changes nothing.
#[tauri::command(async)]
fn remote_allow() -> Result<Value, String> {
    use base64::Engine;
    let port = REMOTE.lock().ok().and_then(|r| r.as_ref().map(|r| r.port)).ok_or("turn remote access on first")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?.to_string_lossy().to_string();
    // handed over encoded, so that no quoting between the two shells can change what runs
    let wide: Vec<u8> = firewall_script(&exe, port).encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(wide);
    let mut c = std::process::Command::new("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-Command", &format!("Start-Process powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{encoded}'")]);
    let out = quiet(c).output().map_err(|e| e.to_string())?;
    match remote_firewall(port).as_deref() {
        Some("ok") => Ok(remote_state()),
        _ if !out.status.success() => Err("Windows was not given permission, so the firewall is as it was".into()),
        _ => Err("the firewall rule could not be made: see docs/remote.md for the rule to add by hand".into()),
    }
}
/// The pairing link as a code a phone's camera reads, drawn as SVG.
fn remote_qr(url: &str) -> String {
    qrcode::QrCode::new(url.as_bytes()).map(|q| q.render::<qrcode::render::svg::Color>().min_dimensions(220, 220).build()).unwrap_or_default()
}
/// What the Remote screen shows: whether it is on, whether Tailscale is up, the paired devices and which are connected now,
/// and, while a pairing code is live, the address that pairs a device (the code rides after the #, which a browser never
/// sends to a server), also as a code to scan.
#[tauri::command(async)]
fn remote_state() -> Value {
    let c = config();
    let port = REMOTE.lock().ok().and_then(|r| r.as_ref().map(|r| r.port));
    let ip = tailscale_ip();
    let live: Vec<String> = REMOTE_LIVE.lock().map(|l| l.iter().map(|x| x.device.clone()).collect()).unwrap_or_default();
    let now = now_secs();
    let devices: Vec<Value> = c["remote"]["devices"].as_array().map(|a| a.iter().map(|d| {
        let id = d["id"].as_str().unwrap_or("");
        json!({ "id": id, "name": d["name"], "added": d["added"], "seen": d["seen"], "live": live.iter().filter(|x| x.as_str() == id).count(), "expired": now.saturating_sub(d["seen"].as_u64().unwrap_or(0)) >= DEVICE_IDLE_SECS })
    }).collect()).unwrap_or_default();
    let code = REMOTE_PAIR.lock().ok().and_then(|p| p.as_ref().filter(|(_, at)| at.elapsed().as_secs() < PAIR_SECS).map(|(c, at)| (c.clone(), PAIR_SECS - at.elapsed().as_secs())));
    let (secure_name, secure_on) = port.map(|p| remote_secure_state(p, false)).unwrap_or_default();
    let pair = match (port, ip, code) {
        (Some(port), Some(ip), Some((code, left))) => { let url = if secure_on { format!("https://{secure_name}/#{code}") } else { format!("http://{ip}:{port}/#{code}") }; json!({ "qr": remote_qr(&url), "url": url, "left": left }) }
        _ => Value::Null,
    };
    let firewall = port.and_then(remote_firewall);
    json!({ "on": c["remote"]["on"].as_bool().unwrap_or(false), "running": port.is_some(), "port": port.unwrap_or(REMOTE_PORT), "tailscale": ip.is_some(), "firewall": firewall, "devices": devices, "pair": pair, "secure": { "on": secure_on, "name": secure_name } })
}
/// Turn the secure address on or off: ask Tailscale to answer https://<this machine's name> and hand it to Maestro. The first
/// time, Tailscale wants this allowed for your network and prints where to do that: that text is given back as `note`.
#[tauri::command(async)]
fn remote_secure(on: bool) -> Result<Value, String> {
    use std::io::Read;
    let port = REMOTE.lock().ok().and_then(|r| r.as_ref().map(|r| r.port)).ok_or("turn remote access on first")?;
    let exe = if cfg!(windows) { let f = PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into())).join("Tailscale").join("tailscale.exe"); if f.is_file() { f } else { PathBuf::from("tailscale") } } else { ["/Applications/Tailscale.app/Contents/MacOS/Tailscale", "/usr/local/bin/tailscale", "/opt/homebrew/bin/tailscale", "/usr/bin/tailscale"].iter().map(PathBuf::from).find(|f| f.is_file()).unwrap_or_else(|| PathBuf::from("tailscale")) };
    let mut c = std::process::Command::new(exe);
    if on { c.args(["serve", "--bg", "--https=443", &format!("http://127.0.0.1:{port}")]); } else { c.args(["serve", "--https=443", "off"]); }
    c.env("TAILSCALE_BE_CLI", "1");
    let mut child = quiet(c).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| format!("Tailscale could not be started: {e}"))?;
    // it waits, without end, for the permission it asks for: give it a few seconds, then read what it said
    let began = std::time::Instant::now();
    let done = loop {
        match child.try_wait() { Ok(Some(_)) => break true, Ok(None) if began.elapsed() > std::time::Duration::from_secs(8) => { let _ = child.kill(); let _ = child.wait(); break false; } Ok(None) => std::thread::sleep(std::time::Duration::from_millis(150)), Err(_) => break true }
    };
    let mut said = String::new();
    if let Some(mut o) = child.stdout.take() { let _ = o.read_to_string(&mut said); }
    if let Some(mut e) = child.stderr.take() { let mut t = String::new(); let _ = e.read_to_string(&mut t); said.push_str(&t); }
    let (_, now_on) = remote_secure_state(port, true);
    let mut state = remote_state();
    if on && !now_on { state["secure"]["note"] = json!(if done && said.trim().is_empty() { "Tailscale did not set the secure address up.".to_string() } else { said.split_whitespace().collect::<Vec<_>>().join(" ") }); }
    Ok(state)
}
/// Turn remote access on or off (kept in maestro.config.json).
#[tauri::command(async)]
fn remote_set(on: bool) -> Result<Value, String> {
    {
        let _one = REMOTE_CONFIG.lock().map_err(|e| e.to_string())?;
        let mut c = config();
        if !c["remote"].is_object() { c["remote"] = json!({}); }
        c["remote"]["on"] = json!(on);
        save_config(&c)?;
    }
    if on { remote_start()?; } else { remote_stop(); }
    Ok(remote_state())
}
/// Make a pairing code: one device, within ten minutes. A code made earlier stops working.
#[tauri::command(async)]
fn remote_pair() -> Result<Value, String> {
    if REMOTE.lock().map_err(|e| e.to_string())?.is_none() { return Err("turn remote access on first".into()); }
    remote_pair_code()?;
    Ok(remote_state())
}
/// Remove one paired device (or, with no id, all of them): its token stops working and its connections are cut now.
#[tauri::command(async)]
fn remote_forget(id: Option<String>) -> Result<Value, String> {
    let gone: Vec<(String, String)> = {
        let _one = REMOTE_CONFIG.lock().map_err(|e| e.to_string())?;
        let mut c = config();
        let Some(list) = c["remote"]["devices"].as_array_mut() else { return Ok(remote_state()) };
        let hit = |d: &Value| id.as_deref().map(|i| d["id"] == i).unwrap_or(true);
        let gone = list.iter().filter(|d| hit(d)).map(|d| (d["id"].as_str().unwrap_or("").to_string(), d["name"].as_str().unwrap_or("A device").to_string())).collect();
        list.retain(|d| !hit(d));
        save_config(&c)?;
        gone
    };
    if let Ok(live) = REMOTE_LIVE.lock() {
        for l in live.iter().filter(|l| gone.iter().any(|(i, _)| *i == l.device)) { l.cut.store(true, std::sync::atomic::Ordering::Relaxed); }
    }
    for (_, name) in &gone { remote_tell("removed", name, None); }
    Ok(remote_state())
}
/// `maestro --remote`: no window, only the remote page, until the program is stopped. For a machine you leave running.
/// It prints a link that pairs one device within ten minutes; start it again for another.
fn remote_only() {
    match remote_start().and_then(|port| remote_pair_code().map(|code| (port, code))) {
        Ok((port, code)) => {
            let ip = tailscale_ip();
            println!("Maestro remote, pairing link (one device, ten minutes): http://{}:{port}/#{code}", ip.map(|i| i.to_string()).unwrap_or_else(|| "127.0.0.1".into()));
            if ip.is_none() { println!("Tailscale is not running here: only this machine itself is answered."); }
            match remote_firewall(port).as_deref() {
                Some("ok") | None => {}
                Some(f) => println!("Windows Firewall: the rule \"{FIREWALL_RULE}\" is {f}, so other devices may not get through. As administrator:\n  {}", firewall_script(&std::env::current_exe().map(|e| e.to_string_lossy().to_string()).unwrap_or_default(), port)),
            }
            loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
        }
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--remote") { return remote_only(); }
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
            // a desktop without a tray (some Linux ones) must not stop the app: the key and the window still work
            let built = tray.on_menu_event(|app, event| match event.id.as_ref() {
                "show" => reveal(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                    toggle(tray.app_handle());
                }
            })
            .build(app);
            if let Err(e) = built { note(format!("no tray icon: {e}")); }

            // what an update left behind: the program it replaced
            if let Some(here) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
                for name in ["maestro.exe.old", "Maestro.exe.old", "maestro.old", "WebView2Loader.dll.old"] { let _ = fs::remove_file(here.join(name)); }
            }
            let _ = REMOTE_APP.set(app.handle().clone());
            if config()["remote"]["on"].as_bool().unwrap_or(false) {
                if let Err(e) = remote_start() { note(format!("remote access could not start: {e}")); }
            }
            // started by Windows at sign-in: stay in the tray until the key is pressed
            if !std::env::args().any(|a| a == "--hidden") {
                reveal(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load, read_log, read_audit, read_memory, save_memory, new_memory, read_skill, save_skill, new_skill, save_law, move_law, retire_law, ledger, mark_live, close_rows, audit, end_session, git_repos, git_status, git_log, git_commit, git_diff, git_tree, git_file, projects, set_project, inspect_project, add_project, machine, starter_plan, starter_apply, term_check, update_check, update_state, update_auto_set, update_page, update_install, remove_project, set_favorite, pick_folder, autostart_get, autostart_set, term_open, term_list, term_write, term_resize, term_close, term_label, pop_out, pop_view, set_on_top, attention, toggle_max, note, clip_set, clip_get, minimize, hide, quit, remote_state, remote_set, remote_pair, remote_forget, remote_allow, remote_secure,
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
    /// What GitHub answers right now (not run by default: it uses the network).   cargo test github -- --ignored --nocapture
    #[test]
    #[ignore]
    fn github_is_asked_for_the_latest_release() {
        match latest_release() {
            Ok(v) => println!("latest: {} newer: {} asset: {}", v["latest"], v["newer"], v["asset"]),
            Err(e) => println!("answer: {e}"),
        }
    }
    /// Does a paste survive the trip? A program inside WSL turns bracketed paste on and echoes what it is sent; this prints
    /// whether the request reaches the terminal's side and whether the paste marks reach the program.   cargo test paste -- --ignored --nocapture
    #[test]
    #[ignore]
    fn a_bracketed_paste_through_the_windows_terminal_layer() {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};
        use std::io::{Read, Write};
        let pair = native_pty_system().openpty(PtySize { rows: 24, cols: 100, pixel_width: 0, pixel_height: 0 }).unwrap();
        let mut cmd = CommandBuilder::new("wsl.exe");
        if std::env::var("PASTE_VIA_TMUX").is_ok() { cmd.args(["--exec", "bash", "/tmp/mp-outer.sh"]); } else { cmd.args(["--exec", "bash", "-c", "printf '\\033[?2004h'; stty raw -echo; echo READY; head -c 30 | cat -v; echo; echo DONE"]); }
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || { let mut b = [0u8; 4096]; while let Ok(n) = reader.read(&mut b) { if n == 0 || tx.send(b[..n].to_vec()).is_err() { break; } } });
        let mut seen = String::new();
        let mut sent = false;
        let start = std::time::Instant::now();
        while start.elapsed().as_secs() < 12 {
            if let Ok(chunk) = rx.recv_timeout(std::time::Duration::from_millis(300)) { seen.push_str(&String::from_utf8_lossy(&chunk)); }
            if !sent && seen.contains("READY") { writer.write_all(b"\x1b[200~line one\rline two\x1b[201~....").unwrap(); writer.flush().unwrap(); sent = true; }
            if seen.contains("DONE") { break; }
        }
        let _ = child.kill();
        println!("the path: {}", if std::env::var("PASTE_VIA_TMUX").is_ok() { "through tmux" } else { "straight into WSL" });
        println!("the terminal side was asked for bracketed paste: {}", seen.contains("\x1b[?2004h"));
        println!("the program received: {:?}", seen.split("READY").nth(1).unwrap_or("").replace('\x1b', "<ESC>"));
    }
    /// On macOS and Linux a project is its folder on this machine, and a session's scripts are written to this user's home.
    #[test]
    #[cfg(unix)]
    fn a_local_project_is_worked_out_and_its_sessions_are_listed() {
        let d = scratch("local");
        fs::write(d.join("AGENTS.md"), "# x\n").unwrap();
        fs::create_dir_all(d.join(".agents").join("rules")).unwrap();
        let v = derive(&d.to_string_lossy(), None).unwrap();
        assert_eq!(v["entry"]["runs"], "native");
        assert_eq!(v["entry"]["wslRoot"], "");
        assert!(v["entry"]["claudeHome"].as_str().unwrap().ends_with("/.claude"));
        let root = v["entry"]["root"].as_str().unwrap().to_string();
        assert!(v["entry"]["memoryDir"].as_str().unwrap().ends_with(&format!("/projects/{}/memory", dashed(&root))));
        assert_eq!((v["found"]["dir"].as_str(), v["found"]["instructionsName"].as_str()), (Some(".agents"), Some("AGENTS.md")));
        assert!(derive("relative/path", None).is_err() && derive("/no/such/folder/here", None).is_err());
        let p = Project { name: "x".into(), root: PathBuf::from(&root), claude_home: home_dir().unwrap().join(".claude"), memory: d.join("memory"), wsl_root: String::new(), distro: String::new(), kit: String::new(), audits: vec![], native: false, dot: d.join(".agents"), instructions: d.join("AGENTS.md") };
        assert_eq!(p.host(), Host::Local);
        let l = launcher(&p).unwrap();
        assert!(l.ends_with("/.local/share/maestro/term.sh") && Path::new(&l).exists());
        let out = p.bash_command(false, &[&l, "--check"]).output().unwrap();
        assert!(out.status.success(), "term.sh --check failed: {}", String::from_utf8_lossy(&out.stderr));
        assert!(p.bash_command(false, &[&l, "--list"]).output().unwrap().status.success());
        assert!(p.bash_command(false, &[&l, "--nonsense"]).output().unwrap().status.code() == Some(2));
        let _ = fs::remove_dir_all(&d);
    }
    /// Remote access answers this machine itself and what arrives on its own Tailscale address, nothing else.
    #[test]
    fn remote_answers_only_what_arrives_over_tailscale() {
        let ip = |t: &str| t.parse::<std::net::IpAddr>().unwrap();
        let ts = Some("100.101.102.103".parse::<std::net::Ipv4Addr>().unwrap());
        // this machine to itself, with or without Tailscale
        assert!(remote_gate(ip("127.0.0.1"), ip("127.0.0.1"), None));
        assert!(remote_gate(ip("::1"), ip("::1"), ts));
        // another device of the Tailscale network, arriving on this machine's Tailscale address
        assert!(remote_gate(ip("100.64.0.9"), ip("100.101.102.103"), ts));
        assert!(remote_gate(ip("::ffff:100.64.0.9"), ip("::ffff:100.101.102.103"), ts));
        // the same range handed out by an internet provider: it arrived on another address of this machine
        assert!(!remote_gate(ip("100.64.0.9"), ip("100.64.0.20"), ts));
        assert!(!remote_gate(ip("100.64.0.9"), ip("100.64.0.20"), None));
        assert!(!remote_gate(ip("100.64.0.9"), ip("100.101.102.103"), None));
        // the home network, the internet, an address just outside the range, and a claim to be this machine from elsewhere
        for no in ["192.168.0.10", "10.0.0.2", "8.8.8.8", "100.63.255.255", "100.128.0.1", "fd7a:115c:a1e0::1"] {
            assert!(!remote_gate(ip(no), ip("100.101.102.103"), ts), "{no}");
            assert!(!remote_gate(ip(no), ip("192.168.0.5"), ts), "{no}");
        }
        assert!(!remote_gate(ip("127.0.0.1"), ip("192.168.0.5"), ts));
        assert!(!remote_gate(ip("192.168.0.10"), ip("127.0.0.1"), ts));
    }
    /// A pairing code works while it is fresh; a device is known by its token's hash, until it has stayed away too long.
    #[test]
    fn remote_codes_expire_and_devices_are_known_by_hash() {
        let now = std::time::Instant::now();
        assert!(pair_ok(&Some(("abcd".into(), now)), "abcd"));
        assert!(!pair_ok(&Some(("abcd".into(), now)), "abce"));
        assert!(!pair_ok(&None, "abcd") && !pair_ok(&Some(("abcd".into(), now)), ""));
        if let Some(old) = now.checked_sub(std::time::Duration::from_secs(PAIR_SECS + 1)) { assert!(!pair_ok(&Some(("abcd".into(), old)), "abcd")); }
        assert!(same_token("a5f9", "a5f9") && !same_token("a5f9", "a5f8") && !same_token("a5f9", "a5f") && !same_token("", "a5f9"));
        assert_eq!(token_hash("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let token = "0123456789abcdef0123456789abcdef";
        let devices = json!([{ "id": "aa", "name": "Phone", "hash": token_hash(token), "seen": 1_000_000 }, { "id": "bb", "name": "Laptop", "hash": token_hash("ffffffffffffffffffffffffffffffff"), "seen": 1_000_000 }]);
        assert_eq!(device_of(&devices, token, 1_000_500), Some(("aa".into(), "Phone".into())));
        assert_eq!(device_of(&devices, "0123456789abcdef0123456789abcdee", 1_000_500), None);
        assert_eq!(device_of(&devices, &token_hash(token), 1_000_500), None); // the hash in the file is not a way in
        assert_eq!(device_of(&devices, token, 1_000_000 + DEVICE_IDLE_SECS), None);
        assert_eq!(device_of(&devices, "", 1_000_500), None);
        assert_eq!(device_of(&json!(null), token, 1_000_500), None);
        assert_eq!(tailnet_ip_in("100.121.112.23\r\n"), Some("100.121.112.23".parse().unwrap()));
        assert_eq!(tailnet_ip_in("Tailscale is stopped.\n"), None);
        assert_eq!(tailnet_ip_in("192.168.0.6\n"), None);
        assert_eq!(tailnet_ip_in(""), None);
        let script = firewall_script("D:\\Apps\\O'Brien\\Maestro.exe", 4747);
        assert!(script.contains("$me = 'D:\\Apps\\O''Brien\\Maestro.exe';") && script.contains("-LocalPort 4747 -RemoteAddress 100.64.0.0/10"));
        let code = remote_qr("http://100.64.0.1:4747/#0123456789abcdef0123456789abcdef");
        assert!(code.contains("<svg") && code.contains("</svg>") && !code.contains("<script"));
    }
    /// A conversation file becomes what was said; the file that fits a screen is the one whose words are on it.
    #[test]
    fn a_conversation_is_read_and_matched_to_a_screen() {
        let file = [
            r#"{"type":"mode","mode":"x"}"#,
            r#"{"type":"user","uuid":"u1","message":{"role":"user","content":"Redraw the tide curve from at most 288 points"}}"#,
            r#"{"type":"user","uuid":"u0","isMeta":true,"message":{"role":"user","content":"a note of the program"}}"#,
            r#"{"type":"user","uuid":"u2","message":{"role":"user","content":"<system-reminder>not for showing</system-reminder>"}}"#,
            r#"{"type":"assistant","uuid":"a1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"I will **smooth** it."},{"type":"tool_use","name":"Bash","input":{"command":"npm test -- TideChart"}}]}}"#,
            r#"{"type":"user","uuid":"u3","message":{"role":"user","content":[{"type":"tool_result","content":"14 passed"}]}}"#,
            r#"{"type":"assistant","uuid":"a2","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"a side conversation"}]}}"#,
            "not json",
        ].join("\n");
        let items = chat_items(&file);
        assert_eq!(items.iter().map(|i| (i.0.as_str(), i.1, i.2.as_str())).collect::<Vec<_>>(), vec![
            ("u1", "user", "Redraw the tide curve from at most 288 points"), ("a1-1", "assistant", "I will **smooth** it."), ("a1-2", "tool", "Bash npm test -- TideChart"),
        ]);
        let screen = squeeze("> Redraw the tide curve from at\n  most 288 points\n● I will smooth it.");
        assert_eq!(chat_fit(&items, &screen), 2); // the question, wrapped by the terminal, and the answer without its marks
        assert_eq!(chat_fit(&items, &squeeze("> Something else entirely was asked here")), 0);
        assert_eq!(squeeze("Héllo,\n  Wörld 42!"), "héllowörld42");
    }
    /// A sealed notification opens again with the device's own keys, and only with them; a screen is read as the pages read it.
    #[test]
    fn a_notification_is_sealed_for_one_device_and_a_screen_is_read() {
        use aes_gcm::aead::Aead;
        use aes_gcm::KeyInit;
        let device = p256::SecretKey::from_slice(&[7u8; 32]).unwrap();
        let mine = p256::SecretKey::from_slice(&[9u8; 32]).unwrap();
        let (auth, salt) = ([3u8; 16], [5u8; 16]);
        let sealed = push_seal(&public_bytes(&device), &auth, b"{\"title\":\"Tide curve\"}", &mine, &salt).unwrap();
        // the header: the salt, a record size, and this message's public key
        assert_eq!((&sealed[..16], &sealed[16..20], sealed[20] as usize), (&salt[..], &4096u32.to_be_bytes()[..], 65));
        assert_eq!(&sealed[21..86], public_bytes(&mine).as_slice());
        // the device's side of it, as a browser does it
        let open = |key: &p256::SecretKey, auth: &[u8]| -> Option<Vec<u8>> {
            let theirs = p256::PublicKey::from_sec1_bytes(&sealed[21..86]).ok()?;
            let shared = p256::ecdh::diffie_hellman(key.to_nonzero_scalar(), theirs.as_affine());
            let mut ikm = [0u8; 32];
            hkdf::Hkdf::<sha2::Sha256>::new(Some(auth), shared.raw_secret_bytes()).expand(&[b"WebPush: info\0".as_slice(), &public_bytes(key), &sealed[21..86]].concat(), &mut ikm).ok()?;
            let prk = hkdf::Hkdf::<sha2::Sha256>::new(Some(&sealed[..16]), &ikm);
            let (mut k, mut n) = ([0u8; 16], [0u8; 12]);
            prk.expand(b"Content-Encoding: aes128gcm\0", &mut k).ok()?;
            prk.expand(b"Content-Encoding: nonce\0", &mut n).ok()?;
            aes_gcm::Aes128Gcm::new_from_slice(&k).ok()?.decrypt(aes_gcm::Nonce::from_slice(&n), &sealed[86..]).ok()
        };
        assert_eq!(open(&device, &auth).unwrap(), b"{\"title\":\"Tide curve\"}\x02");
        assert!(open(&p256::SecretKey::from_slice(&[8u8; 32]).unwrap(), &auth).is_none() && open(&device, &[4u8; 16]).is_none());
        // the pass: three parts, the middle one names the service and an end
        let pass = push_pass(&mine, "https://web.push.apple.com", 1_000);
        let parts: Vec<&str> = pass.split('.').collect();
        let claims: Value = { use base64::Engine; serde_json::from_slice(&b64u().decode(parts[1]).unwrap()).unwrap() };
        assert_eq!((parts.len(), claims["aud"].as_str(), claims["exp"].as_u64()), (3, Some("https://web.push.apple.com"), Some(1_000 + 12 * 3600)));
        assert_eq!({ use base64::Engine; b64u().decode(parts[2]).unwrap().len() }, 64);
        // only real notification services, only https
        for yes in ["https://web.push.apple.com/abc", "https://fcm.googleapis.com/fcm/send/x", "https://updates.push.services.mozilla.com/wpush/v2/x"] { assert!(push_host(yes).is_some(), "{yes}"); }
        for no in ["http://web.push.apple.com/abc", "https://127.0.0.1/x", "https://evil.example/push.apple.com", "https://push.apple.com.evil.example/x", "https://noreply@web.push.apple.com/x", "https://web.push.apple.com:8443/x", ""] { assert!(push_host(no).is_none(), "{no}"); }
        // what a screen says
        let ask = " Bash command\n   npm run build\n Do you want to proceed?\n \u{276f} 1. Yes\n   2. Yes, and don't ask again for npm run build\n   3. No, and tell Claude what to do differently\n";
        assert_eq!(screen_state(ask, true), ("needs", "Do you want to proceed?".to_string()));
        assert_eq!(screen_state(ask, false).0, "idle");
        assert_eq!(screen_state("\u{25cf} Editing src/TideChart.tsx\n\u{273b} Smoothing the curve\u{2026} (42s \u{00b7} \u{2193} 3.1k tokens \u{00b7} esc to interrupt)\n  \u{23f5}\u{23f5} auto mode on (shift+tab to cycle)", true), ("working", "Smoothing the curve\u{2026}".to_string()));
        assert_eq!(screen_state("\u{25cf} The curve now uses 288 points.\n  1. first step done\n  3. not a list of answers\n\u{276f} ", true).0, "idle");
        assert_eq!(screen_state("", true).0, "idle");
    }
    #[test]
    fn a_browser_lists_its_pages() {
        use std::io::{Read, Write};
        // a stand-in for Chrome's debugging port: one answer, with its length, on a connection that stays open
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut b = [0u8; 1024];
            let n = s.read(&mut b).unwrap();
            assert!(String::from_utf8_lossy(&b[..n]).starts_with("GET /json/list "));
            let body = r#"[{"id":"AB12","title":"Shop","type":"page","url":"https://shop.example/cart"},{"id":"W1","title":"worker","type":"service_worker","url":"https://shop.example/sw.js"},{"id":"D1","title":"DevTools","type":"page","url":"devtools://devtools/x"},{"id":"../x","title":"odd","type":"page","url":"https://odd.example/"}]"#;
            s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(400));
        });
        let began = std::time::Instant::now();
        let pages = browser_pages(port).unwrap();
        assert_eq!(pages, vec![("AB12".to_string(), "Shop".to_string(), "https://shop.example/cart".to_string())]);
        assert!(began.elapsed() < std::time::Duration::from_millis(350), "the answer's stated length was not used");
        t.join().unwrap();
        assert!(browser_pages(port).is_none());
    }
    #[test]
    fn remote_requests_are_read_strictly() {
        let (m, p, h) = remote_head(b"GET /xterm.js?v=1 HTTP/1.1\r\nHost: 100.64.0.1:4747\r\nSec-WebSocket-Key:  abc \r\n\r\n").unwrap();
        assert_eq!((m.as_str(), p.as_str()), ("GET", "/xterm.js"));
        assert!(h.contains(&("host".into(), "100.64.0.1:4747".into())) && h.contains(&("sec-websocket-key".into(), "abc".into())));
        for bad in [&b"GET /../maestro.config.json HTTP/1.1\r\n\r\n"[..], b"GET http://elsewhere/ HTTP/1.1\r\n\r\n", b"GET / HTTP/2\r\n\r\n", b"GET /\r\n\r\n", b"GET / HTTP/1.1 extra\r\n\r\n", b"\xff\xfe\r\n\r\n", b"\r\n\r\n", b""] {
            assert!(remote_head(bad).is_none(), "{:?}", String::from_utf8_lossy(bad));
        }
    }
    #[test]
    fn versions_are_compared_number_by_number() {
        assert!(newer("v0.10.1", "0.9.4") && newer("0.2.0", "0.1.9") && newer("1.0", "0.9.9"));
        assert!(!newer("v0.2.0", "0.2.0") && !newer("0.1.9", "0.2.0") && !newer("v0.2.0-rc1", "0.2.0") && !newer("junk", "0.1.0"));
    }
    /// A build is swapped in beside the old one, which is kept as .old; a download without the program changes nothing.
    #[test]
    fn an_update_is_swapped_in_and_a_bad_one_is_refused() {
        let (here, dl) = (scratch("swap-here"), scratch("swap-dl"));
        let exe = here.join(EXE);
        fs::write(&exe, b"old").unwrap();
        fs::write(here.join("WebView2Loader.dll"), b"old-dll").unwrap();
        assert!(swap_in(&dl, &exe).is_err()); // nothing downloaded
        fs::write(dl.join(EXE), b"tiny").unwrap();
        assert!(swap_in(&dl, &exe).is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        fs::write(dl.join(EXE), vec![7u8; 1_200_000]).unwrap();
        fs::write(dl.join("WebView2Loader.dll"), b"new-dll").unwrap();
        swap_in(&dl, &exe).unwrap();
        assert_eq!(fs::read(&exe).unwrap().len(), 1_200_000);
        assert_eq!(fs::read(here.join(format!("{EXE}.old"))).unwrap(), b"old");
        assert_eq!(fs::read(here.join("WebView2Loader.dll")).unwrap(), b"new-dll");
        assert_eq!(fs::read(here.join("WebView2Loader.dll.old")).unwrap(), b"old-dll");
        let _ = (fs::remove_dir_all(&here), fs::remove_dir_all(&dl));
    }
    #[test]
    fn git_paths_and_ids_are_checked() {
        if cfg!(windows) { assert_eq!(win_path("/mnt/d/projects/x/y"), PathBuf::from("D:\\projects\\x\\y")); } else { assert_eq!(win_path("/mnt/d/projects/x/y"), PathBuf::from("/mnt/d/projects/x/y")); }
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
