use std::{collections::BTreeMap, path::PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[serde(rename = "typescript")]
    TypeScript,
    Rust,
    Terraform,
    Terragrunt,
}

impl Language {
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "ts" | "tsx" => Some(Self::TypeScript),
            "rs" => Some(Self::Rust),
            "tf" | "tfvars" => Some(Self::Terraform),
            "hcl" if !is_terraform_lock(path) => Some(Self::Terragrunt),
            _ => None,
        }
    }
}

fn is_terraform_lock(path: &std::path::Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(".terraform.lock.hcl"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Complexity,
    Size,
    Readability,
    Abstraction,
    TypeSafety,
    Architecture,
    Structure,
}

impl Category {
    pub const ALL: [Self; 7] = [
        Self::Complexity,
        Self::Size,
        Self::Readability,
        Self::Abstraction,
        Self::TypeSafety,
        Self::Architecture,
        Self::Structure,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DependencyKind {
    Import,
    ReExport,
}

#[derive(Debug, Clone)]
pub struct ModuleDependency {
    pub specifier: String,
    pub line: usize,
    pub kind: DependencyKind,
}

#[derive(Debug, Clone)]
pub struct CloneCandidate {
    pub fingerprint: u64,
    pub tokens: usize,
    pub line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Note,
    Warning,
    High,
}

impl Severity {
    pub fn from_points(points: f64) -> Self {
        if points >= 8.0 {
            Self::High
        } else if points >= 4.0 {
            Self::Warning
        } else {
            Self::Note
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule: &'static str,
    pub category: Category,
    pub severity: Severity,
    pub points: f64,
    pub path: String,
    pub line: usize,
    pub message: String,
    pub evidence: String,
    pub remediation_prompt: String,
    pub fixable: bool,
}

impl Finding {
    pub fn new(
        rule: &'static str,
        category: Category,
        points: f64,
        location: (String, usize),
        text: (impl Into<String>, impl Into<String>),
    ) -> Self {
        let (path, line) = location;
        let (message, evidence) = text;
        let message = message.into();
        let evidence = evidence.into();
        let remediation_prompt = crate::remediation::finding_prompt(rule, &path, line, &evidence);
        Self {
            rule,
            category,
            severity: Severity::from_points(points),
            points,
            path,
            line,
            message,
            evidence,
            remediation_prompt,
            fixable: false,
        }
    }

    pub fn with_fixable(mut self) -> Self {
        self.fixable = true;
        self
    }
}

#[derive(Debug, Clone)]
pub struct ProposedFix {
    pub rule: &'static str,
    pub start: usize,
    pub end: usize,
    pub expected: String,
    pub replacement: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub kind: &'static str,
    pub path: String,
    pub count: usize,
    pub message: String,
    pub remediation_prompt: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct AstMetrics {
    pub functions: usize,
    pub imports: usize,
    pub classes: usize,
    pub interfaces: usize,
    pub type_aliases: usize,
    pub structs: usize,
    pub enums: usize,
    pub traits: usize,
    pub macro_invocations: usize,
    pub macro_definitions: usize,
    pub macro_inputs_analyzed: usize,
    pub macro_inputs_unresolved: usize,
    pub any_keywords: usize,
    pub type_assertions: usize,
    pub vague_bindings: usize,
    pub thin_wrappers: usize,
    pub hcl_blocks: usize,
    pub hcl_attributes: usize,
    pub terraform_resources: usize,
    pub terraform_variables: usize,
    pub terragrunt_dependencies: usize,
}

impl AstMetrics {
    pub fn add_assign(&mut self, other: &Self) {
        *self = Self {
            functions: self.functions + other.functions,
            imports: self.imports + other.imports,
            classes: self.classes + other.classes,
            interfaces: self.interfaces + other.interfaces,
            type_aliases: self.type_aliases + other.type_aliases,
            structs: self.structs + other.structs,
            enums: self.enums + other.enums,
            traits: self.traits + other.traits,
            macro_invocations: self.macro_invocations + other.macro_invocations,
            macro_definitions: self.macro_definitions + other.macro_definitions,
            macro_inputs_analyzed: self.macro_inputs_analyzed + other.macro_inputs_analyzed,
            macro_inputs_unresolved: self.macro_inputs_unresolved + other.macro_inputs_unresolved,
            any_keywords: self.any_keywords + other.any_keywords,
            type_assertions: self.type_assertions + other.type_assertions,
            vague_bindings: self.vague_bindings + other.vague_bindings,
            thin_wrappers: self.thin_wrappers + other.thin_wrappers,
            hcl_blocks: self.hcl_blocks + other.hcl_blocks,
            hcl_attributes: self.hcl_attributes + other.hcl_attributes,
            terraform_resources: self.terraform_resources + other.terraform_resources,
            terraform_variables: self.terraform_variables + other.terraform_variables,
            terragrunt_dependencies: self.terragrunt_dependencies + other.terragrunt_dependencies,
        };
    }
}

#[derive(Debug)]
pub struct FileAnalysis {
    pub path: PathBuf,
    pub display_path: String,
    pub language: Language,
    pub bytes: u64,
    pub lines: usize,
    pub parse_errors: usize,
    pub metrics: AstMetrics,
    pub findings: Vec<Finding>,
    pub dependencies: Vec<ModuleDependency>,
    pub clone_candidates: Vec<CloneCandidate>,
    pub top_level_statements: usize,
    pub source_fingerprint: u64,
    pub proposed_fixes: Vec<ProposedFix>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct FixSummary {
    pub requested: bool,
    pub applied: usize,
    pub files_changed: usize,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct RepositoryMetrics {
    pub modules: usize,
    pub directories: usize,
    pub internal_dependencies: usize,
    pub unresolved_relative_dependencies: usize,
}

#[derive(Debug, Serialize)]
pub struct FileSummary {
    pub path: String,
    pub score: u8,
    pub lines: usize,
    pub points: f64,
}

#[derive(Debug, Serialize)]
pub struct ScanReport {
    pub root: String,
    pub score: u8,
    pub rating: &'static str,
    pub debt_points: f64,
    pub points_per_kloc: f64,
    pub elapsed_ms: u128,
    pub files: usize,
    pub languages: BTreeMap<Language, usize>,
    pub lines: usize,
    pub bytes: u64,
    pub parse_errors: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub metrics: AstMetrics,
    pub repository_metrics: RepositoryMetrics,
    pub category_scores: BTreeMap<Category, u8>,
    pub hotspots: Vec<FileSummary>,
    pub findings: Vec<Finding>,
    pub fixes: FixSummary,
}
