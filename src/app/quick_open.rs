//! Seletor rápido (`Ctrl+P`): tabelas/views e arquivos SQL do workspace ativo.

use std::path::{Path, PathBuf};

use crate::metadata::{Catalog, RelationKind};

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Relation {
        schema: String,
        name: String,
        kind: RelationKind,
    },
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    pub detail: String,
    pub target: Target,
}

pub struct QuickOpen {
    pub query: String,
    pub items: Vec<Item>,
    pub matches: Vec<usize>,
    pub selected: usize,
    pub input_id: iced::widget::Id,
}

const MAX_RESULTS: usize = 50;

impl QuickOpen {
    pub fn new(catalog: Option<&Catalog>, files: &[PathBuf], files_root: Option<&Path>) -> Self {
        let mut items = Vec::new();
        if let Some(catalog) = catalog {
            for r in catalog.relations() {
                items.push(Item {
                    label: format!("{}.{}", r.schema, r.name),
                    detail: r.kind.label().into(),
                    target: Target::Relation {
                        schema: r.schema.clone(),
                        name: r.name.clone(),
                        kind: r.kind,
                    },
                });
            }
        }
        for f in files {
            let rel = files_root
                .and_then(|root| f.strip_prefix(root).ok())
                .unwrap_or(f);
            items.push(Item {
                label: rel.display().to_string(),
                detail: "arquivo SQL".into(),
                target: Target::File(f.clone()),
            });
        }
        let mut q = Self {
            query: String::new(),
            items,
            matches: Vec::new(),
            selected: 0,
            input_id: iced::widget::Id::unique(),
        };
        q.refilter();
        q
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.refilter();
    }

    pub fn move_selection(&mut self, delta: i32) {
        if self.matches.is_empty() {
            return;
        }
        let len = self.matches.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
    }

    pub fn current(&self) -> Option<&Item> {
        self.matches.get(self.selected).map(|&i| &self.items[i])
    }

    fn refilter(&mut self) {
        let mut scored: Vec<(i64, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| score(&self.query, &item.label).map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| self.items[a.1].label.cmp(&self.items[b.1].label))
        });
        self.matches = scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, i)| i)
            .collect();
        self.selected = 0;
    }
}

/// Correspondência por subsequência, sem diferenciar maiúsculas. Favorece
/// caracteres consecutivos e início de palavras.
pub fn score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.to_lowercase().chars().collect();
    let mut score = 0i64;
    let mut pos = 0usize;
    let mut prev: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = cand[pos..].iter().position(|&c| c == q)? + pos;
        score += 1;
        if prev.is_some_and(|p| p + 1 == found) {
            score += 5;
        }
        if found == 0 || matches!(cand[found - 1], '.' | '_' | '/' | '-' | ' ') {
            score += 3;
        }
        prev = Some(found);
        pos = found + 1;
    }
    Some(score * 100 - cand.len() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_scoring() {
        assert!(score("usr", "public.users").is_some());
        assert!(score("xyz", "public.users").is_none());
        assert!(
            score("users", "public.users").unwrap() > score("users", "public.u_s_e_r_s").unwrap()
        );
    }
}
