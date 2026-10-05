//! Canonical module qualification shared by archive producers and consumers.
use crate::{List, Text};

/// Join an owner and a descriptor name, preserving the longest shared suffix.
/// A descriptor may already be promoted fully or relative to its module.
pub fn qualify_module_name(module_name: &str, name: &str) -> Text {
    if name.contains('.') {
        let owner: List<&str> = module_name.split('.').collect();
        let parts: List<&str> = name.split('.').collect();
        for overlap in (1..=owner.len().min(parts.len())).rev() {
            if owner.as_slice()[owner.len() - overlap..] == parts.as_slice()[..overlap] {
                let prefix = owner.len() - overlap;
                return if prefix == 0 {
                    name.into()
                } else {
                    format!("{}.{}", owner.as_slice()[..prefix].join("."), name).into()
                };
            }
        }
    }
    format!("{module_name}.{name}").into()
}
