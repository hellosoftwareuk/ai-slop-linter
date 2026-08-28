use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use atomicwrites::{AtomicFile, OverwriteBehavior};

use crate::{
    analyzer::{analyze_file, fingerprint},
    model::{FileAnalysis, FixSummary, Language, ProposedFix},
};

const MAX_FIX_PASSES: usize = 64;

struct StagedFile {
    path: PathBuf,
    original: String,
    replacement: String,
    permissions: fs::Permissions,
    applied: usize,
}

pub fn apply(root: &Path, analyses: &[FileAnalysis]) -> Result<FixSummary> {
    let mut staged = Vec::new();
    for analysis in analyses {
        if !eligible(analysis) || analysis.proposed_fixes.is_empty() {
            continue;
        }
        let metadata = fs::symlink_metadata(&analysis.path).with_context(|| {
            format!("cannot inspect '{}' before fixing", analysis.path.display())
        })?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let original = fs::read_to_string(&analysis.path).with_context(|| {
            format!("cannot reread '{}' before fixing", analysis.path.display())
        })?;
        if fingerprint(&original) != analysis.source_fingerprint {
            bail!(
                "'{}' changed after it was analyzed; no fixes were written",
                analysis.path.display()
            );
        }

        let (replacement, applied) = converge(
            &analysis.path,
            root,
            original.clone(),
            analysis.proposed_fixes.clone(),
        )?;
        if replacement != original {
            staged.push(StagedFile {
                path: analysis.path.clone(),
                original,
                replacement,
                permissions: metadata.permissions(),
                applied,
            });
        }
    }

    for file in &staged {
        let current = fs::read_to_string(&file.path)
            .with_context(|| format!("cannot verify '{}' before fixing", file.path.display()))?;
        if current != file.original {
            bail!(
                "'{}' changed while fixes were being staged; no fixes were written",
                file.path.display()
            );
        }
    }

    write_all_or_rollback(&staged)?;
    Ok(FixSummary {
        requested: true,
        applied: staged.iter().map(|file| file.applied).sum(),
        files_changed: staged.len(),
    })
}

fn eligible(analysis: &FileAnalysis) -> bool {
    analysis.language == Language::TypeScript
        && analysis.parse_errors == 0
        && !analysis
            .path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".d.ts"))
}

fn converge(
    path: &Path,
    root: &Path,
    mut source: String,
    mut candidates: Vec<ProposedFix>,
) -> Result<(String, usize)> {
    let mut applied = 0;
    for _ in 0..MAX_FIX_PASSES {
        if candidates.is_empty() {
            return Ok((source, applied));
        }
        let (next, edits) = apply_candidates(&source, candidates)?;
        if edits == 0 || next == source {
            bail!(
                "safe fixes for '{}' did not make progress; no fixes were written",
                path.display()
            );
        }
        applied += edits;
        let next_analysis = analyze_file(path, root, next.clone(), next.len() as u64)
            .with_context(|| format!("cannot validate fixes for '{}'", path.display()))?;
        if next_analysis.parse_errors > 0 {
            bail!(
                "a proposed fix made '{}' invalid; no fixes were written",
                path.display()
            );
        }
        source = next;
        candidates = next_analysis.proposed_fixes;
    }
    bail!(
        "safe fixes for '{}' did not converge after {MAX_FIX_PASSES} passes; no fixes were written",
        path.display()
    )
}

fn apply_candidates(source: &str, mut candidates: Vec<ProposedFix>) -> Result<(String, usize)> {
    candidates.sort_unstable_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| right.end.cmp(&left.end))
            .then_with(|| left.rule.cmp(right.rule))
    });

    let mut selected = Vec::with_capacity(candidates.len());
    let mut occupied_until = 0;
    for candidate in candidates {
        validate_candidate(source, &candidate)?;
        if candidate.start >= occupied_until {
            occupied_until = candidate.end;
            selected.push(candidate);
        }
    }

    let applied = selected.len();
    let mut output = source.to_owned();
    for candidate in selected.into_iter().rev() {
        output.replace_range(candidate.start..candidate.end, &candidate.replacement);
    }
    Ok((output, applied))
}

fn validate_candidate(source: &str, candidate: &ProposedFix) -> Result<()> {
    if candidate.start >= candidate.end
        || !source.is_char_boundary(candidate.start)
        || !source.is_char_boundary(candidate.end)
        || source.get(candidate.start..candidate.end) != Some(candidate.expected.as_str())
    {
        bail!(
            "stale or invalid '{}' edit at line {}; no fixes were written",
            candidate.rule,
            candidate.line
        );
    }
    Ok(())
}

fn write_all_or_rollback(staged: &[StagedFile]) -> Result<()> {
    let mut written = Vec::with_capacity(staged.len());
    for (index, file) in staged.iter().enumerate() {
        let write_result = atomic_write(&file.path, &file.replacement);
        if write_result.is_ok() {
            written.push(index);
        }
        let result = write_result.and_then(|()| {
            fs::set_permissions(&file.path, file.permissions.clone()).map_err(Into::into)
        });
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for written_index in written.into_iter().rev() {
                let previous: &StagedFile = &staged[written_index];
                if let Err(rollback_error) = atomic_write(&previous.path, &previous.original)
                    .and_then(|()| {
                        fs::set_permissions(&previous.path, previous.permissions.clone())
                            .map_err(Into::into)
                    })
                {
                    rollback_errors
                        .push(format!("{}: {rollback_error:#}", previous.path.display()));
                }
            }
            if rollback_errors.is_empty() {
                return Err(error).with_context(|| {
                    format!(
                        "could not write '{}'; earlier writes were rolled back",
                        file.path.display()
                    )
                });
            }
            bail!(
                "could not write '{}': {error:#}; rollback also failed for {}",
                file.path.display(),
                rollback_errors.join(", ")
            );
        }
    }
    Ok(())
}

fn atomic_write(path: &Path, source: &str) -> Result<()> {
    AtomicFile::new(path, OverwriteBehavior::AllowOverwrite)
        .write(|file| -> std::io::Result<()> {
            file.write_all(source.as_bytes())?;
            file.sync_all()
        })
        .map_err(|error| anyhow::anyhow!(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_edits_choose_the_outer_range() {
        let source = "abcdef";
        let fixes = vec![
            ProposedFix {
                rule: "outer",
                start: 1,
                end: 5,
                expected: "bcde".to_owned(),
                replacement: "X".to_owned(),
                line: 1,
            },
            ProposedFix {
                rule: "inner",
                start: 2,
                end: 4,
                expected: "cd".to_owned(),
                replacement: "Y".to_owned(),
                line: 1,
            },
        ];
        let (output, applied) = apply_candidates(source, fixes).expect("edits should apply");
        assert_eq!(output, "aXf");
        assert_eq!(applied, 1);
    }
}
