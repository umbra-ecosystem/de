use std::collections::{BTreeMap, HashMap};

pub struct EnvMapper<'a> {
    pub map: &'a BTreeMap<String, String>,
    pub values: BTreeMap<String, String>,
}

impl<'a> EnvMapper<'a> {
    pub fn new(map: &'a BTreeMap<String, String>) -> Self {
        let env = std::env::vars().collect::<HashMap<_, _>>();
        let values = map
            .iter()
            .filter_map(|(mapped, original)| {
                env.get(original).map(|value| (mapped.clone(), value.clone()))
            })
            .collect();

        Self { map, values }
    }

    pub fn format_str(&self, value: &str) -> String {
        let mut formatted_command = value.to_string();
        for (mapped, original) in self.map.iter() {
            if let Some(value) = self.values.get(mapped) {
                formatted_command = formatted_command.replace(&format!("${{{}}}", original), value);
            }
        }
        tracing::info!("formatted string with env: {value} -> {formatted_command}");
        formatted_command
    }
}
