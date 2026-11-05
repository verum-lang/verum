//! Source declaration loads publish success only after recursive loading closes.
use crate::Type;
use verum_ast::TypeDecl;
use verum_common::{List, Map, Text};

#[derive(Debug, Default)]
pub(crate) struct SourceTypeRegistration {
    pub headers: Vec<super::source_protocol_header::SourceProtocolHeader>,
    pub projection_assumptions: Vec<crate::protocol::WhereClause>,
    pub completed: Map<Text, TypeDecl>,
    pub pending: Map<Text, TypeDecl>,
    pub depth: usize,
    pub failure: Option<Text>,
    pub aliases: Map<Text, (Option<Type>, Option<verum_common::List<Text>>)>,
    pub protocols: Map<Text, SourceProtocolRegistration>,
    pub variants: SourceVariantRegistration,
}

#[derive(Debug)]
pub(crate) struct SourceProtocolRegistration {
    pub publications: Vec<crate::protocol::ProtocolBindingReceipt>,
    pub unifier_name: bool,
    pub defaults: Option<(verum_common::Set<Text>, verum_common::Set<Text>)>,
}

#[derive(Debug, Default)]
pub(crate) struct SourceVariantRegistration {
    pub names: Map<Text, Option<Text>>,
    pub parents: Map<Text, Option<List<Text>>>,
    pub records: Map<Text, Option<indexmap::IndexMap<Text, Type>>>,
    pub paths: Map<Text, Option<List<crate::ty::PathConstructor>>>,
    pub arities: Map<Text, Option<usize>>,
    pub shared_names: Vec<(Text, Text)>,
    pub collisions: List<(Text, Text, Text)>,
}

fn restore_entries<T>(target: &mut Map<Text, T>, journal: Map<Text, Option<T>>) {
    for (name, previous) in journal {
        match previous {
            Some(value) => {
                target.insert(name, value);
            }
            None => {
                target.remove(&name);
            }
        }
    }
}

impl super::TypeChecker {
    pub(super) fn journal_variant_parent(&mut self, name: &Text) {
        if self.source_type_registration.depth != 0 {
            self.source_type_registration
                .variants
                .parents
                .entry(name.clone())
                .or_insert_with(|| self.variant_constructor_parents.get(name).cloned());
        }
    }

    pub(super) fn journal_variant_record(&mut self, name: &Text) {
        if self.source_type_registration.depth != 0 {
            self.source_type_registration
                .variants
                .records
                .entry(name.clone())
                .or_insert_with(|| self.variant_record_fields.get(name).cloned());
        }
    }

    pub(super) fn journal_variant_paths(&mut self, name: &Text) {
        if self.source_type_registration.depth != 0 {
            self.source_type_registration
                .variants
                .paths
                .entry(name.clone())
                .or_insert_with(|| self.hit_path_constructors.get(name).cloned());
        }
    }

    pub(super) fn finish_source_variant_registrations(&mut self, commit: bool) {
        self.unifier.finish_variant_registration_journal(commit);
        let journal = std::mem::take(&mut self.source_type_registration.variants);
        if commit {
            // The shared registry only receives complete declaration groups.
            // No failed group's provisional name can escape to another checker.
            let mut registry = self.protocol_checker.write();
            for (signature, name) in journal.shared_names {
                registry.register_variant_type_name(signature, name);
            }
            self.variant_collision_log.extend(journal.collisions);
        } else {
            restore_entries(&mut self.variant_type_names, journal.names);
            restore_entries(&mut self.variant_constructor_parents, journal.parents);
            restore_entries(&mut self.variant_record_fields, journal.records);
            restore_entries(&mut self.hit_path_constructors, journal.paths);
            restore_entries(&mut self.type_generics_count, journal.arities);
        }
    }
}
