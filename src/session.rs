//! Persist unfinished questions locally, never inside the project repository.
use crate::coach::Question;
use crate::git::Evidence;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub repo: String,
    pub branch: String,
    pub mode: String,
    pub page: usize,
    pub evidence_hash: u64,
    pub next: usize,
    pub questions: Vec<Question>,
}

fn directory() -> Result<PathBuf, String> {
    let root = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }
    .ok_or("Не знайдено папку локального профілю.")?;
    Ok(root.join("codegym").join("sessions"))
}

fn location(repo: &Path, branch: &str, mode: &str) -> Result<PathBuf, String> {
    let mut hasher = DefaultHasher::new();
    repo.to_string_lossy().hash(&mut hasher);
    branch.hash(&mut hasher);
    mode.hash(&mut hasher);
    Ok(directory()?.join(format!("{:016x}.json", hasher.finish())))
}

pub fn evidence_signature(repo: &Path, evidence: &Evidence) -> Result<u64, String> {
    let mut hasher = DefaultHasher::new();
    evidence.context.hash(&mut hasher);
    evidence.branch.hash(&mut hasher);
    evidence.base.hash(&mut hasher);
    // Verify committed and uncommitted changes to avoid resuming stale questions.
    for args in [
        vec!["rev-parse", "HEAD"],
        vec!["status", "--porcelain", "-uno"],
    ] {
        let output = Command::new("git")
            .current_dir(repo)
            .args(&args)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() || output.stdout.len() > 1_000_000 {
            return Err("Не вдалося перевірити Git-стан для відновлення заняття.".into());
        }
        output.stdout.hash(&mut hasher);
    }
    Ok(hasher.finish())
}

impl Session {
    pub fn new(
        repo: &Path,
        branch: &str,
        mode: &str,
        page: usize,
        evidence_hash: u64,
        questions: Vec<Question>,
    ) -> Self {
        Self {
            repo: repo.to_string_lossy().into_owned(),
            branch: branch.to_owned(),
            mode: mode.to_owned(),
            page,
            evidence_hash,
            next: 0,
            questions,
        }
    }

    pub fn load(repo: &Path, branch: &str, mode: &str) -> Result<Option<Self>, String> {
        let file = location(repo, branch, mode)?;
        let content = match fs::read_to_string(&file) {
            Ok(value) => value,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(format!("Не можу прочитати заняття: {err}")),
        };
        if content.len() > 250_000 {
            return Err("Збережене заняття надто велике.".into());
        }
        let session: Session = serde_json::from_str(&content)
            .map_err(|err| format!("Пошкоджений локальний файл заняття: {err}"))?;
        if session.repo != repo.to_string_lossy()
            || session.branch != branch
            || session.mode != mode
        {
            return Ok(None);
        }
        Ok(Some(session))
    }

    pub fn matches(&self, page: usize, signature: u64) -> bool {
        self.page == page
            && self.evidence_hash == signature
            && !self.questions.is_empty()
            && self.next < self.questions.len()
            && self.questions.len() <= 5
    }

    pub fn save(&self, repo: &Path) -> Result<(), String> {
        let file = location(repo, &self.branch, &self.mode)?;
        let parent = file.parent().ok_or("Не знайдено каталог для заняття.")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let tmp = parent.join(format!("session-{}.tmp", std::process::id()));
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(&tmp, &file).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("Не вдалося оновити заняття: {e}")
        })
    }

    pub fn clear(&self, repo: &Path) -> Result<(), String> {
        Self::clear_for(repo, &self.branch, &self.mode)
    }

    /// Recover corrupted local sessions without first deserializing them.
    pub fn clear_for(repo: &Path, branch: &str, mode: &str) -> Result<(), String> {
        let file = location(repo, branch, mode)?;
        match fs::remove_file(file) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("Не можу очистити завершене заняття: {err}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_signature_check_and_serialization() {
        let questions = vec![Question {
            topic: "Логіка".into(),
            question: "Що поверне ця функція?".into(),
            source_path: "src/file.ts".into(),
            source_line: 3,
            reference_answer: "Число".into(),
            hint: "Перевір return".into(),
            practice: None,
        }];
        let repo = Path::new("/temp/repo");
        let mut session = Session::new(repo, "feature", "feature", 2, 12345, questions);
        assert!(session.matches(2, 12345));
        assert!(!session.matches(1, 12345));
        assert!(!session.matches(2, 12));
        session.next = 1;
        assert!(!session.matches(2, 12345));
        let json = serde_json::to_string(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.next, 1);
        assert_eq!(restored.questions[0].source_line, 3);
    }

    #[test]
    fn session_paths_are_separate_for_each_repo() {
        let a = location(Path::new("/one"), "main", "feature").unwrap();
        let b = location(Path::new("/two"), "main", "feature").unwrap();
        let c = location(Path::new("/one"), "feature", "feature").unwrap();
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn saves_updated_cursor_and_recovers_corrupt_state() {
        let unique = format!(
            "codegym-resume-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let repo = PathBuf::from(unique);
        let question = Question {
            topic: "Логіка".into(),
            question: "Що поверне?".into(),
            source_path: "example.ts".into(),
            source_line: 1,
            reference_answer: "Число".into(),
            hint: "Дивись return".into(),
            practice: None,
        };
        let mut session = Session::new(
            &repo,
            "branch",
            "feature",
            2,
            100,
            vec![question.clone(), question],
        );
        let result = (|| -> Result<(), String> {
            session.save(&repo)?;
            session.next = 1;
            session.save(&repo)?;
            let loaded = Session::load(&repo, "branch", "feature")?
                .ok_or("Session not found after saving")?;
            if loaded.next != 1 || !loaded.matches(2, 100) {
                return Err("Cursor was not persisted".into());
            }
            loaded.clear(&repo)?;
            if Session::load(&repo, "branch", "feature")?.is_some() {
                return Err("Session still exists after completion".into());
            }
            let file = location(&repo, "branch", "feature")?;
            fs::write(file, b"invalid-json").map_err(|e| e.to_string())?;
            if Session::load(&repo, "branch", "feature").is_ok() {
                return Err("Corrupt session was unexpectedly parsed".into());
            }
            Session::clear_for(&repo, "branch", "feature")?;
            Ok(())
        })();
        let _ = Session::clear_for(&repo, "branch", "feature");
        result.unwrap();
    }
}
