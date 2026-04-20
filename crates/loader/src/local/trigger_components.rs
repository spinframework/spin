use std::collections::{HashMap, HashSet};

use spin_locked_app::{
    locked::{LockedApp, LockedComponent, LockedTrigger},
    values::ValuesMap,
};

/// We want all component/composition graph information to be in the component,
/// because the component ID is how Spin looks this stuff up. So if a trigger
/// contains a `components` table, e.g. specifying middleware, we want to move
/// that to the component.
///
/// But it's possible to have two triggers pointing to the same primary component,
/// but with different middleware. In this case, we will synthesise a component
/// for each such trigger, with the same main configuration but with its own
/// "extra" components.
pub fn reassign_extras(mut locked: LockedApp) -> LockedApp {
    let mut id_dispenser = SyntheticIdDispenser::new();

    let needs_splitting = needs_splitting(&locked);

    for (component_to_split, triggers) in needs_splitting {
        for trigger in &triggers {
            if !has_extra_components(trigger) {
                // It's possible to have e.g. 3 triggers pointing to the same component,
                // with only one enriched with middleware. The two unenriched ones can
                // continue pointing to the original component.
                continue;
            }

            // We need to split off a munge for this component-trigger combination.
            // Locate the component, clone it under a new ID, and add the new-named clone
            // to the app. Then point the trigger at the new name.
            let mut split_out_component = locked
                .components
                .iter()
                .find(|c| c.id == *component_to_split)
                .unwrap()
                .clone();

            let synthetic_id = id_dispenser.create_id(&trigger.id, &component_to_split);
            split_out_component.id = synthetic_id.clone();
            locked.components.push(split_out_component);
            set_component_id(&mut locked, &trigger.id, &synthetic_id);
        }
    }

    // Now we have cloned components so that each set of { primary + trigger extras }
    // can have its own component, meaning that composition graphs remain uniquely
    // identified by component ID. So we can move all extra trigger components to
    // the now-unique components, where they can later undergo trigger-specific
    // composition.
    move_extras_from_triggers_to_components(&mut locked);

    locked
}

fn needs_splitting(locked: &LockedApp) -> HashMap<String, Vec<LockedTrigger>> {
    let referenced_component_ids: Vec<_> =
        locked.triggers.iter().filter_map(component_id).collect();
    let cid_to_triggers: HashMap<_, _> = referenced_component_ids
        .iter()
        .map(|cid| (cid.clone(), triggers_referencing(&locked.triggers, cid)))
        .collect();
    let needs_splitting = cid_to_triggers
        .into_iter()
        .filter(|(_, triggers)| triggers.len() > 1 && triggers.iter().any(has_extra_components))
        .collect::<HashMap<_, _>>();
    needs_splitting
}

fn move_extras_from_triggers_to_components(locked: &mut LockedApp) {
    for trigger in &mut locked.triggers {
        if let Some(extras) = extra_components(trigger) {
            if let Some(component_id) = component_id(trigger) {
                if let Some(component) = get_component_mut(&mut locked.components, &component_id) {
                    component
                        .metadata
                        .insert("trigger-extras".into(), extras.clone().into());
                    component.metadata.insert(
                        "resolve-extras-using".into(),
                        trigger.trigger_type.clone().into(),
                    );
                    trigger
                        .trigger_config
                        .as_object_mut()
                        .unwrap()
                        .remove("components");
                }
            }
        }
    }
}

fn get_component_mut<'a>(
    components: &'a mut [LockedComponent],
    component_id: &str,
) -> Option<&'a mut LockedComponent> {
    components.iter_mut().find(|c| c.id == component_id)
}

fn component_id(trigger: &LockedTrigger) -> Option<String> {
    trigger
        .trigger_config
        .get("component")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn set_component_id(app: &mut LockedApp, trigger_id: &str, component_id: &str) {
    let trigger = app
        .triggers
        .iter_mut()
        .find(|t| t.id == trigger_id)
        .unwrap();
    trigger
        .trigger_config
        .as_object_mut()
        .unwrap()
        .insert("component".into(), component_id.into());
}

fn extra_components(trigger: &LockedTrigger) -> Option<&ValuesMap> {
    trigger
        .trigger_config
        .get("components")
        .and_then(|v| v.as_object())
}

fn has_extra_components(trigger: &LockedTrigger) -> bool {
    extra_components(trigger).is_some_and(|xcs| !xcs.is_empty())
}

fn triggers_referencing(all_triggers: &[LockedTrigger], cid: &String) -> Vec<LockedTrigger> {
    all_triggers
        .iter()
        .filter(|t| component_id(t).as_ref() == Some(cid))
        .cloned()
        .collect()
}

/// Helper for generating synthetic IDs for split-out components.
/// Just keeps a bit of faffy gunk out of the main flow.
struct SyntheticIdDispenser {
    seen: HashSet<String>,
    disambiguator: u32,
}

impl SyntheticIdDispenser {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
            disambiguator: 0,
        }
    }
    fn create_id(&mut self, trigger_id: &str, component_id: &str) -> String {
        let mut synthetic_id = format!("{component_id}-for-{}", trigger_id);
        if self.seen.contains(&synthetic_id) {
            self.disambiguator += 1;
            synthetic_id = format!("{synthetic_id}-d{}", self.disambiguator);
        }
        self.seen.insert(synthetic_id.clone());
        synthetic_id
    }
}
