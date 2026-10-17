//! Saved explanation preferences, independently versioned from execution templates.
use domain::prompt_template::AgentPromptTemplate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const PROMPT_PRESET_CATALOG_SCHEMA_VERSION: u32 = 2;
pub const MAX_PROMPT_PRESETS: usize = 64;
pub(crate) const LEGACY_PRESET_IDS: [(&str, &str); 3] = [
    ("conceptual-learner", "conceptual"),
    ("practical-learner", "practical"),
    ("analytical-learner", "analytical"),
];

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
    /// Upgrade legacy identities only at persisted-input admission, preserving all content.
    pub fn migrate_legacy_ids(mut self) -> Self {
        for (old, new) in LEGACY_PRESET_IDS {
            let Some(index) = self.presets.iter().position(|p| p.id == old) else {
                continue;
            };
            let mut id = new.to_owned();
            let collision = self.presets.iter().any(|p| p.id == id);
            if collision {
                // Preserve the existing canonical record and retain the legacy record as a copy.
                let mut suffix = 1;
                loop {
                    id = format!("{new}-migrated-{suffix}");
                    if !self.presets.iter().any(|p| p.id == id) {
                        break;
                    }
                    suffix += 1;
                }
            }
            if self.selected_id == old {
                self.selected_id = id.clone();
            }
            let preset = &mut self.presets[index];
            preset.id = id;
            if collision {
                preset.bundled_source = None;
            } else if let Some(source) = &mut preset.bundled_source {
                if source.id == old {
                    source.id = new.into();
                }
            }
        }
        self
    }

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
                    // Retired bundled IDs remain valid in saved, user-editable catalogs.
                    || ![
                        "visual-learner",
                        "conceptual",
                        "practical",
                        "analytical",
                        "evocative",
                        "illustrated",
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
            && (next.selected_id != self.selected_id
                || next.selected().template != self.selected().template)
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
    let mut presets: Vec<_> = [
        ("conceptual", "Conceptual", 10, "Help the user build a coherent mental model of the source. Lead with the central idea, define essential terms, and explain how the concepts relate, why those relationships hold, and where their limits lie. Distinguish similar concepts and make important causes, dependencies and qualifications clear in plain language. Use a brief example, counterexample or analogy when it clarifies a concept or distinction; connect it back to the general idea and state the limits of any analogy. Use a concept map, comparison or annotated illustration when it clarifies the structure, choosing its format using the shared output rules. When using an HTML App, choose useful interactions that let the user inspect a concept, compare related concepts or trace a relationship. Carry that context into questions and offer follow-ups that help explain why a relationship holds, resolve a distinction or explore a qualification."),
        ("practical", "Practical", 10, "Help the user apply the source to a concrete situation. Lead with a worked example that carries the explanation from starting conditions and a clear goal through the relevant decisions and actions to an expected result. Explain the reasoning at each decision and how to recognize a successful outcome. Introduce concepts where they explain the example, then show what carries over to another case, what changes with the conditions, and common mistakes. Label invented examples and assumptions, and describe actions without performing them on the user's behalf. Use a walkthrough, annotated example, decision diagram or checklist when it makes the application easier to follow, choosing its format using the shared output rules. When using an HTML App, choose useful interactions that let the user explore the worked example, inspect a decision or step, or compare a plausible alternative. Carry the relevant step and selected conditions into questions, and connect answers to outcomes, transfer to another case and common mistakes."),
        ("analytical", "Analytical", 9, "Explain the information through its underlying relationships and structure. Define relevant quantities, variables, assumptions, and constraints. Use equations, logical expressions, tables, or graphs when they clarify those relationships, and connect each formal representation to a plain-language explanation and a concrete example. State units, uncertainty, and the conditions under which a model applies. Distinguish source-supported relationships from illustrative models or assumptions; do not invent numerical precision or force qualitative information into formulas. Use a visualization when it clarifies the model or comparison. Choose its format using the shared output rules. When using an HTML App, let interactions help the user examine an assumption, compare cases, or explore the effect of a meaningful parameter when useful. Keep variables, units, uncertainty and model limits visible where relevant. Carry the relevant model and selected case into questions; do not imply that an illustrative calculation is source evidence."),
    ].into_iter().map(|(id, name, version, instruction)| PromptPreset {
        id: id.into(), name: name.into(), revision: 1,
        template: AgentPromptTemplate::with_explanation_strategy(instruction),
        bundled_source: Some(BundledPromptPresetSource { id: id.into(), version }),
    }).collect();
    presets.push(PromptPreset {
        id: "evocative".into(),
        name: "Evocative".into(),
        revision: 1,
        template: AgentPromptTemplate::evocative(),
        bundled_source: Some(BundledPromptPresetSource {
            id: "evocative".into(),
            version: 8,
        }),
    });
    presets.push(PromptPreset {
        id: "illustrated".into(),
        name: "Illustrated".into(),
        revision: 1,
        template: AgentPromptTemplate::with_explanation_strategy(
            "Make a visual composition the core of the explanation. Preserve the source's substantive claims, reasoning, conditions, exceptions, examples and uncertainty; prioritize clarity and depth over brevity unless the user asks for a summary. State any substantive content the composition does not represent. Before choosing output tools, plan the entities, relationships, sequence and labels, then make placement, connections, grouping and annotations communicate that structure. Choose scenes, cutaways, processes, comparisons, maps or infographics according to the content. Use multiple connected sections, panels or views when needed, with a clear reading order. Keep labels concise and place enough explanation beside the relevant elements to make their meaning and reasoning understandable. Choose the medium using the shared output rules: HTML, CSS and SVG suit precise text, values, connectors and layout; generated imagery suits appearance, materials and spatial context, or an integrated diagram when it communicates the content well. When an image is the primary explanation, create a complete annotated visual with the necessary labels, arrows and legends, inspect it when possible and correct material errors; put precision-critical text in HTML or SVG when needed. Publish only actual image bytes or usable URLs returned by tools, and integrate each image at its corresponding point in the explanation. If an image cannot be embedded, rethink the composition; do not duplicate an image gallery with a complete HTML explanation. Distinguish illustrative imagery from source evidence. In an HTML App, use meaningful interactions to reveal annotations, compare views, trace relationships or explore a meaningful variable, while keeping the central explanation readable initially. Let users ask contextual questions about selected figures, labels or relationships through the shared Lens question API.",
        ),
        bundled_source: Some(BundledPromptPresetSource {
            id: "illustrated".into(),
            version: 1,
        }),
    });
    presets
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::prompt_template::BUILT_IN_RESPONSE_INSTRUCTION;

    #[test]
    fn defaults_are_five_ordered_seeds_and_first_is_selected() {
        let catalog = PromptPresetCatalog::default().normalize().unwrap();
        assert_eq!(catalog.schema_version, 2);
        assert_eq!(
            catalog
                .presets
                .iter()
                .map(|preset| preset.name.as_str())
                .collect::<Vec<_>>(),
            [
                "Conceptual",
                "Practical",
                "Analytical",
                "Evocative",
                "Illustrated"
            ]
        );
        assert_eq!(
            catalog
                .presets
                .iter()
                .map(|preset| preset.bundled_source.as_ref().unwrap().version)
                .collect::<Vec<_>>(),
            [10, 10, 9, 8, 1]
        );
        assert_eq!(catalog.selected_id, catalog.presets[0].id);
        assert_eq!(catalog.selected_id, "conceptual");
        assert!(catalog
            .presets
            .iter()
            .filter(|p| p.id != "evocative")
            .all(|p| p.template.common.contains(BUILT_IN_RESPONSE_INSTRUCTION)));
        for preset in catalog
            .presets
            .iter()
            .filter(|preset| preset.id != "evocative")
        {
            let common = &preset.template.common;
            assert!(common.contains("If generated images best communicate the content"));
            assert!(common.contains("prefer an appropriate authorized MCP Apps tool"));
            assert!(common.contains("lens_rich_content.render_html"));
            assert!(!common.contains("lens_output.publish_html"));
            assert!(common.contains("supplementary Markdown and the bundled HTML App body text"));
            assert!(common.contains("For external MCP Apps"));
        }
        assert!(catalog.presets[3]
            .template
            .common
            .contains("Generated images are the primary and required output"));
        let serialized = serde_json::to_value(&catalog).unwrap();
        assert!(!serialized.as_object().unwrap().contains_key("description"));
        for preset in serialized["presets"].as_array().unwrap() {
            assert!(!preset.as_object().unwrap().contains_key("description"));
        }
    }

    #[test]
    fn reset_replaces_every_record_selects_first_and_rejects_stale_editors() {
        let original = PromptPresetCatalog::default();
        let initial_preset = original.selected().clone();
        let mut template = initial_preset.template.clone();
        template.common.push_str("\nMy customization");
        let edited = original
            .apply(PromptPresetMutation::Update {
                id: initial_preset.id.clone(),
                expected_revision: 1,
                name: "Edited".into(),
                template,
            })
            .unwrap();
        let added = edited
            .apply_with_creation_id(
                PromptPresetMutation::Create {
                    name: "Added".into(),
                    template: initial_preset.template.clone(),
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
        assert_eq!(reset.presets.len(), 5);
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
                id: initial_preset.id,
                expected_revision: edited.selected().revision,
                name: "Stale".into(),
                template: initial_preset.template
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
                id: "practical".into(),
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
    fn identical_templates_still_advance_execution_when_selection_changes() {
        let original = PromptPresetCatalog::default();
        let duplicate_id = Uuid::from_u128(123).to_string();
        let catalog = original
            .apply_with_creation_id(
                PromptPresetMutation::Create {
                    name: "Duplicate".into(),
                    template: original.selected().template.clone(),
                },
                Some(Uuid::from_u128(123)),
            )
            .unwrap();
        assert_eq!(catalog.execution_revision, original.execution_revision);
        let selected = catalog
            .apply(PromptPresetMutation::Select {
                id: duplicate_id.clone(),
            })
            .unwrap();
        assert_eq!(selected.selected().template, catalog.selected().template);
        assert_eq!(selected.execution_revision, catalog.execution_revision + 1);
        assert_eq!(
            selected
                .apply(PromptPresetMutation::Select {
                    id: duplicate_id.clone()
                })
                .unwrap(),
            selected
        );
        let back = selected
            .apply(PromptPresetMutation::Select {
                id: original.selected_id.clone(),
            })
            .unwrap();
        assert_eq!(back.execution_revision, catalog.execution_revision + 2);
        let deleted = selected
            .apply(PromptPresetMutation::Delete {
                id: duplicate_id.clone(),
                expected_revision: 1,
            })
            .unwrap();
        assert_eq!(deleted.selected_id, original.selected_id);
        assert_eq!(deleted.selected().template, selected.selected().template);
        assert_eq!(deleted.execution_revision, selected.execution_revision + 1);
        let reset = selected
            .apply(PromptPresetMutation::ResetAll {
                expected_catalog_revision: selected.revision,
            })
            .unwrap();
        assert_eq!(reset.execution_revision, selected.execution_revision + 1);
        let mut inactive_template = catalog.selected().template.clone();
        inactive_template
            .common
            .push_str("\nInactive customization.");
        let edited = catalog
            .apply(PromptPresetMutation::Update {
                id: duplicate_id,
                expected_revision: 1,
                name: "Edited duplicate".into(),
                template: inactive_template,
            })
            .unwrap();
        assert_eq!(edited.execution_revision, catalog.execution_revision);
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
        assert_eq!(
            deleted.clone().normalize().unwrap().presets.len(),
            catalog.presets.len() - 1
        );
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
