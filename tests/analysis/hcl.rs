use super::*;

#[test]
fn terraform_and_terragrunt_paths_are_discovered_without_configuration() {
    assert_eq!(
        Language::from_path(Path::new("infra/main.tf")),
        Some(Language::Terraform)
    );
    assert_eq!(
        Language::from_path(Path::new("infra/prod.auto.tfvars")),
        Some(Language::Terraform)
    );
    assert_eq!(
        Language::from_path(Path::new("live/prod/terragrunt.hcl")),
        Some(Language::Terragrunt)
    );
    assert_eq!(
        Language::from_path(Path::new("live/root.hcl")),
        Some(Language::Terragrunt)
    );
    assert_eq!(Language::from_path(Path::new(".terraform.lock.hcl")), None);
}

#[test]
fn clean_terraform_has_an_explicit_module_contract() {
    let source = r#"
variable "region" {
  description = "Cloud region for this deployment"
  type        = string
}

module "network" {
  source = "./modules/network"
  region = var.region
}

resource "aws_instance" "api" {
  for_each      = var.instances
  ami           = each.value.ami
  instance_type = each.value.instance_type
}
"#;
    let analysis = analyze_inline("infra/main.tf", source);

    assert_eq!(analysis.language, Language::Terraform);
    assert_eq!(analysis.parse_errors, 0);
    assert_eq!(analysis.metrics.terraform_variables, 1);
    assert_eq!(analysis.metrics.terraform_resources, 1);
    assert!(analysis.findings.is_empty(), "{:?}", analysis.findings);
}

#[test]
fn terraform_finds_interface_flow_lifecycle_and_source_slop() {
    let variables = (0..5)
        .map(|index| format!("variable \"input_{index}\" {{}}"))
        .collect::<Vec<_>>()
        .join("\n");
    let locals = (0..31)
        .map(|index| format!("  setting_{index} = \"value-{index}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let attributes = (0..82)
        .map(|index| format!("  setting_{index} = {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let source = format!(
        r#"
{variables}

module "network" {{
  source = "git::https://github.com/acme/network.git"
}}

locals {{
{locals}
}}

resource "example_service" "large" {{
{attributes}
  depends_on = [
    module.a,
    module.b,
    module.c,
    module.d,
    module.e,
  ]
  lifecycle {{
    ignore_changes = all
  }}
  dynamic "first" {{ content {{ value = 1 }} }}
  dynamic "second" {{ content {{ value = 2 }} }}
  dynamic "third" {{ content {{ value = 3 }} }}
}}
"#
    );
    let analysis = analyze_inline("infra/sloppy.tf", &source);

    assert_eq!(analysis.parse_errors, 0);
    for expected in [
        "untyped-variable-cluster",
        "undocumented-interface-cluster",
        "floating-module-source",
        "local-value-cluster",
        "oversized-hcl-block",
        "explicit-dependency-cluster",
        "broad-ignore-changes",
        "dynamic-block-cluster",
    ] {
        assert!(
            has_rule(&analysis, expected),
            "missing {expected}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn terraform_finds_nested_expression_and_collection_flow() {
    let collection = (0..30)
        .map(|index| format!("\"service-{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!(
        r#"
resource "example_service" "flow" {{
  selected = var.first ? (var.second ? (var.third ? (var.fourth ? (var.fifth ? "a" : "b") : "c") : "d") : "e") : "f"
  services = [{collection}]

  first {{
    second {{
      third {{
        fourth {{
          enabled = true
        }}
      }}
    }}
  }}
}}
"#
    );
    let analysis = analyze_inline("infra/flow.tf", &source);

    assert_eq!(analysis.parse_errors, 0);
    for expected in [
        "complex-hcl-expression",
        "large-hcl-collection",
        "deep-hcl-nesting",
    ] {
        assert!(
            has_rule(&analysis, expected),
            "missing {expected}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn hcl_rules_stay_quiet_below_cluster_boundaries() {
    let source = r#"
variable "first" {}
variable "second" {}

module "network" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "~> 6.0"
}

resource "example_service" "bounded" {
  depends_on = [module.a, module.b, module.c, module.d]
  lifecycle {
    ignore_changes = [tags]
  }
  dynamic "first" { content { value = 1 } }
  dynamic "second" { content { value = 2 } }
}
"#;
    let analysis = analyze_inline("infra/bounded.tf", source);

    assert_eq!(analysis.parse_errors, 0);
    for rule in [
        "untyped-variable-cluster",
        "undocumented-interface-cluster",
        "floating-module-source",
        "explicit-dependency-cluster",
        "broad-ignore-changes",
        "dynamic-block-cluster",
    ] {
        assert!(
            !has_rule(&analysis, rule),
            "unexpected {rule}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn terragrunt_finds_hidden_orchestration_flow() {
    let dependencies = (0..5)
        .map(|index| format!("dependency \"unit_{index}\" {{ config_path = \"../unit-{index}\" }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let includes = (0..4)
        .map(|index| format!("include \"layer_{index}\" {{ path = \"../layer-{index}.hcl\" }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let source = format!(
        r#"
terraform {{
  source = "git::https://github.com/acme/infrastructure.git//service"
  before_hook "prepare" {{
    commands = ["plan"]
    execute  = ["./prepare.sh"]
  }}
  after_hook "notify" {{
    commands = ["apply"]
    execute  = ["./notify.sh"]
  }}
  error_hook "recover" {{
    commands  = ["apply"]
    execute   = ["./recover.sh"]
    on_errors = [".*"]
  }}
}}

locals {{
  account = read_terragrunt_config("../account.hcl")
  region  = read_terragrunt_config("../region.hcl")
  team    = read_terragrunt_config("../team.hcl")
  policy  = read_terragrunt_config("../policy.hcl")
}}

{dependencies}
{includes}
"#
    );
    let analysis = analyze_inline("live/prod/terragrunt.hcl", &source);

    assert_eq!(analysis.language, Language::Terragrunt);
    assert_eq!(analysis.parse_errors, 0);
    for expected in [
        "floating-module-source",
        "terragrunt-dependency-cluster",
        "terragrunt-hook-cluster",
        "terragrunt-config-read-cluster",
        "terragrunt-include-cluster",
    ] {
        assert!(
            has_rule(&analysis, expected),
            "missing {expected}: {:?}",
            analysis.findings
        );
    }
}

#[test]
fn terragrunt_local_dependencies_feed_the_shared_cycle_graph() {
    let report = analyze_repository(vec![
        (
            "live/a/terragrunt.hcl".to_owned(),
            "dependency \"b\" { config_path = \"../b\" }".to_owned(),
        ),
        (
            "live/b/terragrunt.hcl".to_owned(),
            "dependency \"a\" { config_path = \"../a\" }".to_owned(),
        ),
    ]);

    assert!(report_has_rule(&report, "dependency-cycle"));
    assert!(report_has_rule(&report, "folder-dependency-cycle"));
}

#[test]
fn terraform_blocks_participate_in_structural_clone_detection() {
    let block = |resource: &str, name: &str| {
        format!(
            r#"resource "{resource}" "{name}" {{
  name        = "{name}"
  description = "managed service"
  enabled     = true
  settings = {{
    retries = 3
    timeout = 30
    mode    = "active"
  }}
  tags = {{
    owner       = "platform"
    environment = "production"
    managed_by  = "terraform"
  }}
  retry_policy = {{
    attempts = 5
    backoff  = "exponential"
  }}
  health_check = {{
    path     = "/health"
    interval = 30
  }}
}}"#
        )
    };
    let report = analyze_repository(vec![
        ("infra/first.tf".to_owned(), block("service_a", "api")),
        ("infra/second.tf".to_owned(), block("service_b", "worker")),
    ]);

    assert!(report_has_rule(&report, "structural-clone"));
}
