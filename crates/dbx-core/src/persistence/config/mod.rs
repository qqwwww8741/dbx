pub mod expression;
// governance disabled — storage methods not available at HEAD after main merge
// pub mod governance;
pub mod layer;
pub mod tag;
pub mod trace;

pub use expression::{parse_expression, resolve_all_expressions_in_value, resolve_expression, Expression};
// pub use governance::{ ... };
pub use layer::{ConfigLayer, ConfigTree, LayerConfig, MergedConfig};
pub use tag::{
    BlockStats, BusinessTag, TagGuard, TagInheritanceWhitelist, TagPolicy, TagValidationResult, TagValidator,
};
pub use trace::{TraceEntry, TraceRingBuffer, TraceStats};

#[cfg(test)]
mod integration_tests {
    use crate::config::*;
    use crate::models::connection::DatabaseType;
    use crate::schema_diff::SchemaDiffPreparationOptions;
    use std::collections::HashMap;

    #[test]
    fn test_config_expression_with_value_injection() {
        let mut tree = ConfigTree::new();

        let mut global = HashMap::new();
        global.insert("base_host".to_string(), serde_json::Value::String("db.example.com".to_string()));
        global.insert("base_port".to_string(), serde_json::Value::Number(serde_json::Number::from(5432)));

        tree.add_layer(LayerConfig {
            layer: ConfigLayer::Global,
            name: "global".to_string(),
            values: global,
            ..Default::default()
        });

        tree.add_layer(LayerConfig {
            layer: ConfigLayer::Project,
            name: "project".to_string(),
            values: HashMap::from([
                ("jdbc_url".to_string(), serde_json::Value::String("${eval:\"jdbc:postgresql://\"}".to_string())),
                ("host".to_string(), serde_json::Value::String("${ref:base_host}".to_string())),
            ]),
            ..Default::default()
        });

        let merged = tree.merge().unwrap();
        let scope = merged.values.clone();

        let url_resolved = resolve_all_expressions_in_value(
            &serde_json::Value::String("${eval:\"jdbc:postgresql://\"}".to_string()),
            &scope,
        )
        .unwrap();
        assert_eq!(url_resolved, serde_json::Value::String("jdbc:postgresql://".to_string()));

        let host_resolved =
            resolve_all_expressions_in_value(&serde_json::Value::String("${ref:base_host}".to_string()), &scope)
                .unwrap();
        assert_eq!(host_resolved, serde_json::Value::String("db.example.com".to_string()));
    }
}
