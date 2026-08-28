use crate::model::{Category, Finding};

use super::core::FunctionMetrics;

const VAGUE_NAME_LINES: usize = 8;
const BOUNDARY_NAME_LINES: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NounClass {
    Vague,
    Boundary,
}

struct GenericName {
    verb: String,
    noun: String,
    class: NounClass,
}

struct WorkThresholds {
    lines: usize,
    decisions: usize,
    complexity: usize,
    mutations: usize,
    chain_steps: usize,
}

pub(super) fn assess(path: &str, function: &FunctionMetrics, findings: &mut Vec<Finding>) {
    if function.test_context || function.thin_wrapper {
        return;
    }
    let Some(name) = classify(&function.name) else {
        return;
    };
    if !hides_substantial_work(function, name.class) {
        return;
    }
    findings.push(Finding::new(
        "generic-function-name",
        Category::Readability,
        debt_points(function),
        (path.to_owned(), function.line),
        (
            format!(
                "`{}` hides domain behaviour behind a generic name",
                function.name
            ),
            format!(
                "generic verb `{}` and noun `{}` describe {} lines with {} decision point(s)",
                name.verb, name.noun, function.lines, function.decision_points
            ),
        ),
    ));
}

fn hides_substantial_work(function: &FunctionMetrics, class: NounClass) -> bool {
    let thresholds = match class {
        NounClass::Vague => WorkThresholds {
            lines: VAGUE_NAME_LINES,
            decisions: 3,
            complexity: 4,
            mutations: 3,
            chain_steps: 5,
        },
        NounClass::Boundary => WorkThresholds {
            lines: BOUNDARY_NAME_LINES,
            decisions: 6,
            complexity: 8,
            mutations: 6,
            chain_steps: 8,
        },
    };
    let observations = [
        (function.lines, thresholds.lines),
        (function.decision_points, thresholds.decisions),
        (function.cognitive_complexity, thresholds.complexity),
        (function.mutation_points, thresholds.mutations),
        (function.max_chain_steps, thresholds.chain_steps),
    ];
    observations
        .iter()
        .any(|(observed, threshold)| observed >= threshold)
}

fn debt_points(function: &FunctionMetrics) -> f64 {
    (2.5 + function.lines.saturating_sub(VAGUE_NAME_LINES) as f64 / 20.0
        + function.decision_points as f64 * 0.25)
        .min(8.0)
}

fn classify(name: &str) -> Option<GenericName> {
    let mut words = identifier_words(name);
    if words.first().is_some_and(|word| word == "async") {
        words.remove(0);
    }
    if words.last().is_some_and(|word| word == "async") {
        words.pop();
    }
    if words.len() != 2 || !is_generic_verb(&words[0]) {
        return None;
    }
    let class = noun_class(&words[1])?;
    Some(GenericName {
        verb: words.remove(0),
        noun: words.remove(0),
        class,
    })
}

fn is_generic_verb(word: &str) -> bool {
    matches!(
        word,
        "do" | "execute" | "handle" | "manage" | "perform" | "process" | "run"
    )
}

fn noun_class(word: &str) -> Option<NounClass> {
    match word {
        "data" | "info" | "information" | "input" | "inputs" | "item" | "items" | "object"
        | "objects" | "operation" | "operations" | "output" | "outputs" | "record" | "records"
        | "result" | "results" | "stuff" | "task" | "tasks" | "thing" | "things" | "value"
        | "values" | "work" => Some(NounClass::Vague),
        "command" | "commands" | "event" | "events" | "message" | "messages" | "payload"
        | "payloads" | "request" | "requests" | "response" | "responses" => {
            Some(NounClass::Boundary)
        }
        _ => None,
    }
}

fn identifier_words(name: &str) -> Vec<String> {
    let characters = name.trim_start_matches("r#").chars().collect::<Vec<_>>();
    let mut words = Vec::new();
    let mut current = String::new();
    for (index, character) in characters.iter().copied().enumerate() {
        if !character.is_ascii_alphanumeric() {
            push_word(&mut words, &mut current);
            continue;
        }
        if starts_word(&characters, index, &current) {
            push_word(&mut words, &mut current);
        }
        current.push(character.to_ascii_lowercase());
    }
    push_word(&mut words, &mut current);
    words
}

fn starts_word(characters: &[char], index: usize, current: &str) -> bool {
    if current.is_empty() || !characters[index].is_ascii_uppercase() {
        return false;
    }
    let previous = characters[index - 1];
    let next_is_lowercase = characters
        .get(index + 1)
        .is_some_and(char::is_ascii_lowercase);
    previous.is_ascii_lowercase()
        || previous.is_ascii_digit()
        || (previous.is_ascii_uppercase() && next_is_lowercase)
}

fn push_word(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, identifier_words, NounClass};

    #[test]
    fn generic_names_are_split_across_supported_identifier_styles() {
        assert_eq!(identifier_words("processData"), ["process", "data"]);
        assert_eq!(identifier_words("handle_request"), ["handle", "request"]);
        assert_eq!(identifier_words("HTTPResponse"), ["http", "response"]);
        assert_eq!(identifier_words("r#do_work"), ["do", "work"]);
    }

    #[test]
    fn classifier_requires_a_wholly_generic_verb_noun_pair() {
        assert_eq!(
            classify("processData").map(|name| name.class),
            Some(NounClass::Vague)
        );
        assert_eq!(
            classify("handleRequest").map(|name| name.class),
            Some(NounClass::Boundary)
        );
        assert!(classify("processInvoiceData").is_none());
        assert!(classify("calculateInvoiceTotal").is_none());
        assert!(classify("handler").is_none());
    }
}
