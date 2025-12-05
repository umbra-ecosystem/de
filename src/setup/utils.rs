use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
};

pub struct EnvMapper<'a> {
    pub _map: Cow<'a, BTreeMap<String, String>>,
    pub values: BTreeMap<String, String>,
}

impl Default for EnvMapper<'_> {
    fn default() -> Self {
        Self {
            _map: Cow::Owned(BTreeMap::new()),
            values: BTreeMap::new(),
        }
    }
}

impl<'a> EnvMapper<'a> {
    pub fn new(map: &'a BTreeMap<String, String>) -> Self {
        let env = std::env::vars().collect::<HashMap<_, _>>();
        let values = map
            .iter()
            .filter_map(|(mapped, original)| {
                env.get(original)
                    .map(|value| (mapped.clone(), value.clone()))
            })
            .collect();

        tracing::debug!("Env mapper created with values: {values:?}");

        Self {
            _map: Cow::Borrowed(map),
            values,
        }
    }

    pub fn with_env<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.values.insert(key.into(), value.into());
        self
    }

    pub fn format_str(&self, value: &str) -> String {
        let mut formatted_command = value.to_string();
        for (name, value) in self.values.iter() {
            formatted_command = formatted_command.replace(&format!("${{{}}}", name), value);
        }
        tracing::info!("formatted string with env: {value} -> {formatted_command}");
        formatted_command
    }
}
