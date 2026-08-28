use crate::model::Language;

struct RuleRemediation {
    rule: &'static str,
    guidance: &'static str,
}

const RULE_REMEDIATIONS: &[RuleRemediation] = &[
    RuleRemediation { rule: "prefer-const", guidance: "Change only bindings proven by semantic reference analysis to have an initializer and no writes after declaration." },
    RuleRemediation { rule: "object-property-shorthand", guidance: "Use identifier shorthand where an ordinary object property repeats the same safe key and value name." },
    RuleRemediation { rule: "redundant-boolean-conditional", guidance: "Replace the literal true/false branches with an explicitly parenthesized boolean coercion while evaluating the condition once." },
    RuleRemediation { rule: "duplicate-type-member", guidance: "Remove exact repeated union or intersection members while preserving the first occurrence and surrounding type semantics." },
    RuleRemediation { rule: "collapsible-if", guidance: "Combine the two strict, else-free nested conditions with short-circuiting AND while retaining the original body." },
    RuleRemediation { rule: "prefer-dot-property", guidance: "Use dot syntax only for a statically known identifier property, retaining optional-chain behavior and avoiding numeric literal ambiguity." },
    RuleRemediation { rule: "prefer-static-object-key", guidance: "Remove computed brackets only from an ordinary static identifier key, preserving computed __proto__ semantics." },
    RuleRemediation { rule: "empty-else", guidance: "Remove an else branch only when its block is structurally empty and contains no comments." },
    RuleRemediation { rule: "redundant-terminal-return", guidance: "Remove an argument-free return only when it is the final direct statement of a function body." },
    RuleRemediation { rule: "redundant-terminal-continue", guidance: "Remove an unlabeled continue only when it is the final direct statement of the current braced loop body." },
    RuleRemediation { rule: "redundant-boolean-return", guidance: "Replace exact true/false return branches with an explicitly parenthesized boolean coercion that evaluates the condition once." },
    RuleRemediation { rule: "unnecessary-empty-statement", guidance: "Remove a standalone empty statement only when it is a direct list item and not serving as a control-flow body." },
    RuleRemediation { rule: "redundant-type-identity", guidance: "Remove never from a union or unknown from an intersection when another member remains and comments do not document the type." },
    RuleRemediation { rule: "duplicate-type-assertion", guidance: "Remove the outer assertion only when adjacent nested assertions use exactly the same source type." },
    RuleRemediation { rule: "duplicate-non-null-assertion", guidance: "Collapse adjacent non-null assertions to one while retaining the asserted expression." },
    RuleRemediation { rule: "jsx-boolean-shorthand", guidance: "Use JSX boolean shorthand only when the explicit attribute expression is the literal true." },
    RuleRemediation { rule: "collapsible-else-if", guidance: "Flatten an else block only when its sole statement is an if and no comments would be displaced." },
    RuleRemediation { rule: "invert-empty-if", guidance: "Invert a direct statement-list condition whose first block is empty and move its non-empty else block into the active branch." },
    RuleRemediation { rule: "empty-finally", guidance: "Remove a structurally empty, comment-free finally clause only when the try statement retains its catch handler." },
    RuleRemediation { rule: "redundant-local-key-remap", guidance: "Rename the closed local property and every statically proven read as one atomic change; do not alter API, serialization, database, reflection, typed, returned, or otherwise observable keys." },
    RuleRemediation { rule: "suspicious-key-remap", guidance: "Decide whether the key is an intentional wire, API, database, framework, or compatibility contract. Preserve it when externally observable; simplify it only after proving every producer and consumer changes together." },
    RuleRemediation { rule: "long-function", guidance: "Extract cohesive named operations around domain steps, keeping data ownership and the public contract explicit." },
    RuleRemediation { rule: "complex-function", guidance: "Flatten control flow with guard clauses and extract decision policies into named, independently testable functions." },
    RuleRemediation { rule: "deep-nesting", guidance: "Replace nested branches with early exits or extracted operations so each scope has one clear responsibility." },
    RuleRemediation { rule: "parameter-bundle", guidance: "Introduce a meaningful parameter object or split responsibilities; do not hide unrelated values in a generic options bag." },
    RuleRemediation { rule: "large-file", guidance: "Split the file along cohesive domain responsibilities and keep the public entrypoint small and explicit." },
    RuleRemediation { rule: "vague-names", guidance: "Rename bindings for their domain meaning and role, avoiding generic placeholders that force readers to infer intent." },
    RuleRemediation { rule: "wrapper-cluster", guidance: "Remove pass-through layers or consolidate them behind one abstraction that owns a real policy or transformation." },
    RuleRemediation { rule: "boolean-soup", guidance: "Name meaningful predicates, simplify equivalent logic, and use a decision table or policy function when combinations encode business rules." },
    RuleRemediation { rule: "else-if-chain", guidance: "Replace the serial chain with named strategies, a lookup table, or an explicit state/dispatch model where appropriate." },
    RuleRemediation { rule: "branch-fanout", guidance: "Partition cases by responsibility or move dispatch to a table/strategy while preserving exhaustive behavior." },
    RuleRemediation { rule: "exit-point-cluster", guidance: "Group validation and failure handling into clear phases, retaining useful guard clauses but reducing scattered termination paths." },
    RuleRemediation { rule: "branch-dense-function", guidance: "Spread compressed decisions into named steps so the main function reads as an ordered workflow." },
    RuleRemediation { rule: "nested-callbacks", guidance: "Extract named callbacks or use a sequential async/control-flow structure that makes execution order visible." },
    RuleRemediation { rule: "nested-ternary", guidance: "Replace nested conditional expressions with named predicates or an explicit statement-level decision." },
    RuleRemediation { rule: "any-cluster", guidance: "Model the actual data shape, use unknown at untrusted boundaries, and narrow it with validation before use." },
    RuleRemediation { rule: "assertion-cluster", guidance: "Move validation to the boundary and derive trusted types so downstream code no longer needs repeated casts or non-null assertions." },
    RuleRemediation { rule: "dependency-cycle", guidance: "Identify the shared policy causing the cycle, move it behind a lower-level contract, and make dependencies point in one direction." },
    RuleRemediation { rule: "module-fanout", guidance: "Split orchestration from domain work and depend on cohesive facades rather than many implementation modules." },
    RuleRemediation { rule: "coupling-hub", guidance: "Separate inbound API responsibilities from outbound orchestration so changes do not converge on one module." },
    RuleRemediation { rule: "barrel-maze", guidance: "Collapse re-export layers and expose a single intentional package boundary close to the implementation." },
    RuleRemediation { rule: "unstable-dependency", guidance: "Invert the dependency through a stable contract owned by the stable module, leaving volatile details behind the boundary." },
    RuleRemediation { rule: "workspace-boundary-bypass", guidance: "Import from the package's declared public export, or deliberately add a supported export without exposing unrelated internals." },
    RuleRemediation { rule: "crowded-folder", guidance: "Group files into cohesive domain subfolders with explicit entrypoints instead of arbitrary size-based buckets." },
    RuleRemediation { rule: "wide-folder", guidance: "Introduce meaningful intermediate domain groupings while avoiding empty taxonomy-only directories." },
    RuleRemediation { rule: "deep-folder-chain", guidance: "Collapse transit-only directories and retain levels only where they separate real sibling choices." },
    RuleRemediation { rule: "wrapper-directory", guidance: "Remove the forwarding directory or move real boundary responsibility into its entrypoint." },
    RuleRemediation { rule: "folder-dependency-cycle", guidance: "Choose an ownership direction between the folders and extract shared contracts into a lower-level cohesive area." },
    RuleRemediation { rule: "folder-coupling-hub", guidance: "Split the folder by responsibility and replace broad internal access with small public boundaries." },
    RuleRemediation { rule: "misplaced-module", guidance: "Move the module beside the code it primarily depends on, or reduce that dependency concentration if its current ownership is intentional." },
    RuleRemediation { rule: "catch-all-folder", guidance: "Partition the generic folder by domain capability and relocate helpers next to their principal consumers." },
    RuleRemediation { rule: "async-without-await", guidance: "Remove unnecessary async semantics, or restore the missing awaited operation while preserving the caller's intended contract." },
    RuleRemediation { rule: "mutation-cluster", guidance: "Replace scattered reassignment with immutable transformations or smaller state transitions whose inputs and outputs are explicit." },
    RuleRemediation { rule: "boolean-parameter-cluster", guidance: "Replace interacting boolean flags with a named mode, enum, discriminated union, or configuration type that rules out invalid combinations." },
    RuleRemediation { rule: "empty-catch", guidance: "Handle, translate, or deliberately propagate the error; if suppression is intentional, document the exact safe failure condition." },
    RuleRemediation { rule: "panic-path-cluster", guidance: "Propagate structured errors or prove invariants at one boundary instead of scattering unwrap, expect, panic, todo, or unreachable paths." },
    RuleRemediation { rule: "structural-clone", guidance: "Consolidate the repeated policy behind one domain-owned operation, while keeping genuinely different behavior explicit instead of introducing a generic parameter maze." },
    RuleRemediation { rule: "tangled-chain", guidance: "Break the fluent pipeline into named transformations at changes in cardinality, failure behavior, or responsibility so intermediate intent is inspectable." },
    RuleRemediation { rule: "input-mutation", guidance: "Return an updated value or make mutation an explicit command contract; avoid surprising callers by changing borrowed or passed-in state implicitly." },
    RuleRemediation { rule: "error-laundering", guidance: "Propagate or translate the failure into a typed outcome instead of returning an ordinary default that is indistinguishable from successful empty data." },
    RuleRemediation { rule: "assertionless-test", guidance: "Assert an externally observable outcome or expected failure; remove the test if execution alone cannot distinguish correct from broken behavior." },
    RuleRemediation { rule: "boolean-call-soup", guidance: "Replace positional boolean literals with a named options object, enum, or mode so the call site states its behavior." },
    RuleRemediation { rule: "oversized-hcl-block", guidance: "Split the configuration along independently owned resources or module boundaries, keeping provider-required nested blocks beside their owner." },
    RuleRemediation { rule: "deep-hcl-nesting", guidance: "Flatten nested configuration where the provider schema permits it, and extract repeated or conditional structure into a focused module with an explicit interface." },
    RuleRemediation { rule: "complex-hcl-expression", guidance: "Name intermediate decisions in small cohesive locals or module inputs so the final argument reads as a direct declaration rather than an embedded program." },
    RuleRemediation { rule: "large-hcl-collection", guidance: "Move the collection behind a named local, typed input, data source, or separate data file only when that makes ownership and review boundaries clearer." },
    RuleRemediation { rule: "dynamic-block-cluster", guidance: "Prefer a typed collection passed to a focused child module, or make a small number of concrete nested blocks explicit when their behavior differs." },
    RuleRemediation { rule: "local-value-cluster", guidance: "Partition locals by cohesive domain purpose and remove pass-through aliases; promote real contracts to typed module inputs or outputs." },
    RuleRemediation { rule: "untyped-variable-cluster", guidance: "Add precise Terraform type constraints, using object attributes and optional fields to make the accepted module contract inspectable." },
    RuleRemediation { rule: "undocumented-interface-cluster", guidance: "Add concise descriptions that state intent, units, constraints, and sensitive behavior for public variables and outputs." },
    RuleRemediation { rule: "floating-module-source", guidance: "Pin registry modules with a version constraint and Git or Terragrunt sources with an immutable tag or commit ref, then review the resulting plan." },
    RuleRemediation { rule: "broad-ignore-changes", guidance: "List only externally managed attributes and document the ownership reason; remove the lifecycle suppression if Terraform should reconcile the value." },
    RuleRemediation { rule: "explicit-dependency-cluster", guidance: "Prefer direct expression references so Terraform can infer precise edges; retain depends_on only for documented hidden behavioral dependencies." },
    RuleRemediation { rule: "terragrunt-dependency-cluster", guidance: "Introduce a smaller orchestration boundary or stack and pass only the required outputs instead of coordinating many units from one configuration." },
    RuleRemediation { rule: "terragrunt-hook-cluster", guidance: "Move substantial imperative behavior into a named, tested script or pipeline step and keep only the minimal lifecycle integration in Terragrunt." },
    RuleRemediation { rule: "terragrunt-config-read-cluster", guidance: "Consolidate shared configuration behind one clearly owned include or input contract so readers do not reconstruct values from many files." },
    RuleRemediation { rule: "terragrunt-include-cluster", guidance: "Reduce inheritance layers and expose one intentional shared configuration boundary, keeping unit-specific values local and explicit." },
];

pub fn finding_prompt(rule: &str, path: &str, line: usize, evidence: &str) -> String {
    let guidance = RULE_REMEDIATIONS
        .iter()
        .find(|entry| entry.rule == rule)
        .map_or(
            "Reduce the measured hotspot with the smallest behavior-preserving refactor.",
            |entry| entry.guidance,
        );
    format!(
        "Review `{path}:{line}` for `{rule}`. Evidence: {evidence}. {guidance} Preserve observable behavior, add or update focused tests, and explain the resulting trade-offs."
    )
}

pub fn parser_prompt(path: &str, language: Language, count: usize) -> String {
    let parser = match language {
        Language::TypeScript => "TypeScript/TSX parser",
        Language::Rust => "Rust parser",
        Language::Terraform => "Terraform HCL parser",
        Language::Terragrunt => "Terragrunt HCL parser",
    };
    format!(
        "Repair the {count} syntax error(s) reported by the {parser} in `{path}`. First reproduce them with the project's normal formatter or compiler, make the smallest syntax-preserving correction, then rerun Slop and the relevant tests. Do not silence or exclude the file."
    )
}

pub fn fatal_prompt(error: &anyhow::Error) -> String {
    format!(
        "Diagnose this Slop CLI failure: `{error:#}`. Verify the target path, permissions, UTF-8 source, and command arguments; fix the underlying cause rather than bypassing discovery, then rerun the same command."
    )
}
