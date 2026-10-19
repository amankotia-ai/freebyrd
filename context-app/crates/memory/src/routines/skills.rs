//! The skills the person already has, so Freebyrd suggests none that
//! exists: their own in `~/.claude/skills` (including the ones synced from
//! claude.ai), the skills and commands of their enabled plugins, and the
//! skills kept in their projects' `.claude/skills`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::detect;
use crate::one_line;
use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// What it is called: "push-to-github", or "vercel:deploy" for a
    /// plugin's.
    pub name: String,
    pub description: String,
}

/// Every skill the person has under `home`, and in the projects the memory
/// knows.
pub fn installed(store: &Store, home: &Path) -> Vec<Installed> {
    let mut found = Vec::new();
    collect(&home.join(".claude/skills"), None, 3, &mut found);
    for (plugin, path) in enabled_plugins(home) {
        collect(&path.join("skills"), Some(&plugin), 1, &mut found);
        commands(&path.join("commands"), &plugin, &mut found);
    }
    for root in project_roots(store) {
        collect(
            &Path::new(&root).join(".claude/skills"),
            None,
            1,
            &mut found,
        );
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found.dedup_by(|a, b| a.name == b.name);
    found
}

/// The installed skills that share the most words with `words`, best first.
pub fn relevant<'a>(
    installed: &'a [Installed],
    words: &[String],
    max: usize,
) -> Vec<&'a Installed> {
    let mut scored: Vec<(usize, &Installed)> = installed
        .iter()
        .filter_map(|skill| {
            let text = format!(
                "{} {}",
                skill.name.replace([':', '-', '_'], " "),
                skill.description
            );
            let theirs = detect::words(&text, 60);
            let score = words
                .iter()
                .filter(|word| theirs.iter().any(|t| detect::same_word(word, t)))
                .count();
            (score > 0).then_some((score, skill))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored
        .into_iter()
        .take(max)
        .map(|(_, skill)| skill)
        .collect()
}

/// Skills in `dir`: each folder with a SKILL.md, looked for up to `depth`
/// levels down (synced skills sit in a folder per account).
fn collect(dir: &Path, plugin: Option<&str>, depth: usize, found: &mut Vec<Installed>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(text) = fs::read_to_string(path.join("SKILL.md")) {
            let (name, description) = front_matter(&text);
            let name = name.unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
            found.push(Installed {
                name: match plugin {
                    Some(plugin) => format!("{plugin}:{name}"),
                    None => name,
                },
                description: one_line(&description.unwrap_or_default(), 300),
            });
        } else if depth > 1 && path.is_dir() {
            collect(&path, plugin, depth - 1, found);
        }
    }
}

/// A plugin's commands (`commands/<name>.md`), which also answer to a
/// slash command.
fn commands(dir: &Path, plugin: &str, found: &mut Vec<Installed>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let Some(stem) = path.file_stem() else {
            continue;
        };
        let text = fs::read_to_string(&path).unwrap_or_default();
        found.push(Installed {
            name: format!("{plugin}:{}", stem.to_string_lossy()),
            description: one_line(&front_matter(&text).1.unwrap_or_default(), 300),
        });
    }
}

/// The enabled plugins and their folders, from Claude Code's settings and
/// its list of installed plugins.
fn enabled_plugins(home: &Path) -> Vec<(String, PathBuf)> {
    let read = |path: PathBuf| {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
    };
    let (Some(settings), Some(installed)) = (
        read(home.join(".claude/settings.json")),
        read(home.join(".claude/plugins/installed_plugins.json")),
    ) else {
        return Vec::new();
    };
    let Some(plugins) = installed["plugins"].as_object() else {
        return Vec::new();
    };
    plugins
        .iter()
        .filter(|(key, _)| settings["enabledPlugins"][key.as_str()] == true)
        .filter_map(|(key, versions)| {
            let path = versions.as_array()?.first()?["installPath"].as_str()?;
            let name = key.split('@').next()?.to_owned();
            Some((name, PathBuf::from(path)))
        })
        .collect()
}

fn project_roots(store: &Store) -> Vec<String> {
    let Ok(mut statement) = store.conn().prepare("SELECT root FROM project") else {
        return Vec::new();
    };
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// `name` and `description` from a SKILL.md's front matter, including
/// folded ones (`description: >-` and indented lines after it).
pub fn front_matter(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let mut name = None;
    let mut description = None;
    let mut keep = |key: &str, value: String| {
        let value = value.trim().to_owned();
        if value.is_empty() {
            return;
        }
        match key {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => {}
        }
    };
    let mut folded: Option<(String, Vec<String>)> = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if let Some((_, parts)) = folded.as_mut()
            && (indented || line.trim().is_empty())
        {
            parts.push(line.trim().to_owned());
            continue;
        }
        if let Some((key, parts)) = folded.take() {
            keep(&key, parts.join(" "));
        }
        if indented {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if matches!(value, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
            folded = Some((key.trim().to_owned(), Vec::new()));
        } else {
            keep(key.trim(), unquote(value));
        }
    }
    if let Some((key, parts)) = folded {
        keep(&key, parts.join(" "));
    }
    (name, description)
}

fn unquote(value: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.replace("\\\"", "\"");
        }
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_front_matter() {
        let plain =
            "---\nname: push-to-github\ndescription: Commit and push your work.\n---\n# Body";
        assert_eq!(
            front_matter(plain),
            (
                Some("push-to-github".into()),
                Some("Commit and push your work.".into())
            )
        );
        let folded = "---\nname: \"panorama\"\ndescription: >-\n  The Panorama front door.\n  Use when the user types /panorama.\nmetadata:\n  version: \"3\"\n---\n";
        assert_eq!(
            front_matter(folded),
            (
                Some("panorama".into()),
                Some("The Panorama front door. Use when the user types /panorama.".into())
            )
        );
        assert_eq!(front_matter("# No front matter"), (None, None));
    }

    #[test]
    fn finds_the_persons_synced_plugin_and_project_skills() {
        let home = std::env::temp_dir().join(format!("freebyrd-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let skill = |dir: PathBuf, name: &str, description: &str| {
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {description}\n---\nBody"),
            )
            .unwrap();
        };
        skill(home.join(".claude/skills/mine"), "mine", "My own skill");
        skill(
            home.join(".claude/skills/synced/account/morning"),
            "morning",
            "Render the morning brief",
        );
        let plugin = home.join("plugins/vercel");
        skill(
            plugin.join("skills/deploy"),
            "deploy",
            "Deploy the current project to Vercel",
        );
        fs::create_dir_all(plugin.join("commands")).unwrap();
        fs::write(
            plugin.join("commands/logs.md"),
            "---\ndescription: View deployment logs\n---\n",
        )
        .unwrap();
        skill(
            home.join(".claude/plugins/off/skills/hidden"),
            "hidden",
            "Not enabled",
        );
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"enabledPlugins": {"vercel@market": true, "off@market": false}}"#,
        )
        .unwrap();
        fs::write(
            home.join(".claude/plugins/installed_plugins.json"),
            format!(
                r#"{{"plugins": {{"vercel@market": [{{"installPath": "{}"}}], "off@market": [{{"installPath": "{}"}}]}}}}"#,
                plugin.display(),
                home.join(".claude/plugins/off").display()
            ),
        )
        .unwrap();
        let project = home.join("repo");
        skill(
            project.join(".claude/skills/release"),
            "release",
            "Cut a release",
        );
        let store = Store::open_in_memory().unwrap();
        store
            .project_id(&project.display().to_string(), "repo")
            .unwrap();

        let all = installed(&store, &home);
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["mine", "morning", "release", "vercel:deploy", "vercel:logs"]
        );
        let words = detect::opening("commit and push this and also deploy on vercel");
        let best = relevant(&all, &words, 2);
        assert_eq!(best[0].name, "vercel:deploy");

        fs::remove_dir_all(&home).unwrap();
    }
}
