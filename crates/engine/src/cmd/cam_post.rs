//! Post processor commands: list the posts that turn toolpaths into G-code, save
//! user posts as JSON next to the tool libraries, and run one over an operation's
//! toolpath. The post model and the emitter live in `solidcraft-cam::post`; this
//! module is the hostile-parameter boundary and the file access.
use super::CommandSpec;
use crate::params::bad;
use crate::{Result, Session};
use serde::Deserialize;
use serde_json::{Value, json};
use solidcraft_cam::{EmitOptions, Post, Toolpath};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.post.list", "List Post Processors", list).icon("inspect").noundo().doc_only()
        .params("folder? (a folder of post JSON files; default SolidCraft's posts folder) → posts [{id, name, machine, builtin, file?}]"),
    CommandSpec::new("cam.post.save", "Save Post Processor", save).icon("export").noundo().doc_only()
        .params("post {id, name, machine?, absolute?, unit_line?, rapid?, feed?, arc_cw?, arc_ccw?, feed_word?, spindle?, header?, footer?}, folder? (default SolidCraft's posts folder) → id, path"),
    CommandSpec::new("cam.post.run", "Run Post Processor", run).icon("export").noundo().doc_only()
        .params("id? or file? (a post JSON; default the built-in ISO mill), folder?, operation? (operation id) or segments [segment, …], feed? mm/min (default 500), rpm? (default 8000), tool? (a comment line), out? (also write the program here) → gcode"),
];

/// The folder posts live in: the request's, or SolidCraft's data folder.
fn folder_arg(cmd: &str, p: &Value) -> Result<Option<std::path::PathBuf>> {
    match p.get("folder").and_then(Value::as_str) {
        Some(f) => {
            super::tooling::path(cmd, f)?;
            Ok(Some(std::path::PathBuf::from(f)))
        }
        None => Ok(crate::recovery::data_dir().map(|d| d.join("posts"))),
    }
}

/// Read and validate one post file.
fn read_post(cmd: &str, file: &str) -> Result<Post> {
    super::tooling::path(cmd, file)?;
    let len = solidcraft_io::vfs::len(file).map_err(|e| bad(cmd, format!("could not read the post: {e}")))?;
    if len > 1_048_576 {
        return Err(bad(cmd, "the post file exceeds 1 MiB"));
    }
    let bytes = solidcraft_io::vfs::read(file, 1_048_576).map_err(|e| bad(cmd, format!("could not read the post: {e}")))?;
    let post: Post = serde_json::from_slice(&bytes).map_err(|e| bad(cmd, format!("not a post file: {e}")))?;
    post.check().map_err(|e| bad(cmd, e.to_string()))?;
    Ok(post)
}

fn list(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.post.list";
    let folder = folder_arg(CMD, p)?;
    let _ = &folder;
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut out: Vec<Value> = solidcraft_cam::builtin_posts()
        .into_iter()
        .map(|b| json!({"id": b.id, "name": b.name, "machine": b.machine, "builtin": true}))
        .collect();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(dir) = folder {
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten().take(4096) {
                let fp = e.path();
                if fp.extension().and_then(|x| x.to_str()) == Some("json") {
                    files.push(fp);
                }
            }
        }
        files.sort();
        files.truncate(256);
        for fp in files {
            if fp.metadata().ok().filter(|m| m.is_file() && m.len() <= 1_048_576).is_none() {
                continue;
            }
            let Ok(bytes) = std::fs::read(&fp) else { continue };
            let Ok(post) = serde_json::from_slice::<Post>(&bytes) else { continue };
            if post.check().is_err() {
                continue;
            }
            out.push(json!({"id": post.id, "name": post.name, "machine": post.machine, "file": fp.to_string_lossy(), "builtin": false}));
        }
    }
    Ok(json!({"posts": out}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    post: Value,
    #[serde(default)]
    folder: Option<String>,
}

fn save(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.post.save";
    let r: Save = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    let post: Post = serde_json::from_value(r.post).map_err(|e| bad(CMD, format!("not a post: {e}")))?;
    post.check().map_err(|e| bad(CMD, e.to_string()))?;
    if cfg!(target_arch = "wasm32") {
        return Err(bad(CMD, "not supported yet: saving posts in the browser"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = match &r.folder {
            Some(f) => {
                super::tooling::path(CMD, f)?;
                std::path::PathBuf::from(f)
            }
            None => crate::recovery::data_dir().ok_or_else(|| bad(CMD, "no posts folder; pass folder"))?.join("posts"),
        };
        std::fs::create_dir_all(&dir).map_err(|e| bad(CMD, format!("could not create the posts folder: {e}")))?;
        let file = dir.join(format!("{}.json", post.id));
        let text = serde_json::to_string_pretty(&post).map_err(|e| bad(CMD, e.to_string()))?;
        solidcraft_io::write_atomic(&file, text.as_bytes(), true).map_err(|e| bad(CMD, format!("could not write the post: {e}")))?;
        Ok(json!({"id": post.id, "path": file.to_string_lossy()}))
    }
    #[cfg(target_arch = "wasm32")]
    {
        Err(bad(CMD, "not supported yet: saving posts in the browser"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Run {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    operation: Option<u64>,
    #[serde(default)]
    segments: Option<Value>,
    #[serde(default)]
    feed: Option<f64>,
    #[serde(default)]
    rpm: Option<f64>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    out: Option<String>,
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.post.run";
    let r: Run = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    let post = match (&r.file, &r.id) {
        (Some(f), _) => read_post(CMD, f)?,
        (None, Some(id)) => {
            if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
                return Err(bad(CMD, "unknown post id"));
            }
            solidcraft_cam::builtin_posts()
                .into_iter()
                .find(|b| &b.id == id)
                .or_else(|| {
                    let dir = match &r.folder {
                        Some(f) => std::path::PathBuf::from(f),
                        None => crate::recovery::data_dir()?,
                    };
                    read_post(CMD, &dir.join(format!("{id}.json")).to_string_lossy()).ok()
                })
                .ok_or_else(|| bad(CMD, format!("no post with id `{id}`")))?
        }
        (None, None) => Post::default(),
    };
    let path = match (r.operation, &r.segments) {
        (Some(id), None) => s
            .doc
            .cam
            .operations
            .iter()
            .find(|o| o.id == id)
            .and_then(|o| o.toolpath.clone())
            .ok_or_else(|| bad(CMD, "that operation has no toolpath"))?,
        (None, Some(v)) => {
            let t: Toolpath = serde_json::from_value(json!({"segments": v.clone()})).map_err(|e| bad(CMD, format!("bad segments: {e}")))?;
            t.check().map_err(|e| bad(CMD, e.to_string()))?;
            t
        }
        (Some(_), Some(_)) => return Err(bad(CMD, "give operation or segments, not both")),
        (None, None) => return Err(bad(CMD, "give operation or segments")),
    };
    let opts = EmitOptions { feed: r.feed.unwrap_or(500.0), rpm: r.rpm.unwrap_or(8000.0), tool: r.tool.clone() };
    let gcode = solidcraft_cam::emit(&post, &path, &opts).map_err(|e| bad(CMD, e.to_string()))?;
    let mut out = json!({"gcode": gcode});
    if let Some(file) = &r.out {
        super::tooling::path(CMD, file)?;
        solidcraft_io::vfs::write(file, gcode.as_bytes()).map_err(|e| bad(CMD, format!("could not write the program: {e}")))?;
        out["out"] = json!(file);
    }
    Ok(out)
}
