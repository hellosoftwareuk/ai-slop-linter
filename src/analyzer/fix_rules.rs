use crate::model::{Category, Finding};

use super::core::Facts;

struct SafeFixRule {
    rule: &'static str,
    category: Category,
    message: &'static str,
    noun: &'static str,
}

const SAFE_FIX_RULES: &[SafeFixRule] = &[
    SafeFixRule {
        rule: "prefer-const",
        category: Category::Readability,
        message: "Immutable bindings are declared as mutable",
        noun: "binding",
    },
    SafeFixRule {
        rule: "object-property-shorthand",
        category: Category::Readability,
        message: "Object properties repeat their binding names",
        noun: "property",
    },
    SafeFixRule {
        rule: "redundant-boolean-conditional",
        category: Category::Readability,
        message: "Conditional expressions restate boolean values",
        noun: "conditional",
    },
    SafeFixRule {
        rule: "duplicate-type-member",
        category: Category::TypeSafety,
        message: "Type expressions repeat identical members",
        noun: "type expression",
    },
    SafeFixRule {
        rule: "collapsible-if",
        category: Category::Complexity,
        message: "Nested if statements can express one decision",
        noun: "if statement",
    },
    SafeFixRule {
        rule: "prefer-dot-property",
        category: Category::Readability,
        message: "Static property access uses noisy bracket syntax",
        noun: "property access",
    },
    SafeFixRule {
        rule: "prefer-static-object-key",
        category: Category::Readability,
        message: "Static object keys use unnecessary computed syntax",
        noun: "object key",
    },
    SafeFixRule {
        rule: "empty-else",
        category: Category::Readability,
        message: "Empty else blocks create a branch with no behavior",
        noun: "else block",
    },
    SafeFixRule {
        rule: "redundant-terminal-return",
        category: Category::Readability,
        message: "Terminal empty returns restate function fallthrough",
        noun: "return statement",
    },
    SafeFixRule {
        rule: "redundant-terminal-continue",
        category: Category::Readability,
        message: "Terminal continue statements restate loop flow",
        noun: "continue statement",
    },
    SafeFixRule {
        rule: "redundant-boolean-return",
        category: Category::Readability,
        message: "If branches restate a boolean return value",
        noun: "if statement",
    },
    SafeFixRule {
        rule: "unnecessary-empty-statement",
        category: Category::Readability,
        message: "Empty statements add punctuation without behavior",
        noun: "empty statement",
    },
    SafeFixRule {
        rule: "redundant-type-identity",
        category: Category::TypeSafety,
        message: "Type expressions include identity members",
        noun: "type member",
    },
    SafeFixRule {
        rule: "duplicate-type-assertion",
        category: Category::TypeSafety,
        message: "Nested type assertions repeat the same type",
        noun: "type assertion",
    },
    SafeFixRule {
        rule: "duplicate-non-null-assertion",
        category: Category::TypeSafety,
        message: "Non-null assertions are repeated",
        noun: "non-null assertion",
    },
    SafeFixRule {
        rule: "jsx-boolean-shorthand",
        category: Category::Readability,
        message: "JSX boolean props explicitly restate true",
        noun: "JSX prop",
    },
    SafeFixRule {
        rule: "collapsible-else-if",
        category: Category::Complexity,
        message: "Else blocks wrap a single if statement",
        noun: "else block",
    },
    SafeFixRule {
        rule: "invert-empty-if",
        category: Category::Readability,
        message: "Empty first branches hide the branch that does work",
        noun: "if statement",
    },
    SafeFixRule {
        rule: "empty-finally",
        category: Category::Readability,
        message: "Empty finally blocks add no cleanup behavior",
        noun: "finally block",
    },
];

pub(super) fn assess(path: &str, facts: &Facts, findings: &mut Vec<Finding>) {
    for specification in SAFE_FIX_RULES {
        let mut matching = facts
            .proposed_fixes
            .iter()
            .filter(|candidate| candidate.rule == specification.rule);
        let Some(first) = matching.next() else {
            continue;
        };
        let count = 1 + matching.count();
        let (noun, suffix) = plural_noun(specification.noun, count);
        findings.push(
            Finding::new(
                specification.rule,
                specification.category,
                (1.0 + count.saturating_sub(1) as f64 * 0.25).min(4.0),
                (path.to_owned(), first.line),
                (
                    specification.message,
                    format!(
                        "{count} {}{} can be rewritten without changing behavior",
                        noun, suffix
                    ),
                ),
            )
            .with_fixable(),
        );
    }
}

fn plural_noun(noun: &str, count: usize) -> (&str, &'static str) {
    if count == 1 {
        return (noun, "");
    }
    match noun {
        "property" => ("properties", ""),
        "property access" => ("property accesses", ""),
        _ => (noun, "s"),
    }
}

#[cfg(test)]
mod tests {
    use super::plural_noun;

    #[test]
    fn finding_evidence_pluralizes_property_terms() {
        assert_eq!(plural_noun("property", 2), ("properties", ""));
        assert_eq!(plural_noun("property access", 2), ("property accesses", ""));
        assert_eq!(plural_noun("binding", 2), ("binding", "s"));
    }
}
