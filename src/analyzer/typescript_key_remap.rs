use std::collections::HashSet;

use oxc_ast::{
    ast::{
        Expression, ObjectExpression, ObjectProperty, ObjectPropertyKind, Program, PropertyKey,
        PropertyKind,
    },
    AstKind,
};
use oxc_semantic::Semantic;
use oxc_span::Span;

use crate::model::ProposedFix;

use super::{
    core::{KeyRemapSignal, LineIndex},
    typescript_fix_context::FixContext,
    typescript_key_remap_flow::{self, Edit},
};

pub(super) struct KeyRemapAnalysis {
    pub fixes: Vec<ProposedFix>,
    pub blocked: Vec<KeyRemapSignal>,
}

pub(super) fn is_potential_property(property: &ObjectProperty<'_>) -> bool {
    candidate_names(property).is_some()
}

pub(super) fn collect(
    program: &Program<'_>,
    source: &str,
    semantic: &Semantic<'_>,
) -> KeyRemapAnalysis {
    if semantic.nodes().is_empty() {
        return KeyRemapAnalysis {
            fixes: Vec::new(),
            blocked: Vec::new(),
        };
    }
    let lines = LineIndex::new(source);
    let mut context = FixContext::new(source, &program.comments);
    let mut blocked = Vec::new();
    let direct_eval = has_direct_eval(semantic);

    for (property_id, node) in semantic.nodes().iter_enumerated() {
        let AstKind::ObjectProperty(property) = node.kind() else {
            continue;
        };
        let Some((old_key, new_key)) = candidate_names(property) else {
            continue;
        };
        let object_id = semantic.nodes().parent_id(property_id);
        let AstKind::ObjectExpression(object) = semantic.nodes().kind(object_id) else {
            continue;
        };
        let line = lines.line(property.span.start);
        let result = prove_candidate(
            semantic,
            CandidateProof {
                object,
                object_id,
                property_span: property.span,
                old_key,
                new_key,
            },
            source,
            direct_eval,
        );
        match result {
            Ok(mut edits) => {
                edits.insert(
                    0,
                    Edit {
                        span: property.span,
                        replacement: new_key.to_owned(),
                    },
                );
                if edits.iter().any(|edit| context.has_comment(edit.span)) {
                    blocked.push(signal(
                        old_key,
                        new_key,
                        line,
                        "comments document the mapping or one of its uses",
                    ));
                    continue;
                }
                let mut unique = HashSet::new();
                let group = property.span.start;
                for edit in edits {
                    if unique.insert((edit.span.start, edit.span.end)) {
                        context.propose_grouped(
                            "redundant-local-key-remap",
                            edit.span,
                            edit.replacement,
                            group,
                        );
                    }
                }
            }
            Err(reason) => blocked.push(signal(old_key, new_key, line, reason)),
        }
    }

    KeyRemapAnalysis {
        fixes: context.into_fixes(),
        blocked,
    }
}

struct CandidateProof<'a> {
    object: &'a ObjectExpression<'a>,
    object_id: oxc_semantic::NodeId,
    property_span: Span,
    old_key: &'a str,
    new_key: &'a str,
}

fn prove_candidate(
    semantic: &Semantic<'_>,
    candidate: CandidateProof<'_>,
    source: &str,
    direct_eval: bool,
) -> Result<Vec<Edit>, &'static str> {
    if direct_eval {
        return Err("direct eval can observe names outside the static graph");
    }
    validate_object(
        candidate.object,
        candidate.property_span,
        candidate.old_key,
        candidate.new_key,
    )?;
    let owner = typescript_key_remap_flow::owner_for_object(
        semantic,
        candidate.object_id,
        vec![candidate.old_key.to_owned()],
    )?;
    typescript_key_remap_flow::prove(semantic, owner, candidate.new_key, source)
}

fn validate_object(
    object: &ObjectExpression<'_>,
    candidate_span: Span,
    old_key: &str,
    new_key: &str,
) -> Result<(), &'static str> {
    let mut old_key_count = 0;
    for item in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = item else {
            return Err("a spread may already provide the proposed shorthand key");
        };
        if property.computed {
            return Err("a computed sibling key may collide with the proposed shorthand");
        }
        let Some(name) = static_property_name(property) else {
            continue;
        };
        if name == old_key {
            old_key_count += 1;
        }
        if name == new_key && property.span != candidate_span {
            return Err("the object already contains the proposed shorthand key");
        }
    }
    if old_key_count != 1 {
        return Err("the original key is duplicated in the object literal");
    }
    Ok(())
}

fn candidate_names<'a>(property: &'a ObjectProperty<'_>) -> Option<(&'a str, &'a str)> {
    if property.shorthand
        || property.computed
        || property.method
        || property.kind != PropertyKind::Init
    {
        return None;
    }
    let old_key = static_property_name(property)?;
    let Expression::Identifier(value) = &property.value else {
        return None;
    };
    let new_key = value.name.as_str();
    (old_key != "__proto__" && old_key != new_key && names_are_similar(old_key, new_key))
        .then_some((old_key, new_key))
}

pub(super) fn static_property_name<'a>(property: &'a ObjectProperty<'_>) -> Option<&'a str> {
    match &property.key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.as_str()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.as_str()),
        _ => None,
    }
}

fn names_are_similar(left: &str, right: &str) -> bool {
    let left = normalized_name(left);
    let right = normalized_name(right);
    if left.len().min(right.len()) < 4 {
        return false;
    }
    left == right || bounded_edit_distance(&left, &right, 1).is_some()
}

fn normalized_name(name: &str) -> String {
    let canonical = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    for (long, short) in [
        ("identifier", "id"),
        ("reference", "ref"),
        ("number", "num"),
        ("quantity", "qty"),
        ("description", "desc"),
        ("message", "msg"),
    ] {
        if let Some(prefix) = canonical.strip_suffix(long) {
            return format!("{prefix}{short}");
        }
    }
    canonical
}

fn bounded_edit_distance(left: &str, right: &str, limit: usize) -> Option<usize> {
    if left.len().abs_diff(right.len()) > limit {
        return None;
    }
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut distances = vec![vec![0; right.len() + 1]; left.len() + 1];
    for (index, row) in distances.iter_mut().enumerate() {
        row[0] = index;
    }
    for (index, distance) in distances[0].iter_mut().enumerate() {
        *distance = index;
    }
    for left_index in 1..=left.len() {
        let mut row_minimum = usize::MAX;
        for right_index in 1..=right.len() {
            let substitution = distances[left_index - 1][right_index - 1]
                + usize::from(left[left_index - 1] != right[right_index - 1]);
            let insertion = distances[left_index][right_index - 1] + 1;
            let deletion = distances[left_index - 1][right_index] + 1;
            let mut distance = substitution.min(insertion).min(deletion);
            if left_index > 1
                && right_index > 1
                && left[left_index - 1] == right[right_index - 2]
                && left[left_index - 2] == right[right_index - 1]
            {
                distance = distance.min(distances[left_index - 2][right_index - 2] + 1);
            }
            distances[left_index][right_index] = distance;
            row_minimum = row_minimum.min(distance);
        }
        if row_minimum > limit {
            return None;
        }
    }
    distances[left.len()][right.len()]
        .le(&limit)
        .then_some(distances[left.len()][right.len()])
}

pub(super) fn has_direct_eval(semantic: &Semantic<'_>) -> bool {
    semantic.nodes().iter().any(|node| {
        matches!(
            node.kind(),
            AstKind::CallExpression(call)
                if matches!(
                    call.callee.get_inner_expression(),
                    Expression::Identifier(identifier) if identifier.name == "eval"
                )
        )
    })
}

fn signal(key: &str, value: &str, line: usize, reason: impl Into<String>) -> KeyRemapSignal {
    KeyRemapSignal {
        key: key.to_owned(),
        value: value.to_owned(),
        line,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded_edit_distance, names_are_similar, normalized_name};

    #[test]
    fn name_similarity_understands_casing_delimiters_abbreviations_and_small_typos() {
        assert!(names_are_similar("action_id", "actionId"));
        assert!(names_are_similar("action-identifier", "actionId"));
        assert!(names_are_similar("external_reference", "externalRef"));
        assert!(names_are_similar("acitonId", "actionId"));
        assert!(!names_are_similar("status", "actionId"));
        assert!(!names_are_similar("id", "ids"));
        assert_eq!(normalized_name("ACTION_ID"), "actionid");
        assert_eq!(bounded_edit_distance("action", "aciton", 1), Some(1));
        assert_eq!(bounded_edit_distance("action", "acton", 1), Some(1));
    }
}
