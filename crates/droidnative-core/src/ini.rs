//! Minimal order-preserving INI reader/writer compatible with Python's configparser output,
//! used to edit `/var/lib/waydroid/waydroid.cfg` without disturbing keys we do not own.

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    Section(String),
    Entry { key: String, value: String },
    Other(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ini {
    lines: Vec<Line>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let lines = text
            .lines()
            .map(|raw| {
                let t = raw.trim();
                if t.starts_with('[') && t.ends_with(']') {
                    Line::Section(t[1..t.len() - 1].trim().to_string())
                } else if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
                    Line::Other(raw.to_string())
                } else if let Some((k, v)) = t.split_once('=').or_else(|| t.split_once(':')) {
                    Line::Entry { key: k.trim().to_string(), value: v.trim().to_string() }
                } else {
                    Line::Other(raw.to_string())
                }
            })
            .collect();
        Self { lines }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        let mut in_section = false;
        for line in &self.lines {
            match line {
                Line::Section(s) => in_section = s == section,
                Line::Entry { key: k, value } if in_section && k == key => return Some(value),
                _ => {}
            }
        }
        None
    }

    pub fn entries(&self, section: &str) -> Vec<(String, String)> {
        let mut in_section = false;
        let mut out = Vec::new();
        for line in &self.lines {
            match line {
                Line::Section(s) => in_section = s == section,
                Line::Entry { key, value } if in_section => out.push((key.clone(), value.clone())),
                _ => {}
            }
        }
        out
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let mut in_section = false;
        let mut section_end: Option<usize> = None;
        for (i, line) in self.lines.iter_mut().enumerate() {
            match line {
                Line::Section(s) => {
                    if in_section {
                        break;
                    }
                    in_section = s == section;
                    if in_section {
                        section_end = Some(i + 1);
                    }
                }
                Line::Entry { key: k, value: v } if in_section => {
                    if k == key {
                        *v = value.to_string();
                        return;
                    }
                    section_end = Some(i + 1);
                }
                _ => {}
            }
        }
        let entry = Line::Entry { key: key.to_string(), value: value.to_string() };
        match section_end {
            Some(pos) => self.lines.insert(pos, entry),
            None => {
                if !self.lines.is_empty() && !matches!(self.lines.last(), Some(Line::Other(s)) if s.trim().is_empty()) {
                    self.lines.push(Line::Other(String::new()));
                }
                self.lines.push(Line::Section(section.to_string()));
                self.lines.push(entry);
            }
        }
    }

    /// Returns true if the key existed.
    pub fn remove(&mut self, section: &str, key: &str) -> bool {
        let mut in_section = false;
        let pos = self.lines.iter().position(|line| match line {
            Line::Section(s) => {
                in_section = s == section;
                false
            }
            Line::Entry { key: k, .. } => in_section && k == key,
            Line::Other(_) => false,
        });
        match pos {
            Some(i) => {
                self.lines.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Section(s) => out.push_str(&format!("[{s}]\n")),
                Line::Entry { key, value } => out.push_str(&format!("{key} = {value}\n")),
                Line::Other(raw) => {
                    out.push_str(raw);
                    out.push('\n');
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "[waydroid]\narch = x86_64\nvendor_type = MAINLINE\nsuspend_action = freeze\n\n[properties]\npersist.waydroid.multi_windows = true\n";

    #[test]
    fn get_and_entries() {
        let ini = Ini::parse(CFG);
        assert_eq!(ini.get("waydroid", "arch"), Some("x86_64"));
        assert_eq!(ini.get("properties", "arch"), None);
        assert_eq!(ini.entries("properties"), vec![("persist.waydroid.multi_windows".into(), "true".into())]);
    }

    #[test]
    fn set_updates_inserts_and_creates_sections() {
        let mut ini = Ini::parse(CFG);
        ini.set("waydroid", "suspend_action", "none");
        ini.set("waydroid", "drm_device", "/dev/dri/renderD128");
        ini.set("extra", "k", "v");
        let out = ini.render();
        assert!(out.contains("suspend_action = none\ndrm_device = /dev/dri/renderD128\n\n[properties]"));
        assert!(out.ends_with("[extra]\nk = v\n"));
        assert_eq!(Ini::parse(&out).get("waydroid", "drm_device"), Some("/dev/dri/renderD128"));
    }

    #[test]
    fn remove_only_touches_target_section() {
        let mut ini = Ini::parse("[a]\nx = 1\n[b]\nx = 2\n");
        assert!(ini.remove("b", "x"));
        assert!(!ini.remove("b", "x"));
        assert_eq!(ini.render(), "[a]\nx = 1\n[b]\n");
    }

    #[test]
    fn round_trip_preserves_comments() {
        let text = "# managed\n[waydroid]\narch = arm64\n";
        assert_eq!(Ini::parse(text).render(), text);
    }
}
