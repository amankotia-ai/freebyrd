//! Reads new activity from each source into the store. Every source keeps a
//! cursor (a byte offset or a row ID), so each run reads only what is new,
//! and every row has an ID from its source, so reading something twice adds
//! nothing. Message stores are read the same way in Littlebird
//! (`littlebird/FINDINGS.md` §1: a cursor database and deduplication).

use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::Result;

use crate::store::Store;

pub mod claude_code;
pub mod history;
pub mod pages;

#[derive(Debug, Default)]
pub struct SourceReport {
    pub name: &'static str,
    pub files: usize,
    pub added: usize,
    pub errors: Vec<String>,
}

impl SourceReport {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            ..Self::default()
        }
    }
}

impl fmt::Display for SourceReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} new from {} files",
            self.name, self.added, self.files
        )?;
        if !self.errors.is_empty() {
            write!(
                f,
                ", {} errors (first: {})",
                self.errors.len(),
                self.errors[0]
            )?;
        }
        Ok(())
    }
}

/// Reads every source once.
pub fn all(store: &Store) -> Vec<SourceReport> {
    vec![
        claude_code::ingest(store),
        history::ingest(store),
        pages::ingest(store),
    ]
}

/// Calls `each` with every complete line added to `path` since the last
/// run, then moves the cursor past them, all in one transaction. A file
/// replaced since (another inode, or shorter than the cursor) is read from
/// the start. A last line without its newline is still being written, and
/// waits for the next run.
fn read_new_lines(
    store: &Store,
    source: &str,
    path: &Path,
    mut each: impl FnMut(&str) -> Result<usize>,
) -> Result<usize> {
    let meta = fs::metadata(path)?;
    let identity = meta.ino().to_string();
    let key = path.to_string_lossy();
    let len = i64::try_from(meta.len())?;
    let mut position = match store.cursor(source, &key)? {
        Some((position, known)) if known == identity && position <= len => position,
        _ => 0,
    };
    if position == len {
        return Ok(0);
    }

    let mut reader = BufReader::new(File::open(path)?);
    reader.seek(SeekFrom::Start(u64::try_from(position)?))?;
    let tx = store.transaction()?;
    let mut added = 0;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 || line.last() != Some(&b'\n') {
            break;
        }
        position += i64::try_from(read)?;
        if let Ok(text) = std::str::from_utf8(&line) {
            added += each(text.trim_end())?;
        }
    }
    store.set_cursor(source, &key, position, &identity)?;
    tx.commit()?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn reads_only_new_complete_lines() {
        let dir = std::env::temp_dir().join(format!("freebyrd-lines-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.jsonl");
        let store = Store::open_in_memory().unwrap();
        let read = |store: &Store| {
            let mut lines = Vec::new();
            read_new_lines(store, "test", &path, |line| {
                lines.push(line.to_owned());
                Ok(1)
            })
            .unwrap();
            lines
        };

        fs::write(&path, "one\ntwo\npart").unwrap();
        assert_eq!(read(&store), ["one", "two"]);
        assert!(read(&store).is_empty());

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"ial\nthree\n").unwrap();
        assert_eq!(read(&store), ["partial", "three"]);

        // Replaced by a new file: read from the start.
        fs::remove_file(&path).unwrap();
        fs::write(&path, "fresh\n").unwrap();
        assert_eq!(read(&store), ["fresh"]);

        fs::remove_dir_all(&dir).unwrap();
    }
}
