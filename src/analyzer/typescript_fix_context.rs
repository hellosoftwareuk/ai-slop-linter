use oxc_ast::ast::Comment;
use oxc_span::Span;

use crate::model::ProposedFix;

use super::core::LineIndex;

pub(super) struct FixContext<'source, 'ast> {
    source: &'source str,
    lines: LineIndex,
    comments: &'ast [Comment],
    fixes: Vec<ProposedFix>,
}

impl<'source, 'ast> FixContext<'source, 'ast> {
    pub(super) fn new(source: &'source str, comments: &'ast [Comment]) -> Self {
        Self {
            source,
            lines: LineIndex::new(source),
            comments,
            fixes: Vec::new(),
        }
    }

    pub(super) fn propose(&mut self, rule: &'static str, span: Span, replacement: String) {
        if span.is_empty()
            || self.has_comment(span)
            || replacement == self.slice(span).unwrap_or("")
        {
            return;
        }
        let Some(expected) = self.slice(span).map(str::to_owned) else {
            return;
        };
        self.fixes.push(ProposedFix {
            rule,
            start: span.start as usize,
            end: span.end as usize,
            expected,
            replacement,
            line: self.lines.line(span.start),
        });
    }

    pub(super) fn slice(&self, span: Span) -> Option<&'source str> {
        self.source.get(span.start as usize..span.end as usize)
    }

    pub(super) fn has_comment(&self, span: Span) -> bool {
        self.comments
            .iter()
            .any(|comment| comment.span.start < span.end && comment.span.end > span.start)
    }

    pub(super) fn remove_rule(&mut self, rule: &str) {
        self.fixes.retain(|candidate| candidate.rule != rule);
    }

    pub(super) fn into_fixes(self) -> Vec<ProposedFix> {
        self.fixes
    }
}
