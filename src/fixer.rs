use std::{
    cmp::{Ordering, Reverse},
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
    candidates.sort_unstable_by(compare_candidates);
    for candidate in &candidates {
        validate_candidate(source, candidate)?;
    }
    let selected = select_disjoint_batches(candidates);
    let applied = selected.len();
    Ok((apply_edits(source, selected), applied))
}

/// Picks fixes in priority order. A grouped fix travels with every later fix
/// of the same rule and group, and the whole batch is dropped if any of its
/// edits overlap each other or an already selected edit.
fn select_disjoint_batches(candidates: Vec<ProposedFix>) -> Vec<ProposedFix> {
    let mut pending: Vec<Option<ProposedFix>> = candidates.into_iter().map(Some).collect();
    let mut selected = Vec::with_capacity(pending.len());
    for index in 0..pending.len() {
        let Some(batch) = take_batch(&mut pending, index) else {
            continue;
        };
        if batch_is_disjoint(&batch, &selected) {
            selected.extend(batch);
        }
    }
    selected
}

/// Removes the fix at `index` and, when it is grouped, every later fix in its group.
fn take_batch(pending: &mut [Option<ProposedFix>], index: usize) -> Option<Vec<ProposedFix>> {
    let first = pending[index].take()?;
    let Some(group) = first.group else {
        return Some(vec![first]);
    };
    let rule = first.rule;
    let members = pending[index + 1..]
        .iter_mut()
        .filter_map(|slot| slot.take_if(|fix| fix.rule == rule && fix.group == Some(group)));
    Some(std::iter::once(first).chain(members).collect())
}

fn apply_edits(source: &str, mut edits: Vec<ProposedFix>) -> String {
    edits.sort_unstable_by_key(|edit| edit.start);
    let mut output = source.to_owned();
    // Back to front, so each edit's byte offsets are still valid.
    for edit in edits.into_iter().rev() {
        output.replace_range(edit.start..edit.end, &edit.replacement);
    }
    output
}

fn batch_is_disjoint(batch: &[ProposedFix], selected: &[ProposedFix]) -> bool {
    batch.iter().enumerate().all(|(index, candidate)| {
        !overlaps_any(candidate, selected) && !overlaps_any(candidate, &batch[..index])
    })
}

fn compare_candidates(left: &ProposedFix, right: &ProposedFix) -> Ordering {
    (
        fix_priority(left.rule),
        left.start,
        Reverse(left.end),
        left.rule,
    )
        .cmp(&(
            fix_priority(right.rule),
            right.start,
            Reverse(right.end),
            right.rule,
        ))
}

fn overlaps_any(candidate: &ProposedFix, selected: &[ProposedFix]) -> bool {
    for existing in selected {
        if candidate.end > existing.start && candidate.start < existing.end {
            return true;
        }
    }
    false
}

fn fix_priority(rule: &str) -> u8 {
    match rule {
        "redundant-local-key-remap" | "single-use-local-alias" => 0,
        "else-after-exit" | "terminal-guard-clause" | "duplicate-branch-body" => 2,
        _ => 1,
    }
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
                group: None,
                start: 1,
                end: 5,
                expected: "bcde".to_owned(),
                replacement: "X".to_owned(),
                line: 1,
            },
            ProposedFix {
                rule: "inner",
                group: None,
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

    #[test]
    fn key_remaps_win_over_overlapping_style_edits_without_reordering_offsets() {
        let source = "old middle access";
        let fixes = vec![
            ProposedFix {
                rule: "prefer-dot-property",
                group: None,
                start: 11,
                end: 17,
                expected: "access".to_owned(),
                replacement: "style".to_owned(),
                line: 1,
            },
            ProposedFix {
                rule: "redundant-local-key-remap",
                group: Some(1),
                start: 0,
                end: 3,
                expected: "old".to_owned(),
                replacement: "new".to_owned(),
                line: 1,
            },
            ProposedFix {
                rule: "redundant-local-key-remap",
                group: Some(1),
                start: 11,
                end: 17,
                expected: "access".to_owned(),
                replacement: "renamed".to_owned(),
                line: 1,
            },
        ];
        let (output, applied) = apply_candidates(source, fixes).expect("edits should apply");
        assert_eq!(output, "new middle renamed");
        assert_eq!(applied, 2);
    }

    #[test]
    fn overlapping_group_is_skipped_atomically() {
        let source = "alias and use";
        let fixes = vec![
            ProposedFix {
                rule: "outer",
                group: None,
                start: 0,
                end: 9,
                expected: "alias and".to_owned(),
                replacement: "kept".to_owned(),
                line: 1,
            },
            ProposedFix {
                rule: "grouped",
                group: Some(7),
                start: 0,
                end: 5,
                expected: "alias".to_owned(),
                replacement: String::new(),
                line: 1,
            },
            ProposedFix {
                rule: "grouped",
                group: Some(7),
                start: 10,
                end: 13,
                expected: "use".to_owned(),
                replacement: "source".to_owned(),
                line: 1,
            },
        ];
        let (output, applied) = apply_candidates(source, fixes).expect("edits should apply");
        assert_eq!(output, "kept use");
        assert_eq!(applied, 1);
    }

    fn staged(path: &Path, original: &str, replacement: &str) -> StagedFile {
        StagedFile {
            path: path.to_owned(),
            original: original.to_owned(),
            replacement: replacement.to_owned(),
            permissions: fs::metadata(path.parent().expect("test paths have a parent"))
                .expect("parent exists")
                .permissions(),
            applied: 1,
        }
    }

    #[test]
    fn failed_write_rolls_back_earlier_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first = directory.path().join("first.ts");
        fs::write(&first, "original").expect("seed first file");
        // A regular file cannot be a parent directory, so this write fails
        // even when the tests run as root.
        let blocker = directory.path().join("blocker");
        fs::write(&blocker, "").expect("seed blocker");
        let mut second = staged(&first, "", "new");
        second.path = blocker.join("second.ts");
        let files = [staged(&first, "original", "replaced"), second];

        let error = write_all_or_rollback(&files).expect_err("second write must fail");

        assert!(format!("{error:#}").contains("earlier writes were rolled back"));
        assert_eq!(
            fs::read_to_string(&first).expect("first exists"),
            "original"
        );
    }

    #[test]
    fn successful_writes_replace_every_file() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let paths = ["a.ts", "b.ts"].map(|name| directory.path().join(name));
        for path in &paths {
            fs::write(path, "before").expect("seed file");
        }
        let files = paths.each_ref().map(|path| staged(path, "before", "after"));

        write_all_or_rollback(&files).expect("writes succeed");

        for path in &paths {
            assert_eq!(fs::read_to_string(path).expect("file exists"), "after");
        }
    }
}
