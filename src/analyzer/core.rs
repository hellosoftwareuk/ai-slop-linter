use crate::model::{CloneCandidate, ModuleDependency, ProposedFix};

#[derive(Debug)]
pub(super) struct KeyRemapSignal {
    pub key: String,
    pub value: String,
    pub line: usize,
    pub reason: String,
}

#[derive(Debug)]
pub(super) struct HclBlockMetrics {
    pub block_type: String,
    pub label: String,
    pub line: usize,
    pub lines: usize,
    pub depth: usize,
    pub attributes: usize,
    pub nested_blocks: usize,
    pub dynamic_blocks: usize,
    pub max_expression_complexity: usize,
    pub expression_line: usize,
    pub max_collection_items: usize,
    pub collection_line: usize,
}

#[derive(Debug)]
pub(super) struct FunctionMetrics {
    pub name: String,
    pub line: usize,
    pub lines: usize,
    pub parameters: usize,
    pub boolean_parameters: usize,
    pub async_function: bool,
    pub await_points: usize,
    pub mutation_points: usize,
    pub panic_paths: usize,
    pub max_chain_steps: usize,
    pub max_chain_callbacks: usize,
    pub input_mutations: usize,
    pub input_mutation_threshold: usize,
    pub test_function: bool,
    pub assertions: usize,
    pub cognitive_complexity: usize,
    pub max_nesting: usize,
    pub nested_conditionals: usize,
    pub decision_points: usize,
    pub max_boolean_operators: usize,
    pub max_branch_fanout: usize,
    pub max_else_if_chain: usize,
    pub exit_points: usize,
    pub anonymous_depth: usize,
    pub thin_wrapper: bool,
}

#[derive(Debug)]
struct FunctionFrame {
    name: String,
    line: usize,
    lines: usize,
    parameters: usize,
    boolean_parameters: usize,
    async_function: bool,
    await_points: usize,
    mutation_points: usize,
    panic_paths: usize,
    max_chain_steps: usize,
    max_chain_callbacks: usize,
    input_mutations: usize,
    input_mutation_threshold: usize,
    test_function: bool,
    assertions: usize,
    cognitive_complexity: usize,
    nesting: usize,
    max_nesting: usize,
    conditional_depth: usize,
    nested_conditionals: usize,
    decision_points: usize,
    boolean_depth: usize,
    current_boolean_operators: usize,
    max_boolean_operators: usize,
    max_branch_fanout: usize,
    max_else_if_chain: usize,
    exit_points: usize,
    anonymous_depth: usize,
    thin_wrapper: bool,
    has_body: bool,
}

#[derive(Debug, Default)]
pub(super) struct Facts {
    pub functions: Vec<FunctionMetrics>,
    pub any_locations: Vec<usize>,
    pub assertion_locations: Vec<usize>,
    pub empty_catches: Vec<usize>,
    pub error_laundering: Vec<usize>,
    pub boolean_literal_calls: Vec<(usize, usize)>,
    pub vague_bindings: Vec<(String, usize)>,
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
    pub dependencies: Vec<ModuleDependency>,
    pub clone_candidates: Vec<CloneCandidate>,
    pub top_level_statements: usize,
    pub hcl_blocks: Vec<HclBlockMetrics>,
    pub untyped_variables: Vec<(String, usize)>,
    pub undocumented_interfaces: Vec<(String, usize)>,
    pub floating_sources: Vec<(String, usize)>,
    pub broad_ignore_changes: Vec<usize>,
    pub wide_explicit_dependencies: Vec<(usize, usize)>,
    pub terragrunt_hooks: Vec<usize>,
    pub terragrunt_dependencies: Vec<usize>,
    pub terragrunt_config_reads: Vec<usize>,
    pub terragrunt_includes: Vec<usize>,
    pub proposed_fixes: Vec<ProposedFix>,
    pub key_remaps: Vec<KeyRemapSignal>,
}

#[derive(Debug, Default)]
pub(super) struct MetricCollector {
    frames: Vec<FunctionFrame>,
    pub facts: Facts,
}

pub(super) struct FunctionInput {
    pub name: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub parameters: usize,
    pub boolean_parameters: usize,
    pub async_function: bool,
    pub test_function: bool,
    pub input_mutation_threshold: usize,
    pub thin_wrapper: bool,
    pub has_body: bool,
}

impl MetricCollector {
    pub fn start_function(&mut self, input: FunctionInput) {
        let anonymous_depth = if input.name.is_none() {
            self.frames
                .last()
                .map_or(1, |parent| parent.anonymous_depth + 1)
        } else {
            0
        };
        self.frames.push(FunctionFrame {
            name: input
                .name
                .unwrap_or_else(|| format!("anonymous@{}", input.start_line)),
            line: input.start_line,
            lines: input.end_line.saturating_sub(input.start_line) + 1,
            parameters: input.parameters,
            boolean_parameters: input.boolean_parameters,
            async_function: input.async_function,
            await_points: 0,
            mutation_points: 0,
            panic_paths: 0,
            max_chain_steps: 0,
            max_chain_callbacks: 0,
            input_mutations: 0,
            input_mutation_threshold: input.input_mutation_threshold,
            test_function: input.test_function,
            assertions: 0,
            cognitive_complexity: 0,
            nesting: 0,
            max_nesting: 0,
            conditional_depth: 0,
            nested_conditionals: 0,
            decision_points: 0,
            boolean_depth: 0,
            current_boolean_operators: 0,
            max_boolean_operators: 0,
            max_branch_fanout: 0,
            max_else_if_chain: 0,
            exit_points: 0,
            anonymous_depth,
            thin_wrapper: input.thin_wrapper,
            has_body: input.has_body,
        });
    }

    pub fn finish_function(&mut self) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        if !frame.has_body {
            return;
        }
        self.facts.functions.push(FunctionMetrics {
            name: frame.name,
            line: frame.line,
            lines: frame.lines,
            parameters: frame.parameters,
            boolean_parameters: frame.boolean_parameters,
            async_function: frame.async_function,
            await_points: frame.await_points,
            mutation_points: frame.mutation_points,
            panic_paths: frame.panic_paths,
            max_chain_steps: frame.max_chain_steps,
            max_chain_callbacks: frame.max_chain_callbacks,
            input_mutations: frame.input_mutations,
            input_mutation_threshold: frame.input_mutation_threshold,
            test_function: frame.test_function,
            assertions: frame.assertions,
            cognitive_complexity: frame.cognitive_complexity,
            max_nesting: frame.max_nesting,
            nested_conditionals: frame.nested_conditionals,
            decision_points: frame.decision_points,
            max_boolean_operators: frame.max_boolean_operators,
            max_branch_fanout: frame.max_branch_fanout,
            max_else_if_chain: frame.max_else_if_chain,
            exit_points: frame.exit_points,
            anonymous_depth: frame.anonymous_depth,
            thin_wrapper: frame.thin_wrapper,
        });
    }

    pub fn enter_control_flow(&mut self, nesting: bool, conditional_expression: bool) {
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        frame.cognitive_complexity += 1 + usize::from(nesting) * frame.nesting;
        frame.decision_points += 1;
        if conditional_expression {
            if frame.conditional_depth > 0 {
                frame.nested_conditionals += 1;
            }
            frame.conditional_depth += 1;
        }
        if nesting {
            frame.nesting += 1;
            frame.max_nesting = frame.max_nesting.max(frame.nesting);
        }
    }

    pub fn enter_boolean_operator(&mut self) {
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        if frame.boolean_depth == 0 {
            frame.current_boolean_operators = 0;
        }
        frame.boolean_depth += 1;
        frame.current_boolean_operators += 1;
        frame.max_boolean_operators = frame
            .max_boolean_operators
            .max(frame.current_boolean_operators);
        frame.cognitive_complexity += 1;
        frame.decision_points += 1;
    }

    pub fn leave_boolean_operator(&mut self) {
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        frame.boolean_depth = frame.boolean_depth.saturating_sub(1);
        if frame.boolean_depth == 0 {
            frame.current_boolean_operators = 0;
        }
    }

    pub fn record_branch_fanout(&mut self, branches: usize) {
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        frame.max_branch_fanout = frame.max_branch_fanout.max(branches);
        // The switch or match node itself contributes one decision in
        // `enter_control_flow`; the remaining arms are additional choices.
        frame.decision_points += branches.saturating_sub(1);
    }

    pub fn record_else_if_chain(&mut self, conditions: usize) {
        if let Some(frame) = self.frames.last_mut() {
            frame.max_else_if_chain = frame.max_else_if_chain.max(conditions);
        }
    }

    pub fn record_exit_point(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.exit_points += 1;
        }
    }

    pub fn record_await_point(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.await_points += 1;
        }
    }

    pub fn record_mutation(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.mutation_points += 1;
        }
    }

    pub fn record_panic_path(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.panic_paths += 1;
        }
    }

    pub fn record_chain(&mut self, steps: usize, callbacks: usize) {
        if let Some(frame) = self.frames.last_mut() {
            if steps > frame.max_chain_steps
                || (steps == frame.max_chain_steps && callbacks > frame.max_chain_callbacks)
            {
                frame.max_chain_steps = steps;
                frame.max_chain_callbacks = callbacks;
            }
        }
    }

    pub fn record_input_mutation(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.input_mutations += 1;
        }
    }

    pub fn record_assertion(&mut self) {
        if let Some(frame) = self
            .frames
            .iter_mut()
            .rev()
            .find(|frame| frame.test_function)
        {
            frame.assertions += 1;
        }
    }

    pub fn leave_control_flow(&mut self, nesting: bool, conditional_expression: bool) {
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        if nesting {
            frame.nesting = frame.nesting.saturating_sub(1);
        }
        if conditional_expression {
            frame.conditional_depth = frame.conditional_depth.saturating_sub(1);
        }
    }
}

pub(super) fn is_vague_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "data" | "item" | "value" | "result" | "temp" | "tmp" | "thing" | "stuff" | "obj" | "info"
    )
}

pub(super) struct ParsedFacts {
    pub facts: Facts,
    pub parse_errors: usize,
}

pub(super) struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut starts = Vec::with_capacity(source.len() / 32 + 1);
        starts.push(0);
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index as u32 + 1)),
        );
        Self { starts }
    }

    pub fn line(&self, offset: u32) -> usize {
        self.starts.partition_point(|start| *start <= offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_maps_offsets() {
        let index = LineIndex::new("one\ntwo\nthree");
        assert_eq!(index.line(0), 1);
        assert_eq!(index.line(4), 2);
        assert_eq!(index.line(8), 3);
    }

    #[test]
    fn vague_names_are_deliberately_narrow() {
        assert!(is_vague_name("data"));
        assert!(is_vague_name("RESULT"));
        assert!(!is_vague_name("order"));
        assert!(!is_vague_name("i"));
    }
}
