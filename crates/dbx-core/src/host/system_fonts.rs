#[cfg(feature = "system-fonts")]
pub fn list_system_fonts() -> Vec<String> {
    font_kit::source::SystemSource::new()
        .all_families()
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
#[cfg(not(feature = "system-fonts"))]
pub fn list_system_fonts() -> Vec<String> {
    vec![]
}
