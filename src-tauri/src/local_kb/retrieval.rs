use serde::Serialize;

use super::cache::LocalKb;
use super::search::{search_kb_files, KbScope, SearchOptions};
use crate::settings::Settings;

pub const SEMANTIC_STRONG_SCORE: f32 = 0.70;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalDomain {
    Law,
    Case,
    Enterprise,
    General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalConfidence {
    None,
    Weak,
    Strong,
    Exact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalStageKind {
    Catalog,
    Curated,
    Lexical,
    Semantic,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalRetrievalHit {
    pub relative_path: String,
    pub title: String,
    pub excerpt: String,
    pub score: f32,
    pub stage: RetrievalStageKind,
    /// `wiki/sources` 卡片回链的真实 raw；专题页或 raw 命中为空。
    pub raw_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievalStage {
    pub kind: RetrievalStageKind,
    pub confidence: RetrievalConfidence,
    pub result_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalRetrievalReport {
    pub query: String,
    pub domain: RetrievalDomain,
    pub confidence: RetrievalConfidence,
    pub stages: Vec<RetrievalStage>,
    pub hits: Vec<LocalRetrievalHit>,
    pub local_available: bool,
}

impl LocalRetrievalReport {
    pub fn is_sufficient(&self) -> bool {
        self.confidence >= RetrievalConfidence::Strong
    }
}

pub async fn retrieve_local(
    kb: Option<&LocalKb>,
    settings: &Settings,
    domain: RetrievalDomain,
    query: &str,
) -> Result<LocalRetrievalReport, String> {
    let Some(kb) = kb else {
        return Ok(LocalRetrievalReport {
            query: query.to_string(),
            domain,
            confidence: RetrievalConfidence::None,
            stages: Vec::new(),
            hits: Vec::new(),
            local_available: false,
        });
    };
    let mut report = LocalRetrievalReport {
        query: query.to_string(),
        domain,
        confidence: RetrievalConfidence::None,
        stages: Vec::new(),
        hits: Vec::new(),
        local_available: true,
    };

    if domain == RetrievalDomain::Law {
        if let Some(law_name) = extract_law_name_hint(query) {
            let entries = super::law_catalog::lookup_law(&kb.root, &law_name, query);
            let confidence = if entries.is_empty() {
                RetrievalConfidence::None
            } else {
                RetrievalConfidence::Exact
            };
            report.stages.push(RetrievalStage {
                kind: RetrievalStageKind::Catalog,
                confidence,
                result_count: entries.len(),
            });
            report
                .hits
                .extend(entries.into_iter().map(|entry| LocalRetrievalHit {
                    relative_path: entry.local_source,
                    title: entry.regulation_name,
                    excerpt: entry.preview,
                    score: 1.0,
                    stage: RetrievalStageKind::Catalog,
                    raw_source: None,
                }));
            if confidence == RetrievalConfidence::Exact {
                report.confidence = confidence;
                return Ok(report);
            }
        }
    }

    // 已治理的 Wiki 层先充当“目录/卡片”：sources 给出单篇材料摘要和 raw 回链，
    // topics 给出场景入口。它们不进入 embedding，避免和原始正文重复向量化。
    if domain != RetrievalDomain::Enterprise {
        let curated = search_kb_files(
            &kb.root,
            query,
            SearchOptions {
                scopes: Some(vec![KbScope::Topics, KbScope::Sources]),
                max_results: 16,
                snippet_chars: 500,
                case_sensitive: false,
            },
        )
        .map_err(|error| error.to_string())?;
        let curated = curated
            .into_iter()
            .filter(|hit| domain_hit(domain, &hit.relative_path, &hit.title))
            .collect::<Vec<_>>();
        let confidence = curated_confidence(query, &curated);
        report.stages.push(RetrievalStage {
            kind: RetrievalStageKind::Curated,
            confidence,
            result_count: curated.len(),
        });
        report
            .hits
            .extend(curated.iter().take(8).map(|hit| LocalRetrievalHit {
                relative_path: hit.relative_path.clone(),
                title: hit.title.clone(),
                excerpt: hit.snippet.clone(),
                score: hit.score as f32,
                stage: RetrievalStageKind::Curated,
                raw_source: source_card_raw_path(&kb.root, &hit.relative_path),
            }));
        report.confidence = confidence;
        if confidence >= RetrievalConfidence::Strong {
            return Ok(report);
        }
    }

    let scopes = match domain {
        RetrievalDomain::Enterprise => vec![KbScope::Companies],
        _ => vec![
            KbScope::Notes,
            KbScope::CasesExperience,
            KbScope::YuandianCache,
        ],
    };
    let lexical = search_kb_files(
        &kb.root,
        query,
        SearchOptions {
            scopes: Some(scopes),
            max_results: 30,
            snippet_chars: 500,
            case_sensitive: false,
        },
    )
    .map_err(|error| error.to_string())?;
    let lexical: Vec<_> = lexical
        .into_iter()
        .filter(|hit| domain_hit(domain, &hit.relative_path, &hit.title))
        .collect();
    let lexical_confidence = lexical_confidence(domain, query, &lexical);
    report.stages.push(RetrievalStage {
        kind: RetrievalStageKind::Lexical,
        confidence: lexical_confidence,
        result_count: lexical.len(),
    });
    report
        .hits
        .extend(lexical.iter().take(8).map(|hit| LocalRetrievalHit {
            relative_path: hit.relative_path.clone(),
            title: hit.title.clone(),
            excerpt: hit.snippet.clone(),
            score: hit.score as f32,
            stage: RetrievalStageKind::Lexical,
            raw_source: None,
        }));
    report.confidence = lexical_confidence;
    if lexical_confidence >= RetrievalConfidence::Strong {
        return Ok(report);
    }

    let key = settings
        .embedding_api_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let endpoint = settings.embedding_endpoint.as_deref().unwrap_or("");
    let model = settings.embedding_model.as_deref().unwrap_or("");
    if domain != RetrievalDomain::Enterprise
        && key.is_some()
        && !endpoint.is_empty()
        && !model.is_empty()
    {
        let semantic = super::semantic::semantic_search(
            &kb.root,
            query,
            12,
            endpoint,
            model,
            key.unwrap_or_default(),
        )
        .await;
        apply_semantic_result(&mut report, domain, semantic);
    }
    Ok(report)
}

fn curated_confidence(query: &str, hits: &[super::search::KbSearchHit]) -> RetrievalConfidence {
    let Some(first) = hits.first() else {
        return RetrievalConfidence::None;
    };
    let normalized_query = query.split_whitespace().collect::<String>();
    let normalized_title = first.title.split_whitespace().collect::<String>();
    if !normalized_query.is_empty()
        && (normalized_title.contains(&normalized_query)
            || normalized_query.contains(&normalized_title))
    {
        RetrievalConfidence::Exact
    } else if first.score >= 24.0 {
        RetrievalConfidence::Strong
    } else {
        RetrievalConfidence::Weak
    }
}

fn source_card_raw_path(kb_root: &std::path::Path, rel_path: &str) -> Option<String> {
    if !rel_path.starts_with("wiki/sources/") {
        return None;
    }
    let content = std::fs::read_to_string(kb_root.join(rel_path)).ok()?;
    let raw = content.lines().find_map(|line| {
        line.trim()
            .strip_prefix("source_path:")
            .map(str::trim)
            .map(|value| value.trim_matches(['\'', '"']).to_string())
    })?;
    if !raw.starts_with("raw/") || !kb_root.join(&raw).is_file() {
        return None;
    }
    Some(raw)
}

fn apply_semantic_result(
    report: &mut LocalRetrievalReport,
    domain: RetrievalDomain,
    semantic: Result<Vec<super::semantic::KbHit>, String>,
) {
    let semantic = match semantic {
        Ok(hits) => hits
            .into_iter()
            .filter(|hit| domain_hit(domain, &hit.rel_path, ""))
            .take(8)
            .collect::<Vec<_>>(),
        Err(error) => {
            crate::dlog!(
                "本地 embedding 查询失败，已保留词法结果并允许后续补检: {}",
                error
            );
            report.stages.push(RetrievalStage {
                kind: RetrievalStageKind::Semantic,
                confidence: RetrievalConfidence::None,
                result_count: 0,
            });
            return;
        }
    };
    let semantic_confidence = semantic
        .first()
        .map(|hit| {
            if hit.score >= SEMANTIC_STRONG_SCORE {
                RetrievalConfidence::Strong
            } else {
                RetrievalConfidence::Weak
            }
        })
        .unwrap_or(RetrievalConfidence::None);
    report.stages.push(RetrievalStage {
        kind: RetrievalStageKind::Semantic,
        confidence: semantic_confidence,
        result_count: semantic.len(),
    });
    append_semantic_hits(report, semantic, semantic_confidence);
}

fn append_semantic_hits(
    report: &mut LocalRetrievalReport,
    semantic: Vec<super::semantic::KbHit>,
    semantic_confidence: RetrievalConfidence,
) {
    let mut semantic_hits = semantic
        .into_iter()
        .map(|hit| LocalRetrievalHit {
            relative_path: hit.rel_path,
            title: String::new(),
            excerpt: hit.text.chars().take(600).collect(),
            score: hit.score,
            stage: RetrievalStageKind::Semantic,
            raw_source: None,
        })
        .collect::<Vec<_>>();
    if semantic_confidence >= RetrievalConfidence::Strong
        && report.confidence < RetrievalConfidence::Strong
    {
        semantic_hits.append(&mut report.hits);
        report.hits = semantic_hits;
    } else {
        report.hits.append(&mut semantic_hits);
    }
    report.confidence = report.confidence.max(semantic_confidence);
}

pub fn extract_law_name_hint(query: &str) -> Option<String> {
    const SUFFIXES: [&str; 10] = [
        "条例实施细则",
        "司法解释",
        "法典",
        "条例",
        "规定",
        "细则",
        "办法",
        "纪要",
        "指引",
        "法",
    ];
    for token in query.split_whitespace() {
        let clean = token.trim_matches(|c: char| "《》“”\"'，,：:；;".contains(c));
        for suffix in SUFFIXES {
            if let Some(end) = clean.find(suffix).map(|index| index + suffix.len()) {
                let title = clean[..end].trim_matches(['《', '》']);
                if title.chars().count() >= 3 {
                    return Some(title.to_string());
                }
            }
        }
    }
    None
}

fn domain_hit(domain: RetrievalDomain, path: &str, title: &str) -> bool {
    let is_case = ["PTAL_", "QWAL_", "判决", "裁定", "调解书", "案例", "纠纷"]
        .iter()
        .any(|marker| path.contains(marker) || title.contains(marker));
    match domain {
        RetrievalDomain::General => true,
        RetrievalDomain::Law => !is_case,
        RetrievalDomain::Enterprise => path.starts_with("raw/companies/"),
        RetrievalDomain::Case => is_case,
    }
}

fn lexical_confidence(
    domain: RetrievalDomain,
    query: &str,
    hits: &[super::search::KbSearchHit],
) -> RetrievalConfidence {
    let Some(first) = hits.first() else {
        return RetrievalConfidence::None;
    };
    let exact_identifier = match domain {
        RetrievalDomain::Case => query.contains('号') && first.snippet.contains(query),
        RetrievalDomain::Enterprise => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0);
            let fresh =
                first.modified_at > 0 && now.saturating_sub(first.modified_at) <= 30 * 86_400;
            let searchable = format!("{}\n{}", first.title, first.snippet);
            let requested_terms = query
                .split_whitespace()
                .filter(|term| !term.is_empty())
                .collect::<Vec<_>>();
            fresh
                && !requested_terms.is_empty()
                && requested_terms.iter().all(|term| searchable.contains(term))
        }
        _ => false,
    };
    if exact_identifier {
        RetrievalConfidence::Exact
    } else if first.score >= 40.0 {
        RetrievalConfidence::Strong
    } else {
        RetrievalConfidence::Weak
    }
}
