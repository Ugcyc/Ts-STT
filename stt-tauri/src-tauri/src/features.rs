use regex::{Captures, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, sync::OnceLock};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Replacement {
    pub heard: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub id: String,
    pub created_at: u64,
    pub text: String,
    pub seconds: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PerformanceReport {
    pub load_ms: u64,
    pub audio_seconds: f64,
    pub inference_ms: u64,
    pub finish_ms: u64,
    pub max_queue_ms: u64,
    pub first_text_ms: Option<u64>,
    pub overflowed: bool,
}

impl PerformanceReport {
    pub fn real_time_factor(&self) -> f64 {
        if self.audio_seconds > 0.0 {
            self.inference_ms as f64 / 1000.0 / self.audio_seconds
        } else {
            0.0
        }
    }
}

// Hold the last two words inside the preview; only the settled prefix is injected.
pub fn settled_prefix(text: &str) -> String {
    let starts: Vec<usize> = text
        .char_indices()
        .filter_map(|(i, c)| {
            (!c.is_whitespace() && (i == 0 || text[..i].ends_with(char::is_whitespace)))
                .then_some(i)
        })
        .collect();
    if starts.len() <= 2 {
        String::new()
    } else {
        text[..starts[starts.len() - 2]].to_string()
    }
}

pub fn transform_text(text: &str, punctuation: bool, rules: &[Replacement]) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static PUNCTUATION: OnceLock<Regex> = OnceLock::new();
    static SPACES: OnceLock<Regex> = OnceLock::new();
    let mut text = TAGS
        .get_or_init(|| Regex::new(r"\s*<[a-z]{2}(?:-[A-Z]{2})?>\s*").unwrap())
        .replace_all(text, " ")
        .trim()
        .to_string();
    if punctuation {
        static COMMANDS: OnceLock<Regex> = OnceLock::new();
        text = COMMANDS.get_or_init(|| Regex::new(r"(?i)\b(new paragraph|new line|question mark|exclamation mark|comma|period)\b[,.!?]?").unwrap())
            .replace_all(&text, |c: &Captures| match c[1].to_lowercase().as_str() {
                "new paragraph" => "\n\n", "new line" => "\n", "comma" => ",",
                "period" => ".", "question mark" => "?", _ => "!",
            }).into_owned();
    }
    // A single alternation makes replacements non-cascading. Longer phrases win.
    let mut rules: Vec<&Replacement> = rules
        .iter()
        .filter(|r| !r.heard.trim().is_empty())
        .collect();
    rules.sort_by_key(|r| std::cmp::Reverse(r.heard.len()));
    if !rules.is_empty() {
        let pattern = rules
            .iter()
            .map(|r| {
                let phrase = r.heard.trim();
                let left = if phrase
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
                {
                    r"\b"
                } else {
                    ""
                };
                let right = if phrase
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
                {
                    r"\b"
                } else {
                    ""
                };
                format!("{left}{}{right}", regex::escape(phrase))
            })
            .collect::<Vec<_>>()
            .join("|");
        if let Ok(re) = RegexBuilder::new(&pattern).case_insensitive(true).build() {
            text = re
                .replace_all(&text, |c: &Captures| {
                    rules
                        .iter()
                        .find(|r| r.heard.trim().to_lowercase() == c[0].to_lowercase())
                        .map(|r| r.replacement.clone())
                        .unwrap_or_else(|| c[0].to_string())
                })
                .into_owned();
        }
    }
    text = PUNCTUATION
        .get_or_init(|| Regex::new(r"[ \t]+([,.!?])").unwrap())
        .replace_all(&text, "$1")
        .into_owned();
    text = SPACES
        .get_or_init(|| Regex::new(r"[ \t]{2,}").unwrap())
        .replace_all(&text, " ")
        .into_owned();
    text.lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

pub fn save_history(path: &Path, entries: &[TranscriptEntry]) -> Result<(), String> {
    let data = serde_json::to_vec(entries).map_err(|e| e.to_string())?;
    atomic_write(path, &data)
}

pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Missing destination folder")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(data).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_are_whole_words_case_insensitive_and_preserve_paragraphs() {
        assert_eq!(
            transform_text(
                "Periodic reports COMMA new paragraph Next period.",
                true,
                &[]
            ),
            "Periodic reports,\n\nNext."
        );
    }
    #[test]
    fn replacements_are_literal_non_cascading_and_longest_first() {
        let rules = vec![
            Replacement {
                heard: "alpha".into(),
                replacement: "beta".into(),
            },
            Replacement {
                heard: "beta".into(),
                replacement: "gamma".into(),
            },
            Replacement {
                heard: "alpha team".into(),
                replacement: "$1".into(),
            },
        ];
        assert_eq!(
            transform_text("ALPHA team alpha alphabet beta", false, &rules),
            "$1 beta alphabet gamma"
        );
    }
    #[test]
    fn empty_rules_and_disabled_commands_preserve_normal_dictation() {
        assert_eq!(
            transform_text("A period of time.", false, &[]),
            "A period of time."
        );
    }
    #[test]
    fn unicode_prefix_never_splits_a_character() {
        assert_eq!(settled_prefix("Bonjour à tous demain"), "Bonjour à ");
        assert_eq!(settled_prefix("你好世界"), "");
    }
    #[test]
    fn history_overwrites_existing_file_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        atomic_write(&path, b"old").unwrap();
        save_history(&path, &[]).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "[]");
    }
}

pub struct StreamingResampler {
    source_rate: u32,
    buffer: Vec<f32>,
    position: f64,
}

impl StreamingResampler {
    pub fn new() -> Self {
        Self {
            source_rate: 0,
            buffer: Vec::new(),
            position: 0.0,
        }
    }

    pub fn push(&mut self, samples: &[f32], source_rate: u32) -> Vec<f32> {
        if samples.is_empty() || source_rate == 0 {
            return Vec::new();
        }
        if self.source_rate != source_rate {
            self.source_rate = source_rate;
            self.buffer.clear();
            self.position = 0.0;
        }
        if source_rate == 16_000 {
            return samples.to_vec();
        }

        self.buffer.extend_from_slice(samples);
        let step = source_rate as f64 / 16_000.0;
        let mut output = Vec::new();
        while self.position + 1.0 < self.buffer.len() as f64 {
            let index = self.position.floor() as usize;
            let fraction = (self.position - index as f64) as f32;
            output.push(
                self.buffer[index] + (self.buffer[index + 1] - self.buffer[index]) * fraction,
            );
            self.position += step;
        }
        let consumed = (self.position.floor() as usize).min(self.buffer.len());
        if consumed > 0 {
            self.buffer.drain(..consumed);
            self.position -= consumed as f64;
        }
        output
    }
}

impl Default for StreamingResampler {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod audio_tests {
    use super::*;
    #[test]
    fn tiny_callback_buffers_do_not_drop_time_or_panic() {
        let mut r = StreamingResampler::new();
        let samples: Vec<f32> = (0..4800)
            .flat_map(|_| r.push(&[0.25, 0.25], 48000))
            .collect();
        assert!((3199..=3200).contains(&samples.len()));
        assert!(samples.iter().all(|x| (*x - 0.25).abs() < f32::EPSILON));
    }
}
