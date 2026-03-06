//! Property metadata for simulation contexts and other types

use super::types::PropertyInfo;

/// Simulation context properties (available in @simulation functions via `ctx` parameter)
pub fn simulation_context_properties() -> Vec<PropertyInfo> {
    vec![
        PropertyInfo {
            name: "index".to_string(),
            property_type: "Number".to_string(),
            description: "Current element index in the simulation".to_string(),
        },
        PropertyInfo {
            name: "state".to_string(),
            property_type: "Any".to_string(),
            description: "Current simulation state".to_string(),
        },
        PropertyInfo {
            name: "metadata".to_string(),
            property_type: "Object".to_string(),
            description: "Additional simulation metadata".to_string(),
        },
    ]
}
