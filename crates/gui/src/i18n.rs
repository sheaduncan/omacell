//! Minimal Fluent-resource lookup for the 1.0 localization scaffold.

const EN_US: &str = include_str!("../../../i18n/en-US/omacell.ftl");

/// Look up one static English message from the bundled Fluent resource.
///
/// WP-28 intentionally ships only `en-US`. Keeping every call behind this
/// function makes adding locale negotiation and a full Fluent formatter an
/// additive change rather than another UI-string migration.
pub(crate) fn tr(id: &'static str) -> &'static str {
    EN_US
        .lines()
        .filter_map(|line| line.split_once('='))
        .find_map(|(key, value)| (key.trim() == id).then(|| value.trim()))
        .unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::tr;

    #[test]
    fn english_fluent_resource_is_embedded() {
        assert_eq!(tr("palette-title"), "Command palette");
        let missing = "missing-message";
        assert_eq!(tr(missing), missing);
    }
}
