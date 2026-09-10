//! Saved explanation preferences, independently versioned from execution templates.
use domain::prompt_template::AgentPromptTemplate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const PROMPT_PRESET_CATALOG_SCHEMA_VERSION: u32 = 2;
pub const MAX_PROMPT_PRESETS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BundledPromptPresetSource {
    pub id: String,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptPreset {
    pub id: String,
    pub name: String,
    pub revision: u32,
    pub template: AgentPromptTemplate,
    pub bundled_source: Option<BundledPromptPresetSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PromptPresetCatalog {
    pub schema_version: u32,
    pub revision: u32,
    pub execution_revision: u32,
    pub selected_id: String,
    pub presets: Vec<PromptPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PromptPresetMutation {
    Create {
        name: String,
        template: AgentPromptTemplate,
    },
    Update {
        id: String,
        expected_revision: u32,
        name: String,
        template: AgentPromptTemplate,
    },
    Delete {
        id: String,
        expected_revision: u32,
    },
    Select {
        id: String,
    },
    ResetAll {
        expected_catalog_revision: u32,
    },
}

impl Default for PromptPresetCatalog {
    fn default() -> Self {
        let presets = bundled_presets();
        Self {
            schema_version: PROMPT_PRESET_CATALOG_SCHEMA_VERSION,
            revision: 1,
            execution_revision: 1,
            selected_id: presets[0].id.clone(),
            presets,
        }
    }
}

impl PromptPresetCatalog {
    pub fn selected(&self) -> &PromptPreset {
        self.presets
            .iter()
            .find(|preset| preset.id == self.selected_id)
            .expect("validated catalog has a selected preset")
    }

    pub fn normalize(mut self) -> Result<Self, String> {
        for preset in &mut self.presets {
            preset.template = preset.template.clone().normalize()?;
        }
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PROMPT_PRESET_CATALOG_SCHEMA_VERSION
            || self.revision == 0
            || self.execution_revision == 0
        {
            return Err("Unsupported prompt preset catalog version or revision".into());
        }
        if self.presets.is_empty() || self.presets.len() > MAX_PROMPT_PRESETS {
            return Err(format!(
                "Keep between 1 and {MAX_PROMPT_PRESETS} prompt presets"
            ));
        }
        let mut ids = BTreeSet::new();
        for preset in &self.presets {
            if preset.id.is_empty()
                || preset.id.len() > 80
                || !preset
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                || !ids.insert(&preset.id)
            {
                return Err("Prompt preset IDs must be unique bounded identifiers".into());
            }
            if preset.revision == 0
                || preset.name.trim().is_empty()
                || preset.name.chars().count() > 80
                || preset.name.chars().any(char::is_control)
            {
                return Err("Prompt preset name or revision is invalid".into());
            }
            if let Some(source) = &preset.bundled_source {
                if source.id != preset.id
                    || source.version == 0
                    || ![
                        "visual-learner",
                        "conceptual-learner",
                        "practical-learner",
                        "analytical-learner",
                    ]
                    .contains(&source.id.as_str())
                {
                    return Err("Invalid bundled prompt preset source".into());
                }
            }
            preset.template.validate()?;
        }
        if !ids.contains(&self.selected_id) {
            return Err("Selected prompt preset does not exist".into());
        }
        Ok(())
    }

    /// Produces a fully validated next value; failures never partially modify the source.
    pub fn apply(&self, mutation: PromptPresetMutation) -> Result<Self, String> {
        self.apply_with_creation_id(mutation, None)
    }

    /// Random identity allocation belongs to the desktop host, never this pure usecase.
    pub fn apply_with_creation_id(
        &self,
        mutation: PromptPresetMutation,
        creation_id: Option<Uuid>,
    ) -> Result<Self, String> {
        self.validate()?;
        let mut next = self.clone();
        match mutation {
            PromptPresetMutation::Create { name, template } => {
                next.presets.push(PromptPreset {
                    id: creation_id
                        .filter(|id| !id.is_nil())
                        .ok_or("A new preset requires a host-allocated identity")?
                        .to_string(),
                    name,
                    revision: 1,
                    template,
                    bundled_source: None,
                });
            }
            PromptPresetMutation::Update {
                id,
                expected_revision,
                name,
                template,
            } => {
                let preset = next
                    .presets
                    .iter_mut()
                    .find(|preset| preset.id == id)
                    .ok_or("Prompt preset no longer exists")?;
                check_revision(preset, expected_revision)?;
                preset.name = name;
                preset.template = template;
                preset.revision = increment(preset.revision)?;
            }
            PromptPresetMutation::Delete {
                id,
                expected_revision,
            } => {
                let preset = next
                    .presets
                    .iter()
                    .find(|preset| preset.id == id)
                    .ok_or("Prompt preset no longer exists")?;
                check_revision(preset, expected_revision)?;
                if next.presets.len() == 1 {
                    return Err("The last prompt preset cannot be deleted".into());
                }
                next.presets.retain(|preset| preset.id != id);
                if next.selected_id == id {
                    next.selected_id = next.presets[0].id.clone();
                }
            }
            PromptPresetMutation::Select { id } => {
                next.selected_id = id;
            }
            PromptPresetMutation::ResetAll {
                expected_catalog_revision,
            } => {
                if self.revision != expected_catalog_revision {
                    return Err("Prompt presets have changed; reload before resetting".into());
                }
                let next_record_revision = increment(
                    self.presets
                        .iter()
                        .map(|p| p.revision)
                        .max()
                        .unwrap_or(0)
                        .max(self.revision),
                )?;
                next.presets = bundled_presets();
                for preset in &mut next.presets {
                    preset.revision = next_record_revision;
                }
                next.selected_id = next.presets[0].id.clone();
                // Reset revokes all prior execution and editor generations, even when the
                // selected seed already happens to have its initial template.
                next.execution_revision = increment(self.execution_revision)?;
            }
        }
        next = next.normalize()?;
        if next == *self {
            return Ok(next);
        }
        next.revision = increment(self.revision)?;
        if next.execution_revision == self.execution_revision
            && next.selected().template != self.selected().template
        {
            next.execution_revision = increment(self.execution_revision)?;
        }
        Ok(next)
    }
}

fn check_revision(preset: &PromptPreset, expected: u32) -> Result<(), String> {
    if preset.revision != expected {
        return Err("Prompt preset has changed; reload before saving".into());
    }
    Ok(())
}
fn increment(revision: u32) -> Result<u32, String> {
    revision
        .checked_add(1)
        .ok_or_else(|| "Prompt preset revision exhausted".into())
}

pub fn bundled_presets() -> Vec<PromptPreset> {
    [
        ("visual-learner", "Visual Learner", "Lead with an infographic that makes the important ideas, their relationships, and the context needed to understand them clear. Integrate relevant source information and verified supplementary context. Use visual hierarchy, spatial grouping, comparisons, and connections to explain the information. Choose the composition, level of detail, and number of visuals according to what communicates the information most effectively. Keep text readable and avoid unnecessary fragmentation or decorative bulk. Create the infographic as HTML or images using the shared output rules, then provide supplementary explanation and supporting detail in the text that follows."),
        ("conceptual-learner", "Conceptual Learner", "Lead with the central idea and a clear account of how the concepts relate. Define essential terms, explain causes and dependencies, and distinguish similar concepts. Use an HTML concept map, comparison panel, or annotated illustration when it makes the structure easier to understand; choose an image when illustration is more suitable. Follow with a plain-language explanation and a concrete example. Identify the limits of analogies and important qualifications. Do not add a visual that contributes no explanatory value."),
        ("practical-learner", "Practical Learner", "Lead with a concrete worked example that makes the information usable. Show the starting conditions, decisions, steps, and expected result. Use an HTML walkthrough, annotated example, decision diagram, or checklist when it makes the procedure easier to follow; use an image when spatial or physical details are better illustrated. Explain how the same reasoning transfers to another case and highlight common mistakes. Label invented examples and describe actions without performing them on the user's behalf."),
        ("analytical-learner", "Analytical Learner", "Explain the information through its underlying relationships and structure. Define relevant quantities, variables, assumptions, and constraints. Use equations, logical expressions, tables, or graphs when they clarify those relationships, and connect each formal representation to a plain-language explanation and a concrete example. State units, uncertainty, and the conditions under which a model applies. Distinguish source-supported relationships from illustrative models or assumptions; do not invent numerical precision or force qualitative information into formulas. Use a static HTML visualization or an image when it clarifies the model or comparison."),
    ].into_iter().map(|(id, name, instruction)| PromptPreset {
        id: id.into(), name: name.into(), revision: 1,
        template: AgentPromptTemplate::with_explanation_strategy(instruction),
        bundled_source: Some(BundledPromptPresetSource { id: id.into(), version: 2 }),
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::prompt_template::BUILT_IN_RESPONSE_INSTRUCTION;

    #[test]
    fn defaults_are_four_ordered_seeds_and_first_is_selected() {
        let catalog = PromptPresetCatalog::default().normalize().unwrap();
        assert_eq!(catalog.schema_version, 2);
        assert_eq!(catalog.presets.len(), 4);
        assert_eq!(catalog.selected_id, catalog.presets[0].id);
        assert_eq!(catalog.selected_id, "visual-learner");
        assert!(catalog
            .presets
            .iter()
            .all(|p| p.template.common.contains(BUILT_IN_RESPONSE_INSTRUCTION)));
        assert!(!serde_json::to_string(&catalog)
            .unwrap()
            .contains("description"));
    }

    #[test]
    fn reset_replaces_every_record_selects_first_and_rejects_stale_editors() {
        let original = PromptPresetCatalog::default();
        let visual = original.selected().clone();
        let mut template = visual.template.clone();
        template.common.push_str("\nMy customization");
        let edited = original
            .apply(PromptPresetMutation::Update {
                id: visual.id.clone(),
                expected_revision: 1,
                name: "Edited".into(),
                template,
            })
            .unwrap();
        let added = edited
            .apply_with_creation_id(
                PromptPresetMutation::Create {
                    name: "Added".into(),
                    template: visual.template.clone(),
                },
                Some(Uuid::from_u128(123)),
            )
            .unwrap();
        let selected = added
            .apply(PromptPresetMutation::Select {
                id: added.presets.last().unwrap().id.clone(),
            })
            .unwrap();
        let reset = selected
            .apply(PromptPresetMutation::ResetAll {
                expected_catalog_revision: selected.revision,
            })
            .unwrap();
        assert_eq!(reset.presets.len(), 4);
        assert_eq!(reset.selected_id, reset.presets[0].id);
        assert!(reset.revision > selected.revision);
        assert!(reset.execution_revision > selected.execution_revision);
        for (actual, seed) in reset.presets.iter().zip(bundled_presets()) {
            assert_eq!(actual.id, seed.id);
            assert_eq!(actual.name, seed.name);
            assert_eq!(actual.template, seed.template);
            assert!(actual.revision > selected.presets.iter().map(|p| p.revision).max().unwrap());
        }
        assert!(reset
            .apply(PromptPresetMutation::Update {
                id: visual.id,
                expected_revision: edited.selected().revision,
                name: "Stale".into(),
                template: visual.template
            })
            .is_err());
        assert!(reset
            .apply(PromptPresetMutation::ResetAll {
                expected_catalog_revision: selected.revision
            })
            .is_err());
        let again = reset
            .apply(PromptPresetMutation::ResetAll {
                expected_catalog_revision: reset.revision,
            })
            .unwrap();
        assert!(again.execution_revision > reset.execution_revision);
        assert!(again.presets[0].revision > reset.presets[0].revision);
    }

    #[test]
    fn metadata_is_execution_neutral_and_switch_aba_advances_execution() {
        let catalog = PromptPresetCatalog::default();
        let renamed = catalog
            .apply(PromptPresetMutation::Update {
                id: catalog.selected_id.clone(),
                expected_revision: 1,
                name: "Renamed".into(),
                template: catalog.selected().template.clone(),
            })
            .unwrap();
        assert_eq!(renamed.execution_revision, catalog.execution_revision);
        let b = renamed
            .apply(PromptPresetMutation::Select {
                id: "conceptual-learner".into(),
            })
            .unwrap();
        let a = b
            .apply(PromptPresetMutation::Select {
                id: catalog.selected_id.clone(),
            })
            .unwrap();
        assert_eq!(a.execution_revision, catalog.execution_revision + 2);
        assert_eq!(
            a.apply(PromptPresetMutation::Select {
                id: a.selected_id.clone()
            })
            .unwrap(),
            a
        );
    }

    #[test]
    fn creation_and_deletion_preserve_bounded_explicit_identity() {
        let catalog = PromptPresetCatalog::default();
        let change = PromptPresetMutation::Create {
            name: "New".into(),
            template: catalog.selected().template.clone(),
        };
        assert!(catalog.apply(change.clone()).is_err());
        let created = catalog
            .apply_with_creation_id(change.clone(), Some(Uuid::from_u128(123)))
            .unwrap();
        assert!(created
            .apply_with_creation_id(change, Some(Uuid::from_u128(123)))
            .is_err());
        let deleted = catalog
            .apply(PromptPresetMutation::Delete {
                id: catalog.selected_id.clone(),
                expected_revision: 1,
            })
            .unwrap();
        assert_eq!(deleted.clone().normalize().unwrap().presets.len(), 3);
        assert_eq!(deleted.selected_id, catalog.presets[1].id);
        let inactive_deleted = catalog
            .apply(PromptPresetMutation::Delete {
                id: catalog.presets[1].id.clone(),
                expected_revision: 1,
            })
            .unwrap();
        assert_eq!(inactive_deleted.selected_id, catalog.selected_id);
        assert_eq!(
            inactive_deleted.execution_revision,
            catalog.execution_revision
        );
        let last_selected = catalog
            .apply(PromptPresetMutation::Select {
                id: catalog.presets[2].id.clone(),
            })
            .unwrap();
        let deleted_last = last_selected
            .apply(PromptPresetMutation::Delete {
                id: last_selected.selected_id.clone(),
                expected_revision: 1,
            })
            .unwrap();
        assert_eq!(deleted_last.selected_id, catalog.presets[0].id);

        let reset = deleted
            .apply(PromptPresetMutation::ResetAll {
                expected_catalog_revision: deleted.revision,
            })
            .unwrap();
        assert!(reset.presets[0].revision > catalog.presets[0].revision);
        let one = PromptPresetCatalog {
            presets: vec![catalog.selected().clone()],
            ..catalog
        };
        assert!(one
            .apply(PromptPresetMutation::Delete {
                id: one.selected_id.clone(),
                expected_revision: 1,
            })
            .is_err());
    }

    #[test]
    fn invalid_catalog_and_revision_exhaustion_fail_without_mutation() {
        let original = PromptPresetCatalog::default();
        let mut invalid = original.clone();
        invalid.presets.push(invalid.presets[0].clone());
        assert!(invalid.validate().is_err());
        invalid = original.clone();
        invalid.schema_version = 1;
        assert!(invalid.validate().is_err());
        invalid = original.clone();
        invalid.execution_revision = u32::MAX;
        assert!(invalid
            .apply(PromptPresetMutation::ResetAll {
                expected_catalog_revision: invalid.revision
            })
            .is_err());
        assert_eq!(original.revision, 1);
    }
}
