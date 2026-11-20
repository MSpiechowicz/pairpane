use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Static facts about the running application, shown in the About view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    /// Product name.
    pub name: String,
    /// Application version (semver).
    pub version: String,
}

impl AppInfo {
    /// Builds the info from the product name and version.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    #[test]
    fn new_sets_name_and_version() {
        let info = AppInfo::new("Pairpane", "1.2.3");

        assert_eq!(info.name, "Pairpane");
        assert_eq!(info.version, "1.2.3");
    }

    #[test]
    fn serializes_to_exactly_name_and_version() {
        let value = serde_json::to_value(AppInfo::new("Pairpane", "1.2.3")).expect("serializable");

        assert_eq!(value, json!({ "name": "Pairpane", "version": "1.2.3" }));
    }
}
