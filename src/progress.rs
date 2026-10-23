use crate::coach::Question;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

const DAY: u64 = 86_400;
const REVIEW_DAYS: [u64; 5] = [1, 3, 7, 14, 30];

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct TopicProgress {
    pub attempts: u32,
    pub total_points: u32,
    pub successful: u32,
    pub last_practiced: u64,
    #[serde(default)]
    pub correct_streak: u32,
    #[serde(default)]
    pub next_due: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCard {
    pub repo: String,
    pub branch: String,
    pub question: Question,
    /// Hash of code shown when the card was created; prevents stale reviews.
    pub source_hash: u64,
    pub next_due: u64,
    pub streak: u32,
    pub attempts: u32,
}

pub fn source_hash(code: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Store {
    pub version: u32,
    pub topics: BTreeMap<String, TopicProgress>,
    #[serde(default)]
    pub lesson_pages: BTreeMap<String, u32>,
    #[serde(default)]
    pub cards: BTreeMap<String, MemoryCard>,
}

fn data_file() -> Result<PathBuf, String> {
    let root = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".local/share")))
    }
    .ok_or_else(|| "Не можу визначити локальну папку для прогресу.".to_string())?;
    Ok(root.join("codegym").join("progress.json"))
}

impl Store {
    fn card_key(repo: &Path, branch: &str, q: &Question) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        repo.to_string_lossy().hash(&mut hasher);
        branch.hash(&mut hasher);
        q.source_path.hash(&mut hasher);
        q.source_line.hash(&mut hasher);
        q.question.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    pub fn remember_answer(
        &mut self,
        repo: &Path,
        branch: &str,
        question: &Question,
        snippet: &str,
        score: u8,
    ) {
        self.remember_at(repo, branch, question, snippet, score, now());
    }

    fn remember_at(
        &mut self,
        repo: &Path,
        branch: &str,
        question: &Question,
        snippet: &str,
        score: u8,
        timestamp: u64,
    ) {
        let key = Self::card_key(repo, branch, question);
        if score >= 4 && !self.cards.contains_key(&key) {
            return;
        }
        let card = self.cards.entry(key.clone()).or_insert_with(|| MemoryCard {
            repo: repo.to_string_lossy().into_owned(),
            branch: branch.to_owned(),
            question: question.clone(),
            source_hash: source_hash(snippet),
            next_due: timestamp,
            streak: 0,
            attempts: 0,
        });
        card.attempts = card.attempts.saturating_add(1);
        card.source_hash = source_hash(snippet);
        if score >= 4 {
            card.streak = card.streak.saturating_add(1);
            let days =
                REVIEW_DAYS[(card.streak.saturating_sub(1) as usize).min(REVIEW_DAYS.len() - 1)];
            card.next_due = timestamp.saturating_add(days * DAY);
            if card.streak >= 3 {
                self.cards.remove(&key);
            }
        } else {
            card.streak = 0;
            card.next_due = timestamp.saturating_add(if score <= 2 { 0 } else { DAY });
        }
    }

    pub fn due_cards(&self, repo: &Path, branch: &str) -> Vec<MemoryCard> {
        self.due_cards_at(repo, branch, now())
    }

    fn due_cards_at(&self, repo: &Path, branch: &str, timestamp: u64) -> Vec<MemoryCard> {
        let mut cards = self
            .cards
            .values()
            .filter(|card| {
                card.repo == repo.to_string_lossy()
                    && card.branch == branch
                    && card.next_due <= timestamp
            })
            .cloned()
            .collect::<Vec<_>>();
        cards.sort_by_key(|c| (c.next_due, c.attempts));
        cards.truncate(5);
        cards
    }
    fn lesson_key(repo: &Path, branch: &str, mode: &str) -> String {
        format!("{} | {branch} | {mode}", repo.display())
    }

    /// Next one-based lesson page. Only generated sessions advance it.
    pub fn lesson_page(&self, repo: &Path, branch: &str, mode: &str) -> usize {
        self.lesson_pages
            .get(&Self::lesson_key(repo, branch, mode))
            .copied()
            .unwrap_or(0) as usize
            + 1
    }

    pub fn advance_lesson(&mut self, repo: &Path, branch: &str, mode: &str) {
        let key = Self::lesson_key(repo, branch, mode);
        let count = self.lesson_pages.entry(key).or_default();
        *count = count.saturating_add(1);
    }

    pub fn weak_topics(&self, repo: &Path) -> Vec<String> {
        self.weak_topics_at(repo, now())
    }

    fn weak_topics_at(&self, repo: &Path, current: u64) -> Vec<String> {
        let prefix = format!("{} | ", repo.display());
        let mut topics = self
            .topics
            .iter()
            .filter(|(key, value)| key.starts_with(&prefix) && value.attempts > 0)
            .filter_map(|(key, value)| {
                let average = value.total_points as f64 / value.attempts as f64;
                let due = value.next_due != 0 && value.next_due <= current;
                (average < 3.5 || due).then(|| {
                    (
                        key.strip_prefix(&prefix).unwrap_or(key).to_owned(),
                        if due { 0 } else { 1 },
                        average,
                    )
                })
            })
            .collect::<Vec<_>>();
        topics.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.total_cmp(&b.2)));
        topics
            .into_iter()
            .take(3)
            .map(|(name, _, _)| name)
            .collect()
    }

    pub fn load() -> Result<Self, String> {
        let file = data_file()?;
        if !file.is_file() {
            return Ok(Self {
                version: 1,
                ..Self::default()
            });
        }
        let content = fs::read_to_string(&file)
            .map_err(|err| format!("Не можу прочитати {}: {err}", file.display()))?;
        serde_json::from_str(&content)
            .map_err(|err| format!("Пошкоджено файл прогресу: {err}. Не переписую його."))
    }

    pub fn record(&mut self, repo: &Path, topic: &str, score: u8) {
        self.record_at(repo, topic, score, now());
    }

    fn record_at(&mut self, repo: &Path, topic: &str, score: u8, timestamp: u64) {
        let key = format!("{} | {topic}", repo.display());
        let item = self.topics.entry(key).or_default();
        item.attempts += 1;
        item.total_points += score.min(5) as u32;
        item.successful += u32::from(score >= 3);
        item.last_practiced = timestamp;
        if score >= 4 {
            item.correct_streak = item.correct_streak.saturating_add(1);
            let day = REVIEW_DAYS
                [(item.correct_streak.saturating_sub(1) as usize).min(REVIEW_DAYS.len() - 1)];
            item.next_due = timestamp.saturating_add(day * DAY);
        } else {
            item.correct_streak = 0;
            item.next_due = timestamp.saturating_add(if score <= 2 { 0 } else { DAY });
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let file = data_file()?;
        let directory = file.parent().ok_or("Не знайдено папку прогресу")?;
        fs::create_dir_all(directory).map_err(|err| err.to_string())?;
        let temporary = directory.join(format!("progress-{}.tmp", std::process::id()));
        let json = serde_json::to_vec_pretty(self).map_err(|err| err.to_string())?;
        fs::write(&temporary, json).map_err(|err| err.to_string())?;
        if let Err(err) = fs::rename(&temporary, &file) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Не вдалося зберегти прогрес: {err}"));
        }
        Ok(())
    }

    pub fn print(&self, repo: &Path) {
        let prefix = format!("{} | ", repo.display());
        let mut total = 0;
        let mut earned = 0;
        let current = now();
        println!("\nПрогрес для {}", repo.display());
        for (name, item) in self
            .topics
            .iter()
            .filter(|(name, _)| name.starts_with(&prefix))
        {
            total += item.attempts;
            earned += item.total_points;
            let accuracy = if item.attempts == 0 {
                0.0
            } else {
                item.total_points as f64 * 20.0 / item.attempts as f64
            };
            println!(
                "  {:<32} {:>3} відповідей · {:>5.1}% · {} засвоєно · {}",
                name.strip_prefix(&prefix).unwrap_or(name),
                item.attempts,
                accuracy,
                item.successful,
                if item.next_due == 0 {
                    "ще не заплановано".to_owned()
                } else if item.next_due <= current {
                    "повторити зараз".to_owned()
                } else {
                    format!(
                        "повторити через {} дн.",
                        (item.next_due - current).div_ceil(DAY)
                    )
                }
            );
        }
        if total == 0 {
            println!("  Ще немає відповідей. Запусти: codegym");
        } else {
            println!(
                "  Разом: {total} відповідей, середній бал {:.2}/5",
                earned as f64 / total as f64
            );
        }
        let due = self
            .cards
            .values()
            .filter(|c| c.repo == repo.to_string_lossy() && c.next_due <= current)
            .count();
        println!("  Карток помилок до повторення: {due}. Запуск: codegym mistakes");
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |t| t.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_score_and_success() {
        let mut store = Store::default();
        let repo = Path::new("C:/example");
        store.record(repo, "architecture", 5);
        store.record(repo, "architecture", 2);
        let data = store.topics.values().next().unwrap();
        assert_eq!(data.attempts, 2);
        assert_eq!(data.total_points, 7);
        assert_eq!(data.successful, 1);
    }

    #[test]
    fn serialization_roundtrip() {
        let mut store = Store::default();
        store.record(Path::new("/repo"), "feature", 4);
        let s = serde_json::to_string(&store).unwrap();
        let loaded: Store = serde_json::from_str(&s).unwrap();
        assert_eq!(loaded.topics.len(), 1);
    }

    #[test]
    fn recommends_weak_topics_only() {
        let repo = Path::new("/repo");
        let mut store = Store::default();
        store.record(repo, "Виклики", 1);
        store.record(repo, "Архітектура", 5);
        store.record(Path::new("/another"), "Безпека", 0);
        assert_eq!(store.weak_topics(repo), vec!["Виклики"]);
    }

    #[test]
    fn schedules_increasing_review_intervals_and_resets_on_failure() {
        let repo = Path::new("/repo");
        let mut store = Store::default();
        store.record_at(repo, "Логіка", 5, 1000);
        let item = store.topics.values().next().unwrap();
        assert_eq!(item.correct_streak, 1);
        assert_eq!(item.next_due, 1000 + DAY);
        assert!(store.weak_topics_at(repo, 1000).is_empty());
        assert_eq!(store.weak_topics_at(repo, 1000 + DAY), vec!["Логіка"]);
        store.record_at(repo, "Логіка", 4, 1000 + DAY);
        let item = store.topics.values().next().unwrap();
        assert_eq!(item.correct_streak, 2);
        assert_eq!(item.next_due, 1000 + 4 * DAY);
        store.record_at(repo, "Логіка", 1, 1000 + 4 * DAY);
        let item = store.topics.values().next().unwrap();
        assert_eq!(item.correct_streak, 0);
        assert_eq!(item.next_due, 1000 + 4 * DAY);
    }

    #[test]
    fn old_progress_files_without_schedule_fields_still_load() {
        let old = r#"{"version":1,"topics":{"/repo | Логіка":{"attempts":1,"total_points":4,"successful":1,"last_practiced":1234}}}"#;
        let store: Store = serde_json::from_str(old).unwrap();
        let item = store.topics.values().next().unwrap();
        assert_eq!(item.correct_streak, 0);
        assert_eq!(item.next_due, 0);
        assert!(store.lesson_pages.is_empty());
    }

    #[test]
    fn lesson_pages_are_separate_per_branch_and_mode() {
        let repo = Path::new("/repo");
        let mut store = Store::default();
        assert_eq!(store.lesson_page(repo, "feature", "practice"), 1);
        store.advance_lesson(repo, "feature", "practice");
        assert_eq!(store.lesson_page(repo, "feature", "practice"), 2);
        assert_eq!(store.lesson_page(repo, "feature", "feature"), 1);
        assert_eq!(store.lesson_page(repo, "main", "practice"), 1);
        let restored: Store =
            serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
        assert_eq!(restored.lesson_page(repo, "feature", "practice"), 2);
    }

    #[test]
    fn remembers_specific_failed_question_and_repeats_until_mastered() {
        let repo = Path::new("/repo");
        let q = Question {
            topic: "Виклики".into(),
            question: "Навіщо await?".into(),
            source_path: "src/code.ts".into(),
            source_line: 3,
            reference_answer: "Очікування".into(),
            hint: "Promise".into(),
            practice: None,
        };
        let mut store = Store::default();
        store.remember_at(repo, "feature", &q, "code", 1, 100);
        assert_eq!(store.due_cards_at(repo, "feature", 100).len(), 1);
        assert!(store.due_cards_at(repo, "main", 100).is_empty());
        for i in 0..3 {
            let at = 100 + (i as u64) * 35 * DAY;
            store.remember_at(repo, "feature", &q, "code", 5, at);
        }
        assert!(store.cards.is_empty());
        let original = r#"{"version":1,"topics":{},"lesson_pages":{}}"#;
        assert!(
            serde_json::from_str::<Store>(original)
                .unwrap()
                .cards
                .is_empty()
        );
    }
}
