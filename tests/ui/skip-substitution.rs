/// Compile-fail test: `write_draft()` must not be callable on
/// `WizardBuilder<MappingBuilt>` — substitution must happen first.
use opencode_config::wizard_builder::{MappingBuilt, WizardBuilder};

fn must_not_compile(builder: WizardBuilder<'_, MappingBuilt>) {
    let _ = builder.write_draft();
}

fn main() {}
